use std::collections::HashMap;

/// Maps arbitrary global IDs (e.g. item GUIDs, agent keys) to small reusable
/// local IDs scoped to a session. Local IDs are u8 values recycled when freed.
pub struct LocalIdMap<G> {
    global_to_local: HashMap<G, u16>,
    local_to_global: HashMap<u16, G>,
    free_list: Vec<u16>,
    next_id: u16,
}

impl<G> LocalIdMap<G>
where
    G: Eq + std::hash::Hash + Clone,
{
    pub fn new() -> Self {
        Self {
            global_to_local: HashMap::new(),
            local_to_global: HashMap::new(),
            free_list: Vec::new(),
            next_id: 0,
        }
    }

    pub fn get_or_insert(&mut self, global: G) -> u16 {
        if let Some(&local) = self.global_to_local.get(&global) {
            return local;
        }

        let local = if let Some(recycled) = self.free_list.pop() {
            recycled
        } else {
            let id = self.next_id;
            self.next_id += 1;
            id
        };

        self.global_to_local.insert(global.clone(), local);
        self.local_to_global.insert(local, global);
        local
    }

    pub fn remove_by_global(&mut self, global: &G) {
        if let Some(local) = self.global_to_local.remove(global) {
            self.local_to_global.remove(&local);
            self.free_list.push(local);
        }
    }

    pub fn remove_by_local(&mut self, local: u16) {
        if let Some(global) = self.local_to_global.remove(&local) {
            self.global_to_local.remove(&global);
            self.free_list.push(local);
        }
    }

    pub fn get_local(&self, global: &G) -> Option<u16> {
        self.global_to_local.get(global).copied()
    }

    pub fn get_global(&self, local: u16) -> Option<&G> {
        self.local_to_global.get(&local)
    }

    pub fn iter_global(&self) -> impl Iterator<Item = &G> {
        self.global_to_local.keys()
    }
}

impl<G> Default for LocalIdMap<G>
where
    G: Eq + std::hash::Hash + Clone,
{
    fn default() -> Self {
        Self::new()
    }
}
