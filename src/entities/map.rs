use slotmap::SlotMap;
use smallvec::SmallVec;
use std::{collections::HashMap, sync::Arc};
use thiserror::Error;

use crate::constants::MAX_VISIBLE_ITEMS;
use crate::entities::agent::{Agent, AgentKey};
use crate::entities::items::{FloorChangeDirection, Item, ItemAttribute, ItemFlag, ItemGuid};
use crate::entities::player::Player;
use crate::entities::position::{Position, Rect};

pub type RemovedItem = (Item, Option<usize>, Option<(ItemGuid, usize)>);

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

const CHUNK_BITS: u16 = 4;
const CHUNK_SIDE: u16 = 1 << CHUNK_BITS;
const CHUNK_MASK: u16 = CHUNK_SIDE - 1;
const CHUNK_AREA: usize = (CHUNK_SIDE as usize) * (CHUNK_SIDE as usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ChunkCoord {
    cx: u16,
    cy: u16,
    z: u8,
}

impl ChunkCoord {
    fn from_pos(pos: &Position) -> Self {
        ChunkCoord {
            cx: pos.x >> CHUNK_BITS,
            cy: pos.y >> CHUNK_BITS,
            z: pos.z,
        }
    }
}

fn local_index(pos: &Position) -> usize {
    let lx = (pos.x & CHUNK_MASK) as usize;
    let ly = (pos.y & CHUNK_MASK) as usize;
    ly * CHUNK_SIDE as usize + lx
}

#[derive(Debug, Clone)]
struct Chunk {
    tiles: Box<[Option<MapTile>]>,
}

impl Chunk {
    fn new() -> Self {
        let tiles = (0..CHUNK_AREA)
            .map(|_| None)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Chunk { tiles }
    }
}

#[derive(Debug, Clone)]
pub struct GameMap {
    chunks: HashMap<ChunkCoord, Arc<Chunk>>,
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

    pub fn visible_items(&self) -> impl Iterator<Item = &Item> {
        self.items.iter().take(MAX_VISIBLE_ITEMS)
    }
}

impl GameMap {
    pub fn new() -> Self {
        GameMap {
            chunks: HashMap::new(),
            agents: SlotMap::with_key(),
            agent_positions: HashMap::new(),
        }
    }

    pub fn insert_tile(&mut self, pos: Position, tile: MapTile) {
        let coord = ChunkCoord::from_pos(&pos);
        let idx = local_index(&pos);
        let chunk = self
            .chunks
            .entry(coord)
            .or_insert_with(|| Arc::new(Chunk::new()));
        Arc::make_mut(chunk).tiles[idx] = Some(tile);
    }

    fn contains_tile(&self, pos: &Position) -> bool {
        self.get_tile(pos).is_ok()
    }

    fn get_tile_mut(&mut self, pos: &Position) -> Result<&mut MapTile, MapError> {
        let idx = local_index(pos);
        let chunk = self
            .chunks
            .get_mut(&ChunkCoord::from_pos(pos))
            .ok_or(MapError::TileDoesNotExist)?;
        Arc::make_mut(chunk).tiles[idx]
            .as_mut()
            .ok_or(MapError::TileDoesNotExist)
    }

    fn get_tile(&self, pos: &Position) -> Result<&MapTile, MapError> {
        self.chunks
            .get(&ChunkCoord::from_pos(pos))
            .and_then(|chunk| chunk.tiles[local_index(pos)].as_ref())
            .ok_or(MapError::TileDoesNotExist)
    }

    /// Insert an agent at `pos`. Maintains tile agent list and reverse index atomically.
    pub fn insert_agent(&mut self, agent: Agent, pos: &Position) -> Result<AgentKey, MapError> {
        // Validate tile exists before inserting the agent.
        if !self.contains_tile(pos) {
            return Err(MapError::TileDoesNotExist);
        }
        let key = self.agents.insert(agent);
        self.get_tile_mut(pos).unwrap().agents.push(key);
        self.agent_positions.insert(key, pos.clone());
        Ok(key)
    }

    /// Remove an agent entirely. Returns the `Agent` on success.
    pub fn remove_agent(&mut self, key: AgentKey) -> Option<Agent> {
        if let Some(pos) = self.agent_positions.remove(&key)
            && let Ok(tile) = self.get_tile_mut(&pos)
            && let Some(idx) = tile.agents.iter().position(|k| *k == key)
        {
            tile.agents.remove(idx);
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

    pub fn get_player(&self, key: AgentKey) -> Option<&Player> {
        self.agents.get(key)?.get_player()
    }

    pub fn get_player_mut(&mut self, key: AgentKey) -> Option<&mut Player> {
        self.agents.get_mut(key)?.get_player_mut()
    }

    pub fn iter_agents_at(
        &self,
        pos: &Position,
    ) -> Result<impl Iterator<Item = &AgentKey> + '_, MapError> {
        let tile = self.get_tile(pos)?;
        Ok(tile.agents.iter())
    }

    pub fn iter_agents_in_rect<'a>(
        &'a self,
        rect: &Rect,
        z: u8,
    ) -> impl Iterator<Item = &'a AgentKey> + use<'a> {
        let (x0, y0) = (rect.min_x(), rect.min_y());
        let (x1, y1) = (rect.max_x(), rect.max_y());
        let cx_range = (x0 >> CHUNK_BITS)..=(x1 >> CHUNK_BITS);
        let cy_range = (y0 >> CHUNK_BITS)..=(y1 >> CHUNK_BITS);

        cy_range
            .flat_map(move |cy| cx_range.clone().map(move |cx| (cx, cy)))
            .filter_map(move |(cx, cy)| {
                self.chunks
                    .get(&ChunkCoord { cx, cy, z })
                    .map(|chunk| (cx, cy, chunk))
            })
            .flat_map(move |(cx, cy, chunk)| {
                // Clamp the rect to this chunk's bounds, in chunk-local coords.
                let base_x = cx << CHUNK_BITS;
                let base_y = cy << CHUNK_BITS;
                let lx0 = x0.max(base_x) - base_x;
                let lx1 = x1.min(base_x + CHUNK_MASK) - base_x;
                let ly0 = y0.max(base_y) - base_y;
                let ly1 = y1.min(base_y + CHUNK_MASK) - base_y;

                (ly0..=ly1)
                    .flat_map(move |ly| {
                        (lx0..=lx1).filter_map(move |lx| {
                            chunk.tiles[ly as usize * CHUNK_SIDE as usize + lx as usize].as_ref()
                        })
                    })
                    .flat_map(|tile| tile.agents.iter())
            })
    }

    pub fn iter_tiles_in_rect<'a>(
        &'a self,
        rect: &Rect,
        z: u8,
    ) -> impl Iterator<Item = (Position, &'a MapTile)> + use<'a> {
        let (x0, y0) = (rect.min_x(), rect.min_y());
        let (x1, y1) = (rect.max_x(), rect.max_y());
        let cx_range = (x0 >> CHUNK_BITS)..=(x1 >> CHUNK_BITS);
        let cy_range = (y0 >> CHUNK_BITS)..=(y1 >> CHUNK_BITS);

        cy_range
            .flat_map(move |cy| cx_range.clone().map(move |cx| (cx, cy)))
            .filter_map(move |(cx, cy)| {
                self.chunks
                    .get(&ChunkCoord { cx, cy, z })
                    .map(|chunk| (cx, cy, chunk))
            })
            .flat_map(move |(cx, cy, chunk)| {
                // Clamp the rect to this chunk's bounds, in chunk-local coords.
                let base_x = cx << CHUNK_BITS;
                let base_y = cy << CHUNK_BITS;
                let lx0 = x0.max(base_x) - base_x;
                let lx1 = x1.min(base_x + CHUNK_MASK) - base_x;
                let ly0 = y0.max(base_y) - base_y;
                let ly1 = y1.min(base_y + CHUNK_MASK) - base_y;

                (ly0..=ly1).flat_map(move |ly| {
                    (lx0..=lx1).filter_map(move |lx| {
                        chunk.tiles[ly as usize * CHUNK_SIDE as usize + lx as usize]
                            .as_ref()
                            .map(|tile| (Position::new(base_x + lx, base_y + ly, z), tile))
                    })
                })
            })
    }

    pub fn iter_agents(&self) -> impl Iterator<Item = (AgentKey, &Agent)> {
        self.agents.iter()
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
        tile.items.iter().find_map(|i| {
            i.config.get_attributes().find_map(|attr| match attr {
                ItemAttribute::TileFriction(f) => Some(*f),
                _ => None,
            })
        })
    }

    pub fn get_floor_change(&self, pos: &Position) -> Option<FloorChangeDirection> {
        let Ok(tile) = self.get_tile(pos) else {
            return None;
        };
        tile.items.iter().find_map(|it| {
            it.config.get_attributes().find_map(|attr| match attr {
                ItemAttribute::FloorChange(dir) => Some(*dir),
                _ => None,
            })
        })
    }

    pub fn get_visible_items(
        &self,
        pos: &Position,
    ) -> Result<impl Iterator<Item = &Item>, MapError> {
        let tile = self.get_tile(pos)?;
        Ok(tile.items.iter().take(MAX_VISIBLE_ITEMS))
    }

    pub fn get_top_item(&self, pos: &Position) -> Option<&Item> {
        let Ok(tile) = self.get_tile(pos) else {
            return None;
        };
        tile.items.last()
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

    pub fn remove_item_from_tile(
        &mut self,
        pos: &Position,
        guid: &ItemGuid,
        amount: u8,
    ) -> Option<RemovedItem> {
        let tile = self.get_tile_mut(pos).ok()?;

        if let Some(idx) = tile.items.iter().position(|i| i.guid == *guid) {
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
                    Some(idx),
                    None,
                ));
            } else if current_amount == amount {
                return Some((tile.items.remove(idx), Some(idx), None));
            }
            return None;
        }

        for item in tile.items.iter_mut() {
            if let Some(content) = &mut item.content {
                let found = Self::remove_from_container(&item.guid, content, guid, amount);
                if let Some((item, parent)) = found {
                    return Some((item, None, parent));
                }
            }
        }
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

    /// Place `item` at `pos`.
    ///
    /// - `container`: if `None`, pushes directly onto the tile.
    /// - `container`: if `Some((guid, slot))`, finds that container on the tile
    ///   and inserts the item at `slot` within it.
    pub fn place_item(
        &mut self,
        pos: &Position,
        index: Option<usize>,
        container: Option<(&ItemGuid, usize)>,
        item: Item,
    ) -> Result<(), MapError> {
        match container {
            None => {
                let tile = self.get_tile_mut(pos)?;
                tile.items.insert(index.unwrap_or(tile.items.len()), item);
                Ok(())
            }
            Some((target, slot)) => {
                let tile = self.get_tile_mut(pos)?;
                for existing_item in &mut tile.items {
                    if let Some(c) = existing_item.find_by_guid_mut(target) {
                        let cap = c.container_capacity().unwrap();
                        if let Some(content) = &mut c.content {
                            if content.len() >= cap as usize {
                                return Err(MapError::ContainerIsFull);
                            }
                            content.insert(slot, item);
                            return Ok(());
                        }
                    }
                }
                Err(MapError::EntityNotInPosition)
            }
        }
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
        tile.items.iter().find_map(|it| it.find_by_guid(guid))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::agent::{Agent, Pool};
    use crate::entities::creature::CreatureKind;
    use crate::entities::position::Position;

    fn new_creature() -> Agent {
        Agent::from_creature_kind(&CreatureKind {
            name: "Creature".to_string(),
            life: Pool {
                current: 1,
                maximum: 1,
            },
            outfit: (1, (0, 0, 0, 0)),
            speed: 1,
            skills: HashMap::new(),
        })
    }

    fn map_with_one_tile(pos: &Position) -> GameMap {
        let mut map = GameMap::new();
        map.insert_tile(pos.clone(), MapTile::new());
        map
    }

    #[test]
    fn iter_agents_yields_each_inserted_agent() {
        let pos = Position::new(100, 100, 7);
        let mut map = map_with_one_tile(&pos);
        let k1 = map.insert_agent(new_creature(), &pos).unwrap();
        let k2 = map.insert_agent(new_creature(), &pos).unwrap();
        let keys: Vec<_> = map.iter_agents().map(|(k, _)| k).collect();
        assert!(keys.contains(&k1));
        assert!(keys.contains(&k2));
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn insert_agent_fails_when_tile_does_not_exist() {
        let mut map = GameMap::new();
        let pos = Position::new(5, 5, 7);
        assert!(matches!(
            map.insert_agent(new_creature(), &pos),
            Err(MapError::TileDoesNotExist)
        ));
    }

    #[test]
    fn tiles_sharing_a_local_index_across_chunks_do_not_alias() {
        // (0,0) and (CHUNK_SIDE,0) map to the same local index but different chunks.
        let a = Position::new(0, 0, 7);
        let b = Position::new(CHUNK_SIDE, 0, 7);
        let mut map = GameMap::new();
        map.insert_tile(a.clone(), MapTile::new());
        map.insert_tile(b.clone(), MapTile::new());

        let key = map.insert_agent(new_creature(), &a).unwrap();
        assert_eq!(
            map.iter_agents_at(&a).unwrap().copied().collect::<Vec<_>>(),
            vec![key]
        );
        // The tile in the neighbouring chunk exists but must be unaffected.
        assert_eq!(map.iter_agents_at(&b).unwrap().count(), 0);
    }

    #[test]
    fn move_agent_across_chunk_boundary() {
        let from = Position::new(CHUNK_SIDE - 1, 10, 7); // last column of chunk 0
        let to = Position::new(CHUNK_SIDE, 10, 7); // first column of chunk 1
        let mut map = GameMap::new();
        map.insert_tile(from.clone(), MapTile::new());
        map.insert_tile(to.clone(), MapTile::new());

        let key = map.insert_agent(new_creature(), &from).unwrap();
        map.move_agent(key, &to).unwrap();

        assert_eq!(map.agent_position(key), Some(&to));
        assert_eq!(map.iter_agents_at(&from).unwrap().count(), 0);
        assert_eq!(map.iter_agents_at(&to).unwrap().count(), 1);
    }

    #[test]
    fn mutation_after_clone_does_not_affect_snapshot() {
        // Validates the copy-on-write property: a published snapshot (a clone)
        // must not observe mutations made to the live map afterwards.
        let pos = Position::new(3, 3, 7);
        let mut map = map_with_one_tile(&pos);

        let snapshot = map.clone();
        map.insert_agent(new_creature(), &pos).unwrap();

        assert_eq!(snapshot.iter_agents_at(&pos).unwrap().count(), 0);
        assert_eq!(map.iter_agents_at(&pos).unwrap().count(), 1);
    }

    #[test]
    fn get_agents_at_rect_collects_across_chunks_and_excludes_outside() {
        let mut map = GameMap::new();
        let a = Position::new(2, 2, 7); // chunk (0, 0)
        let b = Position::new(18, 3, 7); // chunk (1, 0) — across a chunk boundary
        let outside = Position::new(40, 40, 7);
        for p in [&a, &b, &outside] {
            map.insert_tile(p.clone(), MapTile::new());
        }
        let ka = map.insert_agent(new_creature(), &a).unwrap();
        let kb = map.insert_agent(new_creature(), &b).unwrap();
        let ko = map.insert_agent(new_creature(), &outside).unwrap();

        let rect = Rect::new(0, 0, 20, 10);
        let found: Vec<_> = map.iter_agents_in_rect(&rect, 7).copied().collect();

        assert_eq!(found.len(), 2);
        assert!(found.contains(&ka));
        assert!(found.contains(&kb));
        assert!(!found.contains(&ko));
    }

    #[test]
    fn get_agents_at_rect_is_floor_scoped() {
        let mut map = GameMap::new();
        let pos = Position::new(5, 5, 7);
        map.insert_tile(pos.clone(), MapTile::new());
        map.insert_agent(new_creature(), &pos).unwrap();

        let rect = Rect::new(0, 0, 15, 15);
        assert_eq!(map.iter_agents_in_rect(&rect, 7).count(), 1);
        assert_eq!(map.iter_agents_in_rect(&rect, 6).count(), 0);
    }

    #[test]
    fn get_agents_at_rect_over_void_is_empty() {
        let map = GameMap::new();
        let rect = Rect::new(0, 0, 100, 100);
        assert_eq!(map.iter_agents_in_rect(&rect, 7).count(), 0);
    }

    #[test]
    fn iter_tiles_in_rect_yields_existing_tiles_with_positions() {
        let mut map = GameMap::new();
        let a = Position::new(2, 2, 7); // chunk (0, 0)
        let b = Position::new(18, 3, 7); // chunk (1, 0) — across a chunk boundary
        let outside = Position::new(40, 40, 7);
        for p in [&a, &b, &outside] {
            map.insert_tile(p.clone(), MapTile::new());
        }

        let rect = Rect::new(0, 0, 20, 10);
        let mut found: Vec<Position> = map
            .iter_tiles_in_rect(&rect, 7)
            .map(|(pos, _)| pos)
            .collect();
        found.sort();

        assert_eq!(found, vec![a, b]);
    }

    #[test]
    fn iter_tiles_in_rect_is_floor_scoped() {
        let mut map = GameMap::new();
        let pos = Position::new(5, 5, 7);
        map.insert_tile(pos.clone(), MapTile::new());

        let rect = Rect::new(0, 0, 15, 15);
        assert_eq!(map.iter_tiles_in_rect(&rect, 7).count(), 1);
        assert_eq!(map.iter_tiles_in_rect(&rect, 6).count(), 0);
    }

    #[test]
    fn get_agents_at_rect_respects_inclusive_bounds_within_a_chunk() {
        // Exercises the in-chunk clamp: max_x = 10 must include x=10 and exclude x=11.
        let mut map = GameMap::new();
        let inside = Position::new(10, 10, 7);
        let just_outside = Position::new(11, 10, 7);
        map.insert_tile(inside.clone(), MapTile::new());
        map.insert_tile(just_outside.clone(), MapTile::new());
        let ki = map.insert_agent(new_creature(), &inside).unwrap();
        map.insert_agent(new_creature(), &just_outside).unwrap();

        let rect = Rect::new(0, 0, 10, 10);
        let found: Vec<_> = map.iter_agents_in_rect(&rect, 7).copied().collect();
        assert_eq!(found, vec![ki]);
    }
}
