use slotmap::SlotMap;
use smallvec::SmallVec;
use std::collections::HashMap;
use thiserror::Error;

use crate::constants::MAX_VISIBLE_ITEMS;
use crate::entities::agent::{Agent, AgentKey};
use crate::entities::items::{Item, ItemAttribute, ItemFlag, ItemGuid};
use crate::entities::position::{ItemPlacement, Position};

#[derive(Debug, Clone)]
pub struct MapTile {
    items: SmallVec<[Item; MAX_VISIBLE_ITEMS]>,
    agents: SmallVec<[AgentKey; 1]>,
}

#[derive(Error, Debug)]
pub enum MapError {
    #[error("Tile position does not exist")]
    TileDoesNotExist,
    #[error("Entity does not exist at this position")]
    EntityNotInPosition,
    #[error("Container is full")]
    ContainerIsFull,
}

#[derive(Debug, Clone)]
pub struct GameMap {
    tiles: HashMap<Position, MapTile>,
    agents: SlotMap<AgentKey, Agent>,
    agent_positions: HashMap<AgentKey, Position>,
}

impl MapTile {
    pub fn new() -> Self {
        MapTile {
            items: SmallVec::new(),
            agents: SmallVec::new(),
        }
    }

    pub fn push_item(&mut self, item: Item) {
        self.items.push(item);
    }
}

impl GameMap {
    pub fn new() -> Self {
        GameMap {
            tiles: HashMap::new(),
            agents: SlotMap::with_key(),
            agent_positions: HashMap::new(),
        }
    }

    pub fn insert_tile(&mut self, pos: Position, tile: MapTile) {
        self.tiles.insert(pos, tile);
    }

    fn get_tile_mut(&mut self, pos: &Position) -> Result<&mut MapTile, MapError> {
        if let Some(tile) = self.tiles.get_mut(pos) {
            return Ok(tile);
        }
        Err(MapError::TileDoesNotExist)
    }

    fn get_tile(&self, pos: &Position) -> Result<&MapTile, MapError> {
        if let Some(tile) = self.tiles.get(pos) {
            return Ok(tile);
        }
        Err(MapError::TileDoesNotExist)
    }

    /// Insert an agent at `pos`. Maintains tile agent list and reverse index atomically.
    pub fn insert_agent(&mut self, agent: Agent, pos: &Position) -> Result<AgentKey, MapError> {
        // Validate tile exists before inserting the agent.
        if !self.tiles.contains_key(pos) {
            return Err(MapError::TileDoesNotExist);
        }
        let key = self.agents.insert(agent);
        self.tiles.get_mut(pos).unwrap().agents.push(key);
        self.agent_positions.insert(key, pos.clone());
        Ok(key)
    }

    /// Remove an agent entirely. Returns the `Agent` on success.
    pub fn remove_agent(&mut self, key: AgentKey) -> Option<Agent> {
        if let Some(pos) = self.agent_positions.remove(&key) {
            if let Some(tile) = self.tiles.get_mut(&pos) {
                if let Some(idx) = tile.agents.iter().position(|k| *k == key) {
                    tile.agents.remove(idx);
                }
            }
        }
        self.agents.remove(key)
    }

    /// Move an agent to `new_pos`. Maintains tile lists and reverse index atomically.
    pub fn move_agent(&mut self, key: AgentKey, new_pos: &Position) -> Result<(), MapError> {
        let old_pos = self
            .agent_positions
            .get(&key)
            .cloned()
            .ok_or(MapError::EntityNotInPosition)?;
        let old_tile = self.get_tile_mut(&old_pos)?;
        if let Some(idx) = old_tile.agents.iter().position(|k| *k == key) {
            old_tile.agents.remove(idx);
        }
        let new_tile = self.get_tile_mut(new_pos)?;
        new_tile.agents.push(key);
        self.agent_positions.insert(key, new_pos.clone());
        Ok(())
    }

    pub fn agent_position(&self, key: AgentKey) -> Option<&Position> {
        self.agent_positions.get(&key)
    }

    pub fn get_agent(&self, key: AgentKey) -> Option<&Agent> {
        self.agents.get(key)
    }

    pub fn get_agent_mut(&mut self, key: AgentKey) -> Option<&mut Agent> {
        self.agents.get_mut(key)
    }

    pub fn can_move(&self, pos: &Position, _key: AgentKey) -> bool {
        let tile = self.get_tile(pos);
        if tile.is_err() {
            return false;
        }
        let tile = tile.unwrap();

        let has_ground = tile
            .items
            .iter()
            .any(|i| i.config.has_flag(ItemFlag::Ground));
        if !has_ground {
            return false;
        }
        let unpass = tile
            .items
            .iter()
            .any(|i| i.config.has_flag(ItemFlag::Unpass));
        if unpass {
            return false;
        }

        // TODO: check agent colision

        true
    }

    pub fn tile_friction(&self, pos: &Position) -> Option<u32> {
        let Ok(tile) = self.get_tile(pos) else {
            return None;
        };
        let friction = tile.items.iter().find_map(|i| {
            i.config.get_attributes().find_map(|attr| match attr {
                ItemAttribute::TileFriction(f) => Some(*f),
                _ => None,
            })
        });
        friction
    }

    pub fn get_visible_items(
        &self,
        pos: &Position,
    ) -> Result<impl Iterator<Item = &Item>, MapError> {
        let tile = self.get_tile(pos)?;
        Ok(tile.items.iter().take(MAX_VISIBLE_ITEMS))
    }

    pub fn get_item_at(&self, pos: &Position, index: usize) -> Option<&Item> {
        let Ok(tile) = self.get_tile(pos) else {
            return None;
        };
        tile.items.get(index)
    }

    pub fn can_drop_item(&self, pos: &Position) -> bool {
        let Ok(tile) = self.get_tile(pos) else {
            return false;
        };
        tile.items
            .iter()
            .any(|i| i.config.has_flag(ItemFlag::FullBank))
            && !tile
                .items
                .iter()
                .any(|i| i.config.has_flag(ItemFlag::Bottom))
    }

    pub fn remove_item(
        &mut self,
        placement: &ItemPlacement,
        guid: ItemGuid,
        amount: u8,
    ) -> Option<(Item, Option<(ItemGuid, usize)>)> {
        match placement {
            ItemPlacement::Map(pos) => {
                let tile = self.get_tile_mut(pos).ok()?;

                if let Some(idx) = tile.items.iter().position(|i| i.guid == guid) {
                    let current_amount = tile.items[idx].amount;
                    if current_amount > amount {
                        let item = &mut tile.items[idx];
                        item.amount -= amount;
                        return Some((
                            Item {
                                guid: ItemGuid::new(),
                                config: item.config.clone(),
                                item_id: item.item_id,
                                amount,
                                content: None,
                            },
                            None,
                        ));
                    } else if current_amount == amount {
                        return Some((tile.items.remove(idx), None));
                    }
                    return None;
                }

                for item in tile.items.iter_mut() {
                    if let Some(content) = &mut item.content {
                        let found = Self::remove_from_container(&item.guid, content, &guid, amount);
                        if found.is_some() {
                            return found;
                        }
                    }
                }
            }
            ItemPlacement::Inventory(slot, agent_key) => {
                let agent = self.get_agent_mut(*agent_key)?;
                let player = agent.get_player_mut()?;
                if let Some(item) = player.inventory.get_mut(slot) {
                    if item.guid == guid {
                        if item.amount > amount {
                            item.amount -= amount;
                            return Some((
                                Item {
                                    guid: ItemGuid::new(),
                                    config: item.config.clone(),
                                    item_id: item.item_id,
                                    amount,
                                    content: None,
                                },
                                None,
                            ));
                        } else if item.amount == amount {
                            return Some((player.inventory.remove(slot).unwrap(), None));
                        }
                    } else if let Some(content) = &mut item.content {
                        return Self::remove_from_container(&item.guid, content, &guid, amount);
                    }
                }
            }
        };
        None
    }

    fn remove_from_container(
        parent_guid: &ItemGuid,
        items: &mut Vec<Item>,
        guid: &ItemGuid,
        amount: u8,
    ) -> Option<(Item, Option<(ItemGuid, usize)>)> {
        if let Some(idx) = items.iter().position(|i| i.guid == *guid) {
            let current_amount = items[idx].amount;
            if current_amount > amount {
                let item = &mut items[idx];
                item.amount -= amount;
                return Some((
                    Item {
                        guid: ItemGuid::new(),
                        config: item.config.clone(),
                        item_id: item.item_id,
                        amount,
                        content: None,
                    },
                    Some((item.guid.clone(), idx)),
                ));
            } else if current_amount == amount {
                return Some((items.remove(idx), Some((parent_guid.clone(), idx))));
            }
            return None;
        }

        for item in items.iter_mut() {
            if let Some(content) = &mut item.content {
                let found = Self::remove_from_container(&item.guid, content, guid, amount);
                if found.is_some() {
                    return found;
                }
            }
        }
        None
    }

    pub fn drop_item(&mut self, pos: &Position, item: Item) -> Result<(), MapError> {
        let tile = self.get_tile_mut(pos)?;
        tile.items.push(item);
        Ok(())
    }

    pub fn add_to_container(
        &mut self,
        placement: &ItemPlacement,
        target_container: &ItemGuid,
        container_slot: usize,
        item: Item,
    ) -> Result<(), MapError> {
        match placement {
            ItemPlacement::Map(pos) => {
                let tile = self.get_tile_mut(pos)?;
                for existing_item in &mut tile.items {
                    if let Some(container) =
                        Self::find_container_mut(existing_item, target_container)
                    {
                        if let Some(content) = &mut container.content {
                            let cap = container
                                .config
                                .get_attributes()
                                .find_map(|attr| match attr {
                                    ItemAttribute::Capacity(c) => Some(*c),
                                    _ => None,
                                })
                                .unwrap();
                            if content.len() >= cap as usize {
                                return Err(MapError::ContainerIsFull);
                            }
                            content.insert(container_slot, item);
                            return Ok(());
                        }
                    }
                }
            }
            ItemPlacement::Inventory(inventory_slot, agent_key) => {
                let Some(agent) = self.get_agent_mut(*agent_key) else {
                    return Err(MapError::EntityNotInPosition);
                };
                let Some(player) = agent.get_player_mut() else {
                    return Err(MapError::EntityNotInPosition);
                };
                let Some(slot_item) = player.inventory.get_mut(inventory_slot) else {
                    return Err(MapError::EntityNotInPosition);
                };
                let Some(container) = Self::find_container_mut(slot_item, target_container) else {
                    return Err(MapError::EntityNotInPosition);
                };
                if let Some(content) = &mut container.content {
                    let cap = container
                        .config
                        .get_attributes()
                        .find_map(|attr| match attr {
                            ItemAttribute::Capacity(c) => Some(*c),
                            _ => None,
                        })
                        .unwrap();
                    if content.len() >= cap as usize {
                        return Err(MapError::ContainerIsFull);
                    }
                    content.insert(container_slot, item);
                    return Ok(());
                }
            }
        }

        Err(MapError::EntityNotInPosition)
    }

    fn find_container_mut<'a>(item: &'a mut Item, target: &ItemGuid) -> Option<&'a mut Item> {
        if item.guid == *target {
            return Some(item);
        }
        if let Some(content) = &mut item.content {
            for inner in content {
                if let Some(found) = Self::find_container_mut(inner, target) {
                    return Some(found);
                }
            }
        }
        None
    }

    pub fn get_parent_container(&self, pos: &Position, guid: &ItemGuid) -> Option<&ItemGuid> {
        let Ok(tile) = self.get_tile(pos) else {
            return None;
        };

        for it in tile.items.iter() {
            if let Some((parent_guid, _)) = Self::find_by_id_inner(it, guid, None) {
                return parent_guid;
            }
        }
        None
    }

    pub fn get_item_by_id(&self, pos: &Position, guid: &ItemGuid) -> Option<&Item> {
        let Ok(tile) = self.get_tile(pos) else {
            return None;
        };

        for it in tile.items.iter() {
            if let Some((_, item)) = Self::find_by_id_inner(it, guid, None) {
                return Some(item);
            }
        }
        None
    }

    fn find_by_id_inner<'a>(
        item: &'a Item,
        guid: &ItemGuid,
        parent_guid: Option<&'a ItemGuid>,
    ) -> Option<(Option<&'a ItemGuid>, &'a Item)> {
        if item.guid == *guid {
            return Some((parent_guid, item));
        }
        if let Some(content) = &item.content {
            for inner in content {
                if let Some(found) = Self::find_by_id_inner(inner, guid, Some(&item.guid)) {
                    return Some(found);
                }
            }
        }
        None
    }
}
