use std::collections::HashMap;

use tracing::debug;

const GEN_BITS: u32 = 4;
const GEN_MASK: u16 = (1 << GEN_BITS) - 1;
const FIRST_INDEX: u16 = 1;
const LAST_INDEX: u16 = (u16::MAX >> GEN_BITS) - 1;

/// The wire-facing newtype a map hands out. Implementing it is what lets a map be typed
/// by the id it mints, so an agent id and a container id — both a `u16` underneath —
/// cannot be swapped at a call site.
pub trait LocalId: Copy + Eq {
    fn from_raw(raw: u16) -> Self;
    fn raw(self) -> u16;
}

#[derive(Debug)]
struct Slot<G> {
    generation: u16,
    occupant: Option<G>,
}

#[derive(Debug)]
/// Maps arbitrary global IDs (e.g. item GUIDs, agent keys) to small reusable
/// local IDs scoped to a session.
///
/// A local id is a 12-bit slot index and a 4-bit generation, and the two together are
/// what make a stale id **detectable** rather than silently valid. Freeing a slot bumps
/// its generation, so every id ever handed out for it stops resolving the moment the
/// client is told to forget it.
pub struct LocalIdMap<G, L> {
    global_to_local: HashMap<G, L>,
    slots: Vec<Slot<G>>,
    next_index: u16,
}

impl<G, L> LocalIdMap<G, L>
where
    G: Eq + std::hash::Hash + Clone,
    L: LocalId,
{
    pub fn new() -> Self {
        Self {
            global_to_local: HashMap::new(),
            slots: vec![Slot {
                generation: 0,
                occupant: None,
            }],
            next_index: FIRST_INDEX,
        }
    }

    pub fn get_or_insert(&mut self, global: G) -> L {
        if let Some(&local) = self.global_to_local.get(&global) {
            return local;
        }

        let index = self.claim_index();
        let slot = &mut self.slots[index as usize];
        slot.occupant = Some(global.clone());
        let local = L::from_raw(compose(index, slot.generation));
        self.global_to_local.insert(global, local);
        local
    }

    fn claim_index(&mut self) -> u16 {
        let capacity = (LAST_INDEX - FIRST_INDEX + 1) as usize;
        assert!(
            self.global_to_local.len() < capacity,
            "session local id space exhausted"
        );

        loop {
            let index = self.next_index;
            self.next_index = advance(self.next_index);

            if index as usize >= self.slots.len() {
                self.slots.resize_with(index as usize + 1, || Slot {
                    generation: 0,
                    occupant: None,
                });
                return index;
            }
            if self.slots[index as usize].occupant.is_none() {
                return index;
            }
        }
    }

    pub fn remove_by_local(&mut self, local: L) {
        let (index, generation) = split(local.raw());
        let Some(slot) = self.slots.get_mut(index as usize) else {
            return;
        };
        if slot.generation != generation {
            debug!("ignoring a free naming the stale local id {}", local.raw());
            return;
        }
        let Some(global) = slot.occupant.take() else {
            return;
        };

        self.global_to_local.remove(&global);
        slot.generation = slot.generation.wrapping_add(1) & GEN_MASK;
    }

    pub fn get_local(&self, global: &G) -> Option<L> {
        self.global_to_local.get(global).copied()
    }

    pub fn get_global(&self, local: L) -> Option<&G> {
        let (index, generation) = split(local.raw());
        let slot = self.slots.get(index as usize)?;
        if slot.generation != generation {
            debug!("refusing to resolve the stale local id {}", local.raw());
            return None;
        }
        slot.occupant.as_ref()
    }

    pub fn iter_global(&self) -> impl Iterator<Item = &G> {
        self.global_to_local.keys()
    }
}

fn compose(index: u16, generation: u16) -> u16 {
    (index << GEN_BITS) | (generation & GEN_MASK)
}

fn split(local: u16) -> (u16, u16) {
    (local >> GEN_BITS, local & GEN_MASK)
}

fn advance(index: u16) -> u16 {
    if index == LAST_INDEX {
        FIRST_INDEX
    } else {
        index + 1
    }
}

impl<G, L> Default for LocalIdMap<G, L>
where
    G: Eq + std::hash::Hash + Clone,
    L: LocalId,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Copy, Clone, Eq, PartialEq, Debug)]
    struct TestId(u16);

    impl LocalId for TestId {
        fn from_raw(raw: u16) -> Self {
            Self(raw)
        }

        fn raw(self) -> u16 {
            self.0
        }
    }

    fn a_map() -> LocalIdMap<&'static str, TestId> {
        LocalIdMap::new()
    }

    /// Standing in for the cursor completing a lap, which would otherwise take 4094
    /// allocations to reach.
    fn rewind_cursor<G, L>(map: &mut LocalIdMap<G, L>, index: u16) {
        map.next_index = index;
    }

    #[test]
    fn a_global_keeps_its_id_while_it_is_mapped() {
        let mut map = a_map();
        let first = map.get_or_insert("a");
        assert_eq!(map.get_or_insert("a"), first);
    }

    #[test]
    fn an_id_is_never_zero_and_never_the_absent_sentinel() {
        let mut map = a_map();
        let first = map.get_or_insert("a");
        assert_ne!(first.raw(), 0);

        rewind_cursor(&mut map, LAST_INDEX);
        let last = map.get_or_insert("b");
        assert_ne!(last.raw(), u16::MAX);
    }

    /// The point of the cursor: the id a client may still be holding must not come
    /// back on the next allocation.
    #[test]
    fn a_freed_id_is_not_the_next_one_handed_out() {
        let mut map = a_map();
        let freed = map.get_or_insert("a");
        map.remove_by_local(freed);
        assert_ne!(map.get_or_insert("b"), freed);
    }

    /// The point of the generation: when the cursor does come back around, the id the
    /// client was holding resolves to nothing rather than to whoever took the slot.
    #[test]
    fn an_id_whose_slot_was_reissued_no_longer_resolves() {
        let mut map = a_map();
        let stale = map.get_or_insert("a");
        map.remove_by_local(stale);

        rewind_cursor(&mut map, split(stale.raw()).0);
        let reissued = map.get_or_insert("b");

        assert_eq!(
            split(reissued.raw()).0,
            split(stale.raw()).0,
            "the test must actually reissue the same slot, or it proves nothing"
        );
        assert_ne!(reissued, stale, "the generation must distinguish the two");
        assert_eq!(map.get_global(stale), None);
        assert_eq!(map.get_global(reissued), Some(&"b"));
    }

    /// `handle_close_container` frees by an id the *client* chose, so a stale one must
    /// not evict whoever holds that slot now.
    #[test]
    fn a_free_naming_a_stale_id_leaves_the_current_occupant_alone() {
        let mut map = a_map();
        let stale = map.get_or_insert("a");
        map.remove_by_local(stale);

        rewind_cursor(&mut map, split(stale.raw()).0);
        let reissued = map.get_or_insert("b");

        map.remove_by_local(stale);
        assert_eq!(map.get_global(reissued), Some(&"b"));
    }

    /// Four bits, so detection covers fifteen reuses of a slot and the sixteenth mints
    /// the original id again. Pinned rather than assumed: it is the limit of what the
    /// tag can catch, and the cursor is what keeps a lap far enough away to matter.
    #[test]
    fn a_generation_wraps_after_sixteen_reuses_of_one_slot() {
        let mut map = a_map();
        let first = map.get_or_insert("a");
        let index = split(first.raw()).0;

        let mut latest = first;
        for _ in 0..16 {
            map.remove_by_local(latest);
            rewind_cursor(&mut map, index);
            latest = map.get_or_insert("a");
            assert_eq!(split(latest.raw()).0, index);
        }

        assert_eq!(latest, first, "the sixteenth reuse repeats the original id");
    }

    #[test]
    fn the_cursor_wraps_past_the_last_index_to_the_first() {
        let mut map = a_map();
        rewind_cursor(&mut map, LAST_INDEX);

        assert_eq!(split(map.get_or_insert("a").raw()).0, LAST_INDEX);
        assert_eq!(split(map.get_or_insert("b").raw()).0, FIRST_INDEX);
    }

    /// A lap brings the cursor back onto slots that never emptied, so it has to walk
    /// over them rather than hand one out twice.
    #[test]
    fn a_wrapped_cursor_steps_over_slots_still_in_use() {
        let mut map = a_map();
        let held = map.get_or_insert("a");
        assert_eq!(split(held.raw()).0, FIRST_INDEX);

        rewind_cursor(&mut map, LAST_INDEX);
        assert_eq!(split(map.get_or_insert("b").raw()).0, LAST_INDEX);
        assert_eq!(split(map.get_or_insert("c").raw()).0, FIRST_INDEX + 1);
        assert_eq!(map.get_global(held), Some(&"a"));
    }
}
