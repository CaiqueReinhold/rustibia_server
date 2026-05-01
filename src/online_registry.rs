use std::collections::HashSet;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct OnlineRegistry {
    inner: Arc<Mutex<HashSet<u32>>>,
}

pub struct RegistryGuard {
    inner: Arc<Mutex<HashSet<u32>>>,
    character_id: u32,
}

impl Default for OnlineRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl OnlineRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Returns Some(guard) if character_id was not already registered.
    /// Returns None if the character is already online.
    /// The returned guard removes the entry from the set on drop.
    pub fn try_register(&self, character_id: u32) -> Option<RegistryGuard> {
        let mut set = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if set.insert(character_id) {
            Some(RegistryGuard {
                inner: Arc::clone(&self.inner),
                character_id,
            })
        } else {
            None
        }
    }
}

impl Drop for RegistryGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = self.inner.lock() {
            set.remove(&self.character_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[test]
    fn test_registry_guard_is_send() {
        assert_send::<RegistryGuard>();
    }

    #[test]
    fn test_first_register_succeeds() {
        let registry = OnlineRegistry::new();
        assert!(registry.try_register(42).is_some());
    }

    #[test]
    fn test_duplicate_register_fails_while_guard_alive() {
        let registry = OnlineRegistry::new();
        let _guard = registry.try_register(42).unwrap();
        assert!(registry.try_register(42).is_none());
    }

    #[test]
    fn test_drop_releases_slot() {
        let registry = OnlineRegistry::new();
        {
            let _guard = registry.try_register(42).unwrap();
        } // guard dropped here
        assert!(registry.try_register(42).is_some());
    }

    #[test]
    fn test_different_characters_can_register_simultaneously() {
        let registry = OnlineRegistry::new();
        let _g1 = registry.try_register(1);
        let _g2 = registry.try_register(2);
        assert!(_g1.is_some());
        assert!(_g2.is_some());
    }
}
