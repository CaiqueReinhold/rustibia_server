use crate::{
    constants::{MAX_VISIBLE_ITEMS, PLAYER_VIEWPORT_HEIGHT, PLAYER_VIEWPORT_WIDTH, VIEWPORT_SIZE},
    entities::{
        agent::AgentKey,
        items::{Item, ItemGuid, ItemId},
        map::GameMap,
        position::{Direction, Position},
    },
    messages::ItemStack,
};

pub fn get_map_desc_on_viewport(
    map: &GameMap,
    viewport_center: &Position,
) -> Box<[ItemStack; VIEWPORT_SIZE]> {
    let mut tiles: [[Option<(u16, u8)>; MAX_VISIBLE_ITEMS]; VIEWPORT_SIZE] =
        [[None; MAX_VISIBLE_ITEMS]; VIEWPORT_SIZE];
    for (i, pos) in iter_viewport(viewport_center).enumerate() {
        let items = map.get_visible_items(&pos);
        if let Ok(items) = items {
            for (j, item) in items.enumerate() {
                tiles[i][j] = Some((item.item_id, item.amount));
            }
        }
    }
    Box::new(tiles)
}

pub fn iter_viewport(pos: &Position) -> impl Iterator<Item = Position> {
    let half_w = (PLAYER_VIEWPORT_WIDTH / 2) as u32;
    let half_h = (PLAYER_VIEWPORT_HEIGHT / 2) as u32;

    let x_start = pos.x.saturating_sub(half_w);
    let x_end = pos.x + half_w;
    let y_start = pos.y.saturating_sub(half_h);
    let y_end = pos.y + half_h;
    let z = pos.z;

    (y_start..=y_end).flat_map(move |y| (x_start..=x_end).map(move |x| Position { x, y, z }))
}

pub fn get_map_expansion(
    map: &GameMap,
    viewport_center: &Position,
    direction: &Direction,
) -> Box<[ItemStack]> {
    iter_expansion(viewport_center, direction)
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
        .into_boxed_slice()
}

fn iter_expansion(pos: &Position, direction: &Direction) -> Box<dyn Iterator<Item = Position>> {
    let half_w = (PLAYER_VIEWPORT_WIDTH / 2) as u32;
    let half_h = (PLAYER_VIEWPORT_HEIGHT / 2) as u32;
    let x = pos.x;
    let y = pos.y;
    let z = pos.z;

    let x_start = x.saturating_sub(half_w);
    let x_end = x + half_w;
    let y_start = y.saturating_sub(half_h);
    let y_end = y + half_h;

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
    stack_index: u16,
) -> Option<&'a Item> {
    if position.is_container_coord() || position.is_inventory_coord() {
        // validate is take
        // validate item exists in the position
        None
    } else {
        let item = map.get_item_at(position, stack_index as usize);
        item.filter(|it| it.item_id == item_id)
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

pub fn find_item_in_reach<'a>(
    map: &'a GameMap,
    guid: &'a ItemGuid,
    agent_key: AgentKey,
) -> Option<&'a Item> {
    let player_pos = map.agent_position(agent_key)?;
    for pos in iter_adjacent(player_pos) {
        if let Some(item) = map.get_item_by_id(&pos, guid) {
            return Some(item);
        }
    }

    // TODO: check for player inventory

    None
}
