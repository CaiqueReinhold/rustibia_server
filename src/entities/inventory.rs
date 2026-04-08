use crate::entities::items::{Item, ItemGuid};
use crate::entities::player::InventorySlot;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum InventoryError {
    #[error("Item does not exist in slot")]
    ItemNotInPosition,
    #[error("Container is full")]
    ContainerIsFull,
    #[error("Cannot equip this")]
    CannotEquip,
}

#[derive(Debug, Clone)]
pub struct Inventory {
    slots: HashMap<InventorySlot, Item>,
    pub carried_weight: u32,
}

impl Inventory {
    pub fn new() -> Self {
        Inventory {
            slots: HashMap::new(),
            carried_weight: 0,
        }
    }

    pub fn from_snapshot(slots: HashMap<InventorySlot, Item>) -> Self {
        let carried_weight = slots.values().map(|i| i.total_weight()).sum();
        Inventory {
            slots,
            carried_weight,
        }
    }

    /// Insert `item` into `slot`.
    ///
    /// - `container`: if `None`, replaces the slot item directly and returns the displaced item.
    /// - `container`: if `Some(guid)`, finds that container within the slot item and inserts there.
    ///
    /// Returns `Ok(Some(displaced))` when replacing the slot item directly,
    /// `Ok(None)` on container insert, or `Err` when the container is full/missing.
    pub fn insert(
        &mut self,
        slot: InventorySlot,
        container: Option<(&ItemGuid, usize)>,
        item: Item,
    ) -> Result<Option<Item>, InventoryError> {
        match container {
            None => {
                let weight_added = item.total_weight();
                let old = self.slots.insert(slot, item);
                if let Some(ref old_item) = old {
                    self.carried_weight -= old_item.total_weight();
                }
                self.carried_weight += weight_added;
                Ok(old)
            }
            Some((target_guid, container_pos)) => {
                let slot_item = self
                    .slots
                    .get_mut(&slot)
                    .ok_or(InventoryError::ItemNotInPosition)?;
                let container = slot_item
                    .find_by_guid_mut(target_guid)
                    .ok_or(InventoryError::ItemNotInPosition)?;
                let cap = container.container_capacity().unwrap();
                let content = container
                    .content
                    .as_mut()
                    .ok_or(InventoryError::ItemNotInPosition)?;
                if content.len() >= cap as usize {
                    return Err(InventoryError::ContainerIsFull);
                }
                self.carried_weight += item.total_weight();
                content.insert(container_pos, item);
                Ok(None)
            }
        }
    }

    // Find the first container having at least one free slot inside
    // the InventorySlot::Backpack slot
    pub fn first_available_container(&self) -> Option<&ItemGuid> {
        let backpack = self.slots.get(&InventorySlot::Backpack)?;
        find_available_container(backpack)
    }

    /// Remove item by `guid` from `slot` or from within a container inside that slot.
    /// Handles partial stack removal via `amount`.
    pub fn remove(
        &mut self,
        slot: InventorySlot,
        guid: &ItemGuid,
        amount: u8,
    ) -> Option<(Item, Option<(ItemGuid, usize)>)> {
        let slot_item = self.slots.get_mut(&slot)?;
        if slot_item.guid == *guid {
            if slot_item.amount > amount {
                slot_item.amount -= amount;
                let partial = Item {
                    guid: ItemGuid::new(),
                    config: slot_item.config.clone(),
                    item_id: slot_item.item_id,
                    amount,
                    content: None,
                };
                self.carried_weight -= partial.total_weight();
                return Some((partial, None));
            } else if slot_item.amount == amount {
                let removed = self.slots.remove(&slot).unwrap();
                self.carried_weight -= removed.total_weight();
                return Some((removed, None));
            }
            return None;
        }
        // Item is nested in a container within the slot
        if let Some(content) = &mut slot_item.content {
            let result = remove_from_container(&slot_item.guid.clone(), content, guid, amount);
            if let Some((ref removed, _)) = result {
                self.carried_weight -= removed.total_weight();
            }
            return result;
        }
        None
    }

    /// Remove whatever item is currently in `slot`, regardless of guid.
    /// Used when evicting the current slot occupant to make room for a new item.
    pub fn take_slot(&mut self, slot: &InventorySlot) -> Option<Item> {
        let item = self.slots.remove(slot)?;
        self.carried_weight -= item.total_weight();
        Some(item)
    }

    pub fn get(&self, slot: &InventorySlot) -> Option<&Item> {
        self.slots.get(slot)
    }

    pub fn get_mut(&mut self, slot: &InventorySlot) -> Option<&mut Item> {
        self.slots.get_mut(slot)
    }

    pub fn find_by_guid(&self, guid: &ItemGuid) -> Option<(&Item, InventorySlot)> {
        for (slot, item) in &self.slots {
            if let Some(found) = item.find_by_guid(guid) {
                return Some((found, *slot));
            }
        }
        None
    }

    pub fn keys(&self) -> impl Iterator<Item = &InventorySlot> {
        self.slots.keys()
    }

    pub fn iter(&self) -> impl Iterator<Item = (InventorySlot, &Item)> {
        self.slots.iter().map(|(k, v)| (*k, v))
    }

    pub fn into_slots(self) -> HashMap<InventorySlot, Item> {
        self.slots
    }
}

fn find_available_container(item: &Item) -> Option<&ItemGuid> {
    let available = item.available_capacity()?;
    if available > 0 {
        return Some(&item.guid);
    }
    for child in item.content.as_ref()? {
        if let Some(guid) = find_available_container(child) {
            return Some(guid);
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
            let found = remove_from_container(&item.guid, content, guid, amount);
            if found.is_some() {
                return found;
            }
        }
    }
    None
}
