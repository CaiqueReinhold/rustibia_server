use crate::{
    entities::{
        agent::Agent,
        items::Item,
        map::GameMap,
        position::{ItemPlacement, Position},
    },
    game::map_query::{TileEntity, get_top_entity, item_at_placement},
};
use std::fmt::{Error, Write};

pub fn get_look_description(
    map: &GameMap,
    placement: &ItemPlacement,
    player_pos: &Position,
) -> String {
    let desc = match placement {
        // A tile names no item of its own, so a look there describes whatever is on top.
        ItemPlacement::Map(look_pos) => get_top_entity(map, look_pos).map(|entity| match entity {
            TileEntity::Agent(agent_key) => map
                .get_agent(agent_key)
                .map(get_agent_description)
                .unwrap_or(Ok("".to_owned())),
            TileEntity::Item(item) => get_item_description(item, player_pos.is_within(look_pos, 1)),
        }),
        ItemPlacement::Inventory(..) | ItemPlacement::Container { .. } => {
            item_at_placement(map, placement).map(|item| get_item_description(item, true))
        }
    };

    desc.and_then(|e| e.ok())
        .filter(|s| !String::is_empty(s))
        .unwrap_or("You see nothing".to_owned())
}

fn get_item_description(item: &Item, show_weight: bool) -> Result<String, Error> {
    let mut buff = String::new();
    write!(buff, "You see ")?;

    if item.amount == 1
        && let Some(article) = &item.config.article
    {
        write!(buff, "{} ", article)?;
    } else if item.amount > 1 {
        write!(buff, "{} ", item.amount)?;
    }

    write!(buff, "{}.", item.config.name)?;

    if let Some(desc) = &item.config.description {
        write!(buff, "\n{}", desc)?;
    }

    if show_weight {
        let weight = item.total_weight();
        if weight > 0 {
            write!(buff, "\nIt weights {:.2}oz.", (weight as f32) / 100.0)?;
        }
    }

    Ok(buff)
}

fn get_agent_description(agent: &Agent) -> Result<String, Error> {
    if agent.is_creature() {
        Ok(format!("You see a {}", agent.name()))
    } else {
        Ok(format!("You see {}", agent.name()))
    }
}
