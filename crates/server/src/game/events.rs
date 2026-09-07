use std::collections::HashSet;
use std::sync::Arc;

use crate::entities::{
    agent::{AgentKey, Facing},
    combat::CombatDamage,
    creature::BloodType,
    effects::Missile,
    inventory::InventorySlot,
    items::{ItemGuid, ItemRef},
    position::{Direction, Position},
    skills::SkillType,
};
use crate::persistence::player::PlayerSnapshot;

#[derive(Clone, Debug)]
pub enum BroadcastMessage {
    PlayerSpawned {
        agent_key: AgentKey,
        position: Position,
    },
    AgentMoved {
        agent_key: AgentKey,
        direction: Direction,
        from_position: Position,
        to_position: Position,
    },
    TileChanged {
        position: Position,
    },
    MoveItemDenied {
        agent_key: AgentKey,
        message: String,
    },
    OpenContainer {
        agent_key: AgentKey,
        item: ItemRef,
    },
    UseItemDenied {
        agent_key: AgentKey,
        message: String,
    },
    ContainerUpdated {
        item: ItemRef,
    },
    AgentWalkDenied {
        agent_key: AgentKey,
    },
    UpdateInventorySlot {
        agent_key: AgentKey,
        slot: InventorySlot,
    },
    AgentChangedDirection {
        agent_key: AgentKey,
        facing: Facing,
        position: Position,
    },
    AgentTeleported {
        agent_key: AgentKey,
        from_position: Position,
        to_position: Position,
    },
    AgentDespawned {
        agent_key: AgentKey,
        position: Position,
        snapshot: Option<Arc<PlayerSnapshot>>,
    },
    LogoutDenied {
        agent_key: AgentKey,
    },
    AgentSaid {
        agent_key: AgentKey,
        message: String,
    },
    AgentLostTarget {
        agent_key: AgentKey,
        seq: u32,
    },
    DamageTaken {
        agent_key: AgentKey,
        position: Position,
        blood_type: Option<BloodType>,
        damage: CombatDamage,
    },
    MissileLaunched {
        missile: Missile,
    },
    AttackMissed {
        position: Position,
    },
    SkillProgressUpdated {
        agent_key: AgentKey,
        skill_type: SkillType,
        amount: u64,
    },
    SkillUpgraded {
        agent_key: AgentKey,
        skill_type: SkillType,
        gained: u16,
        amount: u64,
    },
    PlayerManaUpdated {
        agent_key: AgentKey,
    },
    AgentLifeUpdated {
        agent_key: AgentKey,
    },
    PotionDrunk {
        target: AgentKey,
        position: Position,
    },
}

#[derive(PartialEq, Eq, Hash)]
enum RefreshKey {
    Tile(Position),
    Life(AgentKey),
    Mana(AgentKey),
    Slot(AgentKey, InventorySlot),
    Container(ItemGuid),
}

impl BroadcastMessage {
    fn refresh_key(&self) -> Option<RefreshKey> {
        match self {
            BroadcastMessage::TileChanged { position } => Some(RefreshKey::Tile(position.clone())),
            BroadcastMessage::AgentLifeUpdated { agent_key } => Some(RefreshKey::Life(*agent_key)),
            BroadcastMessage::PlayerManaUpdated { agent_key } => Some(RefreshKey::Mana(*agent_key)),
            BroadcastMessage::UpdateInventorySlot { agent_key, slot } => {
                Some(RefreshKey::Slot(*agent_key, *slot))
            }
            BroadcastMessage::ContainerUpdated { item } => {
                Some(RefreshKey::Container(item.guid.clone()))
            }
            _ => None,
        }
    }
}

/// Drops the refresh messages a later one in the same tick makes redundant, keeping the
/// **last** of each key.
pub fn dedupe_refreshes(messages: &mut Vec<BroadcastMessage>) {
    let mut seen: HashSet<RefreshKey> = HashSet::new();
    let mut kept = Vec::with_capacity(messages.len());
    for message in messages.drain(..).rev() {
        if let Some(key) = message.refresh_key()
            && !seen.insert(key)
        {
            continue;
        }
        kept.push(message);
    }
    kept.reverse();
    *messages = kept;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::combat::{CombatDamage, CombatElement};
    use crate::entities::position::ItemPlacement;
    use slotmap::KeyData;

    fn key(n: u64) -> AgentKey {
        AgentKey::from(KeyData::from_ffi((1 << 32) | n))
    }

    fn tile(x: u16) -> BroadcastMessage {
        BroadcastMessage::TileChanged {
            position: Position::new(x, 10, 7),
        }
    }

    fn container(guid: &str, placement: ItemPlacement) -> BroadcastMessage {
        BroadcastMessage::ContainerUpdated {
            item: ItemRef {
                guid: ItemGuid(guid.to_owned()),
                placement,
            },
        }
    }

    fn hit(agent_key: AgentKey, value: u32) -> BroadcastMessage {
        BroadcastMessage::DamageTaken {
            agent_key,
            position: Position::new(10, 10, 7),
            blood_type: Some(BloodType::Blood),
            damage: CombatDamage {
                element: CombatElement::Physical,
                value,
                blocked_shield: false,
                blocked_armor: false,
            },
        }
    }

    #[test]
    fn three_splashes_on_one_tile_refresh_it_once() {
        let mut msgs = vec![tile(10), tile(11), tile(10), tile(10)];

        dedupe_refreshes(&mut msgs);

        assert!(
            matches!(
                msgs.as_slice(),
                [
                    BroadcastMessage::TileChanged { position: first },
                    BroadcastMessage::TileChanged { position: second },
                ] if first.x == 11 && second.x == 10
            ),
            "{msgs:?}"
        );
    }

    #[test]
    fn a_creature_hit_by_two_players_has_its_life_read_once() {
        let rat = key(1);
        let mut msgs = vec![
            BroadcastMessage::AgentLifeUpdated { agent_key: rat },
            BroadcastMessage::AgentLifeUpdated { agent_key: key(2) },
            BroadcastMessage::AgentLifeUpdated { agent_key: rat },
        ];

        dedupe_refreshes(&mut msgs);

        assert_eq!(msgs.len(), 2, "{msgs:?}");
    }

    #[test]
    fn the_same_slot_collapses_but_a_different_one_does_not() {
        let player = key(1);
        let mut msgs = vec![
            BroadcastMessage::UpdateInventorySlot {
                agent_key: player,
                slot: InventorySlot::RightHand,
            },
            BroadcastMessage::UpdateInventorySlot {
                agent_key: player,
                slot: InventorySlot::Backpack,
            },
            BroadcastMessage::UpdateInventorySlot {
                agent_key: player,
                slot: InventorySlot::RightHand,
            },
            BroadcastMessage::UpdateInventorySlot {
                agent_key: key(2),
                slot: InventorySlot::RightHand,
            },
        ];

        dedupe_refreshes(&mut msgs);

        assert_eq!(msgs.len(), 3, "{msgs:?}");
    }

    /// Keeping the first would refresh the container where it no longer is, and the
    /// session resolves that lookup against the end-of-tick snapshot: it would fail.
    #[test]
    fn a_container_is_refreshed_at_the_placement_it_ended_in() {
        let player = key(1);
        let mut msgs = vec![
            container("bag", ItemPlacement::Map(Position::new(10, 10, 7))),
            container(
                "bag",
                ItemPlacement::Inventory(InventorySlot::Backpack, player),
            ),
        ];

        dedupe_refreshes(&mut msgs);

        assert!(
            matches!(
                msgs.as_slice(),
                [BroadcastMessage::ContainerUpdated { item }]
                    if item.placement == ItemPlacement::Inventory(InventorySlot::Backpack, player)
            ),
            "{msgs:?}"
        );
    }

    #[test]
    fn two_hits_of_the_same_size_stay_two_damage_numbers() {
        let rat = key(1);
        let mut msgs = vec![hit(rat, 5), hit(rat, 5)];

        dedupe_refreshes(&mut msgs);

        assert_eq!(msgs.len(), 2, "{msgs:?}");
    }

    #[test]
    fn every_experience_award_survives() {
        let player = key(1);
        let award = || BroadcastMessage::SkillProgressUpdated {
            agent_key: player,
            skill_type: SkillType::Level,
            amount: 40,
        };
        let mut msgs = vec![award(), award()];

        dedupe_refreshes(&mut msgs);

        assert_eq!(msgs.len(), 2, "{msgs:?}");
    }

    #[test]
    fn events_keep_their_order_around_a_collapsed_refresh() {
        let rat = key(1);
        let mut msgs = vec![
            tile(10),
            hit(rat, 5),
            tile(10),
            BroadcastMessage::AgentDespawned {
                agent_key: rat,
                position: Position::new(10, 10, 7),
                snapshot: None,
            },
        ];

        dedupe_refreshes(&mut msgs);

        assert!(
            matches!(
                msgs.as_slice(),
                [
                    BroadcastMessage::DamageTaken { .. },
                    BroadcastMessage::TileChanged { .. },
                    BroadcastMessage::AgentDespawned { .. },
                ]
            ),
            "{msgs:?}"
        );
    }

    #[test]
    fn an_empty_tick_stays_empty() {
        let mut msgs: Vec<BroadcastMessage> = Vec::new();

        dedupe_refreshes(&mut msgs);

        assert!(msgs.is_empty());
    }
}
