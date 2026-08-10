use crate::{
    constants::{
        BASE_FLOOR, MAX_FLOOR, MAX_VISIBLE_ITEMS, MIN_FLOOR, PLAYER_VIEWPORT_HEIGHT,
        PLAYER_VIEWPORT_WIDTH, VIEWPORT_SIZE,
    },
    entities::{
        agent::{Agent, AgentKey},
        items::{ContainerId, Item, ItemGuid, ItemId, ItemRef},
        map::GameMap,
        player::InventorySlot,
        position::{Direction, ItemPlacement, Position, Rect},
    },
    local_id::LocalIdMap,
    messages::ItemStack,
};

pub fn iter_visible_floors(z: u8) -> impl Iterator<Item = u8> {
    let (min_z, max_z) = if z <= BASE_FLOOR {
        (MIN_FLOOR, BASE_FLOOR)
    } else {
        let min_z = if (z as i32) - 2 >= (BASE_FLOOR + 1) as i32 {
            z
        } else {
            BASE_FLOOR + 1
        };
        let max_z = if z + 2 <= MAX_FLOOR { z } else { MAX_FLOOR };
        (min_z, max_z)
    };
    min_z..=max_z
}

pub fn get_map_desc_on_viewport(
    map: &GameMap,
    viewport_center: &Position,
) -> Vec<(u8, Box<[ItemStack; VIEWPORT_SIZE]>)> {
    let half_w = (PLAYER_VIEWPORT_WIDTH / 2) as i32;
    let half_h = (PLAYER_VIEWPORT_HEIGHT / 2) as i32;

    let mut floors = Vec::new();
    for floor in iter_visible_floors(viewport_center.z) {
        let floor_offset = viewport_center.z as i32 - floor as i32;
        let cx = viewport_center.x as i32 + floor_offset;
        let cy = viewport_center.y as i32 + floor_offset;
        let x_start = (cx - half_w).max(0) as u16;
        let y_start = (cy - half_h).max(0) as u16;
        let x_end = (cx + half_w).max(0) as u16;
        let y_end = (cy + half_h).max(0) as u16;
        let rect = Rect::new(x_start, y_start, x_end, y_end);

        let mut tiles = Box::new([[None; MAX_VISIBLE_ITEMS]; VIEWPORT_SIZE]);
        let mut found_any = false;
        for (pos, tile) in map.iter_tiles_in_rect(&rect, floor) {
            let col = (pos.x - x_start) as usize;
            let row = (pos.y - y_start) as usize;
            if col >= PLAYER_VIEWPORT_WIDTH || row >= PLAYER_VIEWPORT_HEIGHT {
                continue;
            }
            let idx = row * PLAYER_VIEWPORT_WIDTH + col;
            if let Some(tile) = tile {
                for (j, item) in tile.visible_items().enumerate() {
                    found_any = true;
                    tiles[idx][j] = Some((item.item_id, item.amount));
                }
            }
        }
        if found_any {
            floors.push((floor, tiles));
        }
    }
    floors
}

fn expansion_rects(pos: &Position, direction: &Direction, floor: u8) -> (Rect, Option<Rect>) {
    let floor_offset = pos.z as i16 - floor as i16;
    let half_w = (PLAYER_VIEWPORT_WIDTH / 2) as i16;
    let half_h = (PLAYER_VIEWPORT_HEIGHT / 2) as i16;
    let x = pos.x as i16;
    let y = pos.y as i16;

    let x_start = (x - half_w + floor_offset).max(0) as u16;
    let x_end = (x + half_w + floor_offset) as u16;
    let y_start = (y - half_h + floor_offset).max(0) as u16;
    let y_end = (y + half_h + floor_offset) as u16;

    match direction {
        Direction::North => (Rect::new(x_start, y_start, x_end, y_start), None),
        Direction::South => (Rect::new(x_start, y_end, x_end, y_end), None),
        Direction::East => (Rect::new(x_end, y_start, x_end, y_end), None),
        Direction::West => (Rect::new(x_start, y_start, x_start, y_end), None),
        Direction::NorthEast => (
            Rect::new(x_start, y_start, x_end, y_start),
            Some(Rect::new(x_end, y_start + 1, x_end, y_end)),
        ),
        Direction::NorthWest => (
            Rect::new(x_start, y_start, x_end, y_start),
            Some(Rect::new(x_start, y_start + 1, x_start, y_end)),
        ),
        Direction::SouthEast => (
            Rect::new(x_start, y_end, x_end, y_end),
            Some(Rect::new(x_end, y_start, x_end, y_end.saturating_sub(1))),
        ),
        Direction::SouthWest => (
            Rect::new(x_start, y_end, x_end, y_end),
            Some(Rect::new(
                x_start,
                y_start,
                x_start,
                y_end.saturating_sub(1),
            )),
        ),
    }
}

pub fn get_map_expansion(
    map: &GameMap,
    viewport_center: &Position,
    direction: &Direction,
) -> Vec<(u8, Box<[ItemStack]>)> {
    let mut floors = Vec::new();
    for floor in iter_visible_floors(viewport_center.z) {
        let (rect1, rect2) = expansion_rects(viewport_center, direction, floor);
        let tiles = [Some(rect1), rect2]
            .into_iter()
            .flatten()
            .flat_map(move |rect| map.iter_tiles_in_rect(&rect, floor));

        let mut found_any = false;
        let mut parsed_tiles =
            Vec::with_capacity(PLAYER_VIEWPORT_WIDTH + PLAYER_VIEWPORT_HEIGHT - 1);
        for (_, tile) in tiles {
            let mut stack: ItemStack = [None; MAX_VISIBLE_ITEMS];
            if let Some(tile) = tile {
                for (i, item) in tile.visible_items().enumerate() {
                    found_any = true;
                    stack[i] = Some((item.item_id, item.amount));
                }
            }
            parsed_tiles.push(stack)
        }

        if found_any {
            floors.push((floor, parsed_tiles.into_boxed_slice()));
        }
    }
    floors
}

pub fn get_agents_in_viewport<'a>(
    map: &'a GameMap,
    position: &'a Position,
) -> impl Iterator<Item = (AgentKey, &'a Agent, Position)> + 'a {
    iter_visible_floors(position.z)
        .flat_map(|floor| map.iter_agents_in_rect(&Rect::player_viewport(position), floor))
        .flat_map(|key: &AgentKey| {
            map.get_agent(*key).map(|agent| {
                (
                    *key,
                    agent,
                    map.agent_position(*key).cloned().unwrap_or_default(),
                )
            })
        })
}

pub fn get_agents_in_expansion<'a>(
    map: &'a GameMap,
    position: &'a Position,
    direction: &'a Direction,
) -> impl Iterator<Item = (AgentKey, &'a Agent, Position)> + 'a {
    iter_visible_floors(position.z).flat_map(move |floor| {
        let (rect1, rect2) = expansion_rects(position, direction, floor);
        [Some(rect1), rect2]
            .into_iter()
            .flatten()
            .flat_map(move |rect| map.iter_agents_in_rect(&rect, floor))
            .copied()
            .flat_map(|key| {
                map.get_agent(key).map(|agent| {
                    (
                        key,
                        agent,
                        map.agent_position(key).cloned().unwrap_or_default(),
                    )
                })
            })
    })
}

pub fn get_tile(map: &GameMap, position: &Position) -> Box<ItemStack> {
    let mut stack: Box<ItemStack> = Box::new([None; MAX_VISIBLE_ITEMS]);
    if let Ok(items) = map.get_visible_items(position) {
        for (i, item) in items.enumerate() {
            stack[i] = Some((item.item_id, item.amount));
        }
    }
    stack
}

pub fn retrieve_item<'a>(
    map: &'a GameMap,
    position: &'a Position,
    item_id: ItemId,
    stack_index: u8,
    containers: &'a LocalIdMap<ItemGuid>,
    agent_key: AgentKey,
) -> Option<(&'a Item, ItemPlacement)> {
    if position.is_container_coord() {
        let container_id = position.y as ContainerId;
        let guid = containers.get_global(container_id)?;
        let (container, placement) = find_item_in_reach(map, guid, agent_key)?;
        let slot = position.z as usize;
        let item = container.content.as_ref()?.get(slot);
        item.filter(|it| it.item_id == item_id)
            .map(|item| (item, placement))
    } else if position.is_inventory_coord() {
        let player = map.get_player(agent_key)?;
        let slot = InventorySlot::from_id(position.y)?;
        player
            .inventory
            .get(&slot)
            .filter(|it| it.item_id == item_id)
            .map(|it| (it, ItemPlacement::Inventory(slot, agent_key)))
    } else {
        let item = map.get_item_at(position, stack_index as usize);
        item.filter(|it| it.item_id == item_id)
            .map(|item| (item, ItemPlacement::Map(position.clone())))
    }
}

fn iter_adjacent(pos: &Position) -> impl Iterator<Item = Position> {
    let x_start = pos.x.saturating_sub(1);
    let x_end = pos.x + 1;
    let y_start = pos.y.saturating_sub(1);
    let y_end = pos.y + 1;
    let z = pos.z;

    (y_start..=y_end).flat_map(move |y| (x_start..=x_end).map(move |x| Position { x, y, z }))
}

pub fn find_item_in_slot<'a>(
    agent: &'a Agent,
    slot: InventorySlot,
    guid: &'a ItemGuid,
) -> Option<&'a Item> {
    let player = agent.get_player()?;
    player.inventory.get(&slot)?.find_by_guid(guid)
}

pub fn find_item_in_reach<'a>(
    map: &'a GameMap,
    guid: &'a ItemGuid,
    agent_key: AgentKey,
) -> Option<(&'a Item, ItemPlacement)> {
    let player_pos = map.agent_position(agent_key)?;
    for pos in iter_adjacent(player_pos) {
        if let Some(item) = map.get_item_by_id(&pos, guid) {
            return Some((item, ItemPlacement::Map(pos)));
        }
    }

    let agent = map.get_agent(agent_key)?;
    let player = agent.get_player()?;
    for slot in player.inventory.keys() {
        if let Some(item) = find_item_in_slot(agent, *slot, guid) {
            return Some((item, ItemPlacement::Inventory(*slot, agent_key)));
        }
    }

    None
}

pub fn find_parent_container<'a>(
    map: &'a GameMap,
    guid: &'a ItemGuid,
    agent_key: AgentKey,
) -> Option<(&'a ItemGuid, ItemPlacement)> {
    let player_pos = map.agent_position(agent_key)?;
    for pos in iter_adjacent(player_pos) {
        if let Some(parent_guid) = map.get_parent_container(&pos, guid) {
            return Some((parent_guid, ItemPlacement::Map(pos)));
        }
    }
    None
}

pub fn find_item_in_placement<'a>(map: &'a GameMap, item_ref: &ItemRef) -> Option<&'a Item> {
    match &item_ref.placement {
        ItemPlacement::Map(item_pos) => map.get_item_by_id(item_pos, &item_ref.guid),
        ItemPlacement::Inventory(slot, inv_agent_key) => map
            .get_player(*inv_agent_key)
            .and_then(|player| player.inventory.get(slot))
            .map(|item| item.find_by_guid(&item_ref.guid))
            .unwrap_or(None),
    }
}

pub enum TileEntity<'a> {
    Item(&'a Item),
    Agent(AgentKey),
}

pub fn get_top_entity<'a>(map: &'a GameMap, pos: &'a Position) -> Option<TileEntity<'a>> {
    if let Ok(last_agent) = map
        .iter_agents_at(pos)
        .map(|mut agents_iter| agents_iter.next().cloned())
        && let Some(last_agent) = last_agent
    {
        return Some(TileEntity::Agent(last_agent));
    } else if let Some(item) = map.get_top_item(pos) {
        return Some(TileEntity::Item(item));
    }

    None
}
