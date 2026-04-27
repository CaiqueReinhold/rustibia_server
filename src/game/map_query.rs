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
        position::{Direction, ItemPlacement, Position},
    },
    local_id::LocalIdMap,
    messages::ItemStack,
};

pub fn iter_visible_floors(position: &Position) -> impl Iterator<Item = u8> {
    let (min_z, max_z) = if position.z <= 7 {
        (MIN_FLOOR, BASE_FLOOR)
    } else {
        let min_z = if (position.z as i32) - 2 >= (BASE_FLOOR + 1) as i32 {
            position.z
        } else {
            BASE_FLOOR + 1
        };
        let max_z = if position.z + 2 <= MAX_FLOOR {
            position.z
        } else {
            MAX_FLOOR
        };
        (min_z, max_z)
    };
    min_z..=max_z
}

pub fn iter_viewport(pos: &Position, floor: u8) -> impl Iterator<Item = Position> {
    let floor_offset = pos.z as i16 - floor as i16;
    let half_w = (PLAYER_VIEWPORT_WIDTH / 2) as i16;
    let half_h = (PLAYER_VIEWPORT_HEIGHT / 2) as i16;
    let x = pos.x as i16;
    let y = pos.y as i16;

    let x_start = (x - half_w + floor_offset).max(0) as u16;
    let x_end = (x + half_w + floor_offset) as u16;
    let y_start = (y - half_h + floor_offset).max(0) as u16;
    let y_end = (y + half_h + floor_offset) as u16;
    let z = floor;

    (y_start..=y_end).flat_map(move |y| (x_start..=x_end).map(move |x| Position::new(x, y, z)))
}

pub fn get_map_desc_on_viewport(
    map: &GameMap,
    viewport_center: &Position,
) -> Vec<(u8, Box<[ItemStack; VIEWPORT_SIZE]>)> {
    let mut floors = Vec::new();
    for floor in iter_visible_floors(viewport_center) {
        let mut tiles = [[None; MAX_VISIBLE_ITEMS]; VIEWPORT_SIZE];
        let mut found_any = false;
        for (i, pos) in iter_viewport(viewport_center, floor).enumerate() {
            let items = map.get_visible_items(&pos);
            if let Ok(items) = items {
                for (j, item) in items.enumerate() {
                    found_any = true;
                    tiles[i][j] = Some((item.item_id, item.amount));
                }
            }
        }
        if found_any {
            floors.push((floor, tiles.into()));
        }
    }
    floors
}

pub fn get_map_expansion(
    map: &GameMap,
    viewport_center: &Position,
    direction: &Direction,
) -> Vec<(u8, Box<[ItemStack]>)> {
    let mut floors = Vec::new();
    for floor in iter_visible_floors(viewport_center) {
        let tiles = iter_expansion(viewport_center, direction, floor)
            .map(|pos| {
                let mut stack: ItemStack = [None; MAX_VISIBLE_ITEMS];
                if let Ok(items) = map.get_visible_items(&pos) {
                    for (i, item) in items.enumerate() {
                        stack[i] = Some((item.item_id, item.amount));
                    }
                }
                stack
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        if tiles.iter().any(|t| t[0].is_some()) {
            floors.push((floor, tiles));
        }
    }
    floors
}

fn iter_expansion(
    pos: &Position,
    direction: &Direction,
    floor: u8,
) -> Box<dyn Iterator<Item = Position>> {
    let floor_offset = pos.z as i16 - floor as i16;
    let half_w = (PLAYER_VIEWPORT_WIDTH / 2) as i16;
    let half_h = (PLAYER_VIEWPORT_HEIGHT / 2) as i16;
    let x = pos.x as i16;
    let y = pos.y as i16;
    let z = floor;

    let x_start = (x - half_w + floor_offset).max(0) as u16;
    let x_end = (x + half_w + floor_offset) as u16;
    let y_start = (y - half_h + floor_offset).max(0) as u16;
    let y_end = (y + half_h + floor_offset) as u16;

    let top_row = {
        (x_start..=x_end).map(move |xi| Position {
            x: xi,
            y: y_start,
            z,
        })
    };
    let bottom_row = (x_start..=x_end).map(move |xi| Position { x: xi, y: y_end, z });
    let left_col = {
        (y_start..=y_end).map(move |yi| Position {
            x: x_start,
            y: yi,
            z,
        })
    };
    let right_col = (y_start..=y_end).map(move |yi| Position { x: x_end, y: yi, z });

    match direction {
        Direction::North => Box::new(top_row),
        Direction::South => Box::new(bottom_row),
        Direction::East => Box::new(right_col),
        Direction::West => Box::new(left_col),
        // For diagonals: full edge row + edge column excluding the shared corner
        Direction::NorthEast => Box::new(top_row.chain(right_col.skip(1))),
        Direction::NorthWest => Box::new(top_row.chain(left_col.skip(1))),
        Direction::SouthEast => {
            Box::new(bottom_row.chain(right_col.take(((y_end - y_start) - 1) as usize)))
        }
        Direction::SouthWest => {
            Box::new(bottom_row.chain(left_col.take(((y_end - y_start) - 1) as usize)))
        }
    }
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
        .get_agents_at(pos)
        .map(|mut agents_iter| agents_iter.next().cloned())
        && let Some(last_agent) = last_agent
    {
        return Some(TileEntity::Agent(last_agent));
    } else if let Some(item) = map.get_top_item(pos) {
        return Some(TileEntity::Item(item));
    }

    None
}
