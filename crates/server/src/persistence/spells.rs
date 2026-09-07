use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use once_cell::sync::Lazy;
use serde::Deserialize;
use thiserror::Error;

use crate::config::CONFIG;
use crate::entities::combat::CombatElement;
use crate::entities::effects::{AreaShape, AreaShapeId, EffectId, MissileId};
use crate::entities::spells::{AreaOrigin, Spell, SpellEffect, SpellGroup, SpellId, SpellTarget};
use crate::entities::vocation::Vocation;
use crate::game::TickDelta;
use crate::persistence::areas::AREA_SHAPES;

pub static SPELLS: Lazy<Arc<HashMap<SpellId, Arc<Spell>>>> = Lazy::new(|| {
    Arc::new(load_spells(&CONFIG.spells_file_path, &AREA_SHAPES).expect("failed to load spells"))
});

#[derive(Error, Debug)]
pub enum SpellsLoadError {
    #[error("I/O error: {0}")]
    ReadError(#[from] std::io::Error),
    #[error("YAML parse error: {0}")]
    ParseError(#[from] serde_yaml::Error),
    #[error("spell {id:?} ({name}) names the area shape `{shape}`, which `areas.yaml` has not")]
    UnknownShape {
        id: SpellId,
        name: String,
        shape: AreaShapeId,
    },
    #[error("spell {id:?} ({name}) has a {field} of {value}, outside 0.00..=655.35")]
    Factor {
        id: SpellId,
        name: String,
        field: &'static str,
        value: f64,
    },
    #[error("spell {id:?} ({name}) declares no effects, so casting it would do nothing")]
    NoEffects { id: SpellId, name: String },
    #[error("spell {id:?} ({name}) has an effect that is not a single `kind:` mapping")]
    MalformedEffect { id: SpellId, name: String },
    #[error("spell {id:?} ({name}) has an effect of kind `{kind}`, which this server cannot run")]
    UnknownEffect {
        id: SpellId,
        name: String,
        kind: String,
    },
    #[error("spell {id:?} ({name}) targets `{target}`, not `self`, `target` or an `area`")]
    UnknownTarget {
        id: SpellId,
        name: String,
        target: String,
    },
    #[error("spell id {id:?} is used by both `{first}` and `{second}`")]
    DuplicateId {
        id: SpellId,
        first: String,
        second: String,
    },
}

// ── Raw YAML deserialization types ────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpellsFile {
    spells: Vec<RawSpell>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSpell {
    id: SpellId,
    name: String,
    group: SpellGroup,
    #[serde(default)]
    group_cooldown: Option<TickDelta>,
    cooldown_ticks: TickDelta,
    mana: u32,
    level: u32,
    vocations: Vec<Vocation>,
    effects: Vec<serde_yaml::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAttack {
    target: serde_yaml::Value,
    element: CombatElement,
    base_power: u16,
    level_factor: f64,
    magic_factor: f64,
    effect_id: EffectId,
    #[serde(default)]
    missile_id: Option<MissileId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawArea {
    origin: RawOrigin,
    #[serde(default)]
    rotate: bool,
    shape: AreaShapeId,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum RawOrigin {
    #[serde(rename = "self")]
    Caster,
    Target,
}

// ── Conversion ────────────────────────────────────────────────────────────────

const FACTOR_SCALE: f64 = 100.0;

fn parse_factor(
    id: SpellId,
    name: &str,
    field: &'static str,
    value: f64,
) -> Result<u16, SpellsLoadError> {
    let scaled = (value * FACTOR_SCALE).round();
    if !scaled.is_finite() || scaled < 0.0 || scaled > f64::from(u16::MAX) {
        return Err(SpellsLoadError::Factor {
            id,
            name: name.to_string(),
            field,
            value,
        });
    }
    Ok(scaled as u16)
}

fn single_entry(value: serde_yaml::Value) -> Option<(String, serde_yaml::Value)> {
    match value {
        serde_yaml::Value::Mapping(mapping) if mapping.len() == 1 => {
            let (key, value) = mapping.into_iter().next()?;
            Some((key.as_str()?.to_string(), value))
        }
        _ => None,
    }
}

fn parse_origin(origin: RawOrigin) -> AreaOrigin {
    match origin {
        RawOrigin::Caster => AreaOrigin::Caster,
        RawOrigin::Target => AreaOrigin::Target,
    }
}

fn parse_target(
    id: SpellId,
    name: &str,
    value: serde_yaml::Value,
    shapes: &HashMap<AreaShapeId, Arc<AreaShape>>,
) -> Result<SpellTarget, SpellsLoadError> {
    let unknown = |target: String| SpellsLoadError::UnknownTarget {
        id,
        name: name.to_string(),
        target,
    };

    if let Some(named) = value.as_str() {
        return match named {
            "self" => Ok(SpellTarget::Caster),
            "target" => Ok(SpellTarget::Target),
            other => Err(unknown(other.to_string())),
        };
    }

    let (kind, payload) = single_entry(value).ok_or_else(|| unknown("a mapping".to_string()))?;
    if kind != "area" {
        return Err(unknown(kind));
    }

    let area: RawArea = serde_yaml::from_value(payload)?;
    let shape = shapes
        .get(&area.shape)
        .cloned()
        .ok_or_else(|| SpellsLoadError::UnknownShape {
            id,
            name: name.to_string(),
            shape: area.shape.clone(),
        })?;

    Ok(SpellTarget::Area {
        origin: parse_origin(area.origin),
        rotate: area.rotate,
        shape,
    })
}

fn parse_effect(
    id: SpellId,
    name: &str,
    value: serde_yaml::Value,
    shapes: &HashMap<AreaShapeId, Arc<AreaShape>>,
) -> Result<SpellEffect, SpellsLoadError> {
    let (kind, payload) = single_entry(value).ok_or_else(|| SpellsLoadError::MalformedEffect {
        id,
        name: name.to_string(),
    })?;

    match kind.as_str() {
        "attack" => {
            let attack: RawAttack = serde_yaml::from_value(payload)?;
            Ok(SpellEffect::Attack {
                target: parse_target(id, name, attack.target, shapes)?,
                element: attack.element,
                base_power: attack.base_power,
                level_factor: parse_factor(id, name, "level_factor", attack.level_factor)?,
                magic_factor: parse_factor(id, name, "magic_factor", attack.magic_factor)?,
                effect_id: attack.effect_id,
                missile_id: attack.missile_id,
            })
        }
        other => Err(SpellsLoadError::UnknownEffect {
            id,
            name: name.to_string(),
            kind: other.to_string(),
        }),
    }
}

impl RawSpell {
    fn into_spell(
        self,
        shapes: &HashMap<AreaShapeId, Arc<AreaShape>>,
    ) -> Result<Spell, SpellsLoadError> {
        if self.effects.is_empty() {
            return Err(SpellsLoadError::NoEffects {
                id: self.id,
                name: self.name,
            });
        }

        let effects = self
            .effects
            .into_iter()
            .map(|effect| parse_effect(self.id, &self.name, effect, shapes))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Spell {
            id: self.id,
            name: self.name,
            group: self.group,
            group_cooldown: self.group_cooldown,
            cooldown: self.cooldown_ticks,
            mana: self.mana,
            level: self.level,
            vocations: self.vocations,
            effects,
        })
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// `shapes` is a parameter rather than a read of `AREA_SHAPES` so that the dependency
/// between the two catalogues is stated at the single site that forces both, and so a test
/// can hand this a shape of its own.
pub fn load_spells(
    path: impl AsRef<Path>,
    shapes: &HashMap<AreaShapeId, Arc<AreaShape>>,
) -> Result<HashMap<SpellId, Arc<Spell>>, SpellsLoadError> {
    load_spells_from_str(&fs::read_to_string(path)?, shapes)
}

/// Split out from `load_spells` for the same reason `load_items_from_str` is: the whole
/// read path over a document the caller owns.
fn load_spells_from_str(
    contents: &str,
    shapes: &HashMap<AreaShapeId, Arc<AreaShape>>,
) -> Result<HashMap<SpellId, Arc<Spell>>, SpellsLoadError> {
    let file: SpellsFile = serde_yaml::from_str(contents)?;

    // `spells:` is a list, so an id typed twice would otherwise collect into the map and
    // lose one of them without a word.
    let mut spells: HashMap<SpellId, Arc<Spell>> = HashMap::with_capacity(file.spells.len());
    for raw in file.spells {
        let spell = Arc::new(raw.into_spell(shapes)?);
        if let Some(first) = spells.get(&spell.id) {
            return Err(SpellsLoadError::DuplicateId {
                id: spell.id,
                first: first.name.clone(),
                second: spell.name.clone(),
            });
        }
        spells.insert(spell.id, spell);
    }
    Ok(spells)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::areas::load_areas;

    fn shape(name: &str) -> HashMap<AreaShapeId, Arc<AreaShape>> {
        HashMap::from([(
            name.to_string(),
            Arc::new(AreaShape::new(vec![(0, 0)].into_boxed_slice())),
        )])
    }

    const AREA_SPELL: &str = r#"
spells:
  - id: 7
    name: Test Wave
    group: attack
    cooldown_ticks: 40
    mana: 25
    level: 18
    vocations: [sorcerer]
    effects:
      - attack:
          target:
            area:
              origin: self
              rotate: true
              shape: probe
          element: fire
          base_power: 40
          level_factor: 0.2
          magic_factor: 1.4
          effect_id: 37
"#;

    fn attack(spell: &Spell) -> (&SpellTarget, u16, u16, Option<MissileId>) {
        match &spell.effects[0] {
            SpellEffect::Attack {
                target,
                level_factor,
                magic_factor,
                missile_id,
                ..
            } => (target, *level_factor, *magic_factor, *missile_id),
        }
    }

    #[test]
    fn an_area_spell_holds_the_shape_it_names() {
        let spells = load_spells_from_str(AREA_SPELL, &shape("probe")).unwrap();
        let spell = &spells[&spells.keys().copied().next().unwrap()];

        match attack(spell).0 {
            SpellTarget::Area {
                origin: AreaOrigin::Caster,
                rotate: true,
                shape,
            } => assert_eq!(
                shape.get_delta(crate::entities::agent::Facing::North),
                [(0, 0)]
            ),
            other => panic!("expected a rotating caster-centred area, got {other:?}"),
        }
    }

    /// A shape name is resolved once, here, so that `cast_spell` has no unknown-shape path.
    /// The cost of that is this error, and it must not be a warning.
    #[test]
    fn a_shape_no_area_file_defines_is_refused() {
        let error = load_spells_from_str(AREA_SPELL, &shape("something_else"))
            .expect_err("a spell pointing at nothing would cast nothing");

        assert!(
            matches!(error, SpellsLoadError::UnknownShape { .. }),
            "unexpected error: {error}"
        );
    }

    /// The scale pin. Authored decimals become hundredths; if this ever changes silently,
    /// every spell's damage moves by a factor of a hundred and nothing fails to compile.
    #[test]
    fn a_factor_is_stored_in_hundredths() {
        let spells = load_spells_from_str(AREA_SPELL, &shape("probe")).unwrap();
        let spell = &spells[&spells.keys().copied().next().unwrap()];
        let (_, level_factor, magic_factor, missile) = attack(spell);

        assert_eq!((level_factor, magic_factor), (20, 140));
        assert_eq!(missile, None, "a wave lands where it is cast");
    }

    #[test]
    fn a_negative_factor_is_refused() {
        let contents = AREA_SPELL.replace("level_factor: 0.2", "level_factor: -0.2");
        let error = load_spells_from_str(&contents, &shape("probe"))
            .expect_err("a negative factor would heal what it hits");

        assert!(
            matches!(
                error,
                SpellsLoadError::Factor {
                    field: "level_factor",
                    ..
                }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn a_reused_id_is_refused_rather_than_overwritten() {
        let contents = format!("{AREA_SPELL}{}", AREA_SPELL.trim_start_matches("\nspells:"));
        let error = load_spells_from_str(&contents, &shape("probe"))
            .expect_err("the second entry would replace the first in the map");

        assert!(
            matches!(error, SpellsLoadError::DuplicateId { .. }),
            "unexpected error: {error}"
        );
    }

    /// Healing is authored the same way and has no planner yet. The loader must refuse the
    /// effect rather than drop it: a spell that loads with its only effect missing is a
    /// spell that costs mana and does nothing.
    #[test]
    fn an_effect_kind_the_server_cannot_run_is_refused() {
        let contents = r#"
spells:
  - id: 1
    name: Light Healing
    group: healing
    cooldown_ticks: 20
    mana: 20
    level: 8
    vocations: [druid]
    effects:
      - heal:
          target: self
          base_power: 8
          level_factor: 0.2
          magic_factor: 1.4
          effect_id: 13
"#;
        let error = load_spells_from_str(contents, &shape("probe"))
            .expect_err("an unknown effect kind must not load as an empty spell");

        assert!(
            matches!(&error, SpellsLoadError::UnknownEffect { kind, .. } if kind == "heal"),
            "unexpected error: {error}"
        );
    }

    /// The catalogue is the real check: every shape name in `spells.yaml` must be a key in
    /// `areas.yaml`, and no test of a fixture can tell you that.
    #[test]
    fn the_shipped_catalogue_loads_and_every_shape_resolves() {
        let areas = load_areas(&CONFIG.areas_file_path).unwrap();
        let spells = load_spells(&CONFIG.spells_file_path, &areas).unwrap();

        assert!(!spells.is_empty(), "loaded no spells at all");

        let named = |name: &str| {
            spells
                .values()
                .find(|spell| spell.name == name)
                .unwrap_or_else(|| panic!("{name} is not among the shipped spells"))
                .clone()
        };

        assert_eq!(attack(&named("Energy Strike")).3, Some(MissileId(36)));
        assert_eq!(attack(&named("Fire Wave")).3, None);
        assert!(matches!(
            attack(&named("Divine Caldera")).0,
            SpellTarget::Area { rotate: false, .. }
        ));
    }

    /// Both catalogues on the path production uses, through the `Lazy`. A failure here is
    /// the poisoned-`Lazy` wall the whole suite hits, so it is worth one cheap test that
    /// names the cause.
    #[test]
    fn both_catalogues_load_through_their_lazies() {
        assert!(!AREA_SHAPES.is_empty());
        assert!(!SPELLS.is_empty());
    }
}
