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
use crate::entities::spells::{
    AreaOrigin, PowerCurve, Spell, SpellAttack, SpellEffect, SpellGroup, SpellHealing, SpellId,
    SpellTargetMode,
};
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
    #[error(
        "spell {id:?} ({name}) has a {field} of {value}, which must be a finite number of 0 or more"
    )]
    Number {
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
    #[error("spell {id:?} ({name}) targets `{target}`, not `self`, `target` or `area`")]
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
    words: String,
    group: SpellGroup,
    #[serde(default)]
    group_cooldown: Option<TickDelta>,
    cooldown_ticks: TickDelta,
    mana: u32,
    level: u16,
    vocations: Vec<Vocation>,
    effects: Vec<serde_yaml::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAttack {
    target: serde_yaml::Value,
    element: CombatElement,
    base_power: f64,
    level_factor: f64,
    magic_factor: f64,
    spread: f64,
    effect_id: EffectId,
    #[serde(default)]
    missile_id: Option<MissileId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHealing {
    target: serde_yaml::Value,
    base_power: f64,
    level_factor: f64,
    magic_factor: f64,
    #[serde(default)]
    spread: f64,
}

/// `self` carries no fields of its own, and this is what refuses one written under it rather
/// than dropping it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCaster {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTargeted {
    range: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawArea {
    origin: RawOrigin,
    #[serde(default)]
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

/// Read as an `f64` and narrowed here so the error can name the number as authored: a value
/// too large for an `f32` becomes an infinity on the way in, and `inf` names nothing.
fn parse_number(
    id: SpellId,
    name: &str,
    field: &'static str,
    value: f64,
) -> Result<f32, SpellsLoadError> {
    let narrowed = value as f32;
    if !narrowed.is_finite() || narrowed < 0.0 {
        return Err(SpellsLoadError::Number {
            id,
            name: name.to_string(),
            field,
            value,
        });
    }
    Ok(narrowed)
}

fn parse_power(
    id: SpellId,
    name: &str,
    base_power: f64,
    level_factor: f64,
    magic_factor: f64,
    spread: f64,
) -> Result<PowerCurve, SpellsLoadError> {
    Ok(PowerCurve {
        base_power: parse_number(id, name, "base_power", base_power)?,
        level_factor: parse_number(id, name, "level_factor", level_factor)?,
        magic_factor: parse_number(id, name, "magic_factor", magic_factor)?,
        spread: parse_number(id, name, "spread", spread)?,
    })
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

/// A target is a mapping tagged by `type:`. Taking the tag out leaves exactly the fields of
/// the mode it names, so `deny_unknown_fields` still catches a typo among them. Hand-rolled
/// rather than an internally tagged serde enum because a `serde_yaml` error raised off a
/// `Value` carries no line, and this way the error names the spell.
fn take_type(value: &mut serde_yaml::Value) -> Option<String> {
    let tag = value.as_mapping_mut()?.remove("type")?;
    Some(tag.as_str()?.to_string())
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
    mut value: serde_yaml::Value,
    shapes: &HashMap<AreaShapeId, Arc<AreaShape>>,
) -> Result<SpellTargetMode, SpellsLoadError> {
    let unknown = |target: String| SpellsLoadError::UnknownTarget {
        id,
        name: name.to_string(),
        target,
    };

    let kind =
        take_type(&mut value).ok_or_else(|| unknown("a mapping without a `type`".to_string()))?;

    match kind.as_str() {
        "self" => {
            let RawCaster {} = serde_yaml::from_value(value)?;
            Ok(SpellTargetMode::Caster)
        }
        "target" => {
            let targeted: RawTargeted = serde_yaml::from_value(value)?;
            Ok(SpellTargetMode::Target {
                range: targeted.range,
            })
        }
        "area" => {
            let area: RawArea = serde_yaml::from_value(value)?;
            let shape =
                shapes
                    .get(&area.shape)
                    .cloned()
                    .ok_or_else(|| SpellsLoadError::UnknownShape {
                        id,
                        name: name.to_string(),
                        shape: area.shape.clone(),
                    })?;

            Ok(SpellTargetMode::Area {
                origin: parse_origin(area.origin),
                shape,
            })
        }
        other => Err(unknown(other.to_string())),
    }
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
            let spell_attack = SpellAttack {
                target: parse_target(id, name, attack.target, shapes)?,
                element: attack.element,
                power: parse_power(
                    id,
                    name,
                    attack.base_power,
                    attack.level_factor,
                    attack.magic_factor,
                    attack.spread,
                )?,
                effect_id: attack.effect_id,
                missile_id: attack.missile_id,
            };
            Ok(SpellEffect::Attack(spell_attack))
        }
        "heal" => {
            let healing: RawHealing = serde_yaml::from_value(payload)?;
            let spell_healing = SpellHealing {
                target: parse_target(id, name, healing.target, shapes)?,
                power: parse_power(
                    id,
                    name,
                    healing.base_power,
                    healing.level_factor,
                    healing.magic_factor,
                    healing.spread,
                )?,
            };
            Ok(SpellEffect::Healing(spell_healing))
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
            words: self.words,
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
    words: test wave
    group: attack
    cooldown_ticks: 40
    mana: 25
    level: 18
    vocations: [sorcerer]
    effects:
      - attack:
          target:
            type: area
            origin: self
            shape: probe
          element: fire
          base_power: 40
          level_factor: 0.2
          magic_factor: 1.4
          spread: 0.25
          effect_id: 37
"#;

    const TARGET_SPELL: &str = r#"
spells:
  - id: 8
    name: Test Strike
    words: test strike
    group: attack
    cooldown_ticks: 40
    mana: 25
    level: 12
    vocations: [sorcerer]
    effects:
      - attack:
          target:
            type: target
            range: 4
          element: energy
          base_power: 45
          level_factor: 0.2
          magic_factor: 1.5
          spread: 0.25
          effect_id: 38
          missile_id: 36
"#;

    fn only_spell(spells: &HashMap<SpellId, Arc<Spell>>) -> Arc<Spell> {
        let id = spells.keys().copied().next().expect("loaded no spells");
        spells[&id].clone()
    }

    fn attack(spell: &Spell) -> &SpellAttack {
        match &spell.effects[0] {
            SpellEffect::Attack(attack) => attack,
            SpellEffect::Healing(_) => panic!("healing spell"),
        }
    }

    fn healing(spell: &Spell) -> &SpellHealing {
        match &spell.effects[0] {
            SpellEffect::Healing(healing) => healing,
            SpellEffect::Attack(_) => panic!("attack spell"),
        }
    }

    #[test]
    fn an_area_spell_holds_the_shape_it_names() {
        let spells = load_spells_from_str(AREA_SPELL, &shape("probe")).unwrap();
        let spell = only_spell(&spells);

        match &attack(&spell).target {
            SpellTargetMode::Area {
                origin: AreaOrigin::Caster,
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

    /// The scale pin. Every number an attack carries reaches `SpellAttack` as authored — the
    /// loader scales nothing — and the damage formula owns what each one means. If this ever
    /// changes silently, every spell's damage moves and nothing fails to compile.
    #[test]
    fn the_numbers_reach_the_spell_as_authored() {
        let spells = load_spells_from_str(AREA_SPELL, &shape("probe")).unwrap();
        let spell = only_spell(&spells);
        let attack = attack(&spell);

        assert_eq!(
            (
                attack.power.base_power,
                attack.power.level_factor,
                attack.power.magic_factor,
                attack.power.spread
            ),
            (40.0, 0.2, 1.4, 0.25)
        );
        assert_eq!(attack.missile_id, None, "a wave lands where it is cast");
    }

    /// The range gates the cast, so a spell that loads without the range it was authored with
    /// is one that reaches further than intended, or not at all.
    #[test]
    fn a_targeted_spell_carries_its_range() {
        let spells = load_spells_from_str(TARGET_SPELL, &shape("probe")).unwrap();
        let spell = only_spell(&spells);

        assert!(
            matches!(attack(&spell).target, SpellTargetMode::Target { range: 4 }),
            "unexpected target mode: {:?}",
            attack(&spell).target
        );
    }

    /// `type:` names the mode, and a name this server has no mode for must not load as some
    /// other mode — nor as a spell whose target silently defaults.
    #[test]
    fn a_target_type_the_server_cannot_run_is_refused() {
        let contents = TARGET_SPELL.replace("type: target", "type: cone");
        let error = load_spells_from_str(&contents, &shape("probe"))
            .expect_err("an unknown target mode must not load");

        assert!(
            matches!(&error, SpellsLoadError::UnknownTarget { target, .. } if target == "cone"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn a_negative_factor_is_refused() {
        let contents = AREA_SPELL.replace("level_factor: 0.2", "level_factor: -0.2");
        let error = load_spells_from_str(&contents, &shape("probe"))
            .expect_err("a negative factor would heal what it hits");

        assert!(
            matches!(
                error,
                SpellsLoadError::Number {
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

    /// The loader must refuse an effect it cannot run rather than drop it: a spell that
    /// loads with its only effect missing is a spell that costs mana and does nothing.
    /// `summon` stands in for the next kind authored ahead of its planner, which is what
    /// `heal` was until it got one.
    #[test]
    fn an_effect_kind_the_server_cannot_run_is_refused() {
        let contents = r#"
spells:
  - id: 1
    name: Test Summon
    words: test summon
    group: support
    cooldown_ticks: 20
    mana: 20
    level: 8
    vocations: [druid]
    effects:
      - summon:
          kind: rat
          count: 2
"#;
        let error = load_spells_from_str(contents, &shape("probe"))
            .expect_err("an unknown effect kind must not load as an empty spell");

        assert!(
            matches!(&error, SpellsLoadError::UnknownEffect { kind, .. } if kind == "summon"),
            "unexpected error: {error}"
        );
    }

    /// A heal carries neither an element nor an effect id, so nothing but this says the
    /// numbers under `heal:` reach `SpellHealing` as authored. An unwritten `spread`
    /// restores a flat amount rather than defaulting to some variance.
    #[test]
    fn a_healing_spell_carries_its_numbers_and_defaults_its_spread() {
        let contents = r#"
spells:
  - id: 9
    name: Test Healing
    words: test healing
    group: healing
    cooldown_ticks: 20
    mana: 20
    level: 8
    vocations: [druid]
    effects:
      - heal:
          target:
            type: self
          base_power: 8
          level_factor: 0.2
          magic_factor: 1.4
"#;
        let spells = load_spells_from_str(contents, &shape("probe")).unwrap();
        let spell = only_spell(&spells);

        let healing = healing(&spell);
        assert!(matches!(healing.target, SpellTargetMode::Caster));
        assert_eq!(
            (
                healing.power.base_power,
                healing.power.level_factor,
                healing.power.magic_factor,
                healing.power.spread
            ),
            (8.0, 0.2, 1.4, 0.0)
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

        assert_eq!(
            attack(&named("Energy Strike")).missile_id,
            Some(MissileId(36))
        );
        assert!(matches!(
            attack(&named("Energy Strike")).target,
            SpellTargetMode::Target { range: 3 }
        ));
        assert_eq!(attack(&named("Fire Wave")).missile_id, None);
        assert!(matches!(
            healing(&named("Light Healing")).target,
            SpellTargetMode::Caster
        ));
        assert!(matches!(
            attack(&named("Divine Caldera")).target,
            SpellTargetMode::Area { .. }
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
