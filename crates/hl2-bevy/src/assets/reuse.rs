//! Bounded content-verified reuse. Paths alone never identify an asset across game/pak overrides.
use std::{
    collections::{BTreeMap, HashMap},
    hash::{DefaultHasher, Hash, Hasher},
    sync::{Arc, Mutex},
};

#[derive(Clone, Hash, PartialEq, Eq)]
struct Key {
    path: String,
    mode: u8,
    hash: u64,
}
struct Entry<T> {
    encoded: Arc<[u8]>,
    value: Arc<T>,
    weight: usize,
    stamp: u64,
}
struct State<T> {
    entries: HashMap<Key, Entry<T>>,
    order: BTreeMap<u64, Key>,
    bytes: usize,
    clock: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
}
pub(crate) struct Reuse<T> {
    limit: usize,
    state: Mutex<State<T>>,
}
impl<T> Reuse<T> {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            state: Mutex::new(State {
                entries: HashMap::new(),
                order: BTreeMap::new(),
                bytes: 0,
                clock: 0,
                hits: 0,
                misses: 0,
                evictions: 0,
            }),
        }
    }
    fn key(path: &str, mode: u8, encoded: &[u8]) -> Key {
        let mut hash = DefaultHasher::new();
        encoded.hash(&mut hash);
        Key {
            path: path.to_owned(),
            mode,
            hash: hash.finish(),
        }
    }
    pub(crate) fn get(&self, path: &str, mode: u8, encoded: &[u8]) -> Option<Arc<T>> {
        let key = Self::key(path, mode, encoded);
        let mut state = self.state.lock().expect("reuse cache");
        state.clock += 1;
        let stamp = state.clock;
        let found = state
            .entries
            .get_mut(&key)
            .filter(|entry| entry.encoded.as_ref() == encoded)
            .map(|entry| {
                let old = entry.stamp;
                entry.stamp = stamp;
                (old, entry.value.clone())
            });
        if let Some((old, value)) = found {
            state.order.remove(&old);
            state.order.insert(stamp, key);
            state.hits += 1;
            Some(value)
        } else {
            state.misses += 1;
            None
        }
    }
    pub(crate) fn insert(
        &self,
        path: &str,
        mode: u8,
        encoded: Arc<[u8]>,
        value: Arc<T>,
        decoded_bytes: usize,
    ) {
        let weight = encoded
            .len()
            .saturating_add(decoded_bytes)
            .saturating_add(path.len())
            .saturating_add(std::mem::size_of::<Entry<T>>());
        if weight > self.limit {
            return;
        }
        let key = Self::key(path, mode, &encoded);
        let mut state = self.state.lock().expect("reuse cache");
        if let Some(old) = state.entries.remove(&key) {
            state.bytes -= old.weight;
            state.order.remove(&old.stamp);
        }
        while state.bytes > self.limit - weight {
            let Some((_, oldest)) = state.order.pop_first() else {
                break;
            };
            if let Some(old) = state.entries.remove(&oldest) {
                state.bytes -= old.weight;
                state.evictions += 1;
            }
        }
        state.clock += 1;
        let stamp = state.clock;
        state.bytes += weight;
        state.order.insert(stamp, key.clone());
        state.entries.insert(
            key,
            Entry {
                encoded,
                value,
                weight,
                stamp,
            },
        );
    }
    pub(crate) fn report(&self) -> serde_json::Value {
        let state = self.state.lock().expect("reuse cache");
        serde_json::json!({"hits":state.hits,"misses":state.misses,"evictions":state.evictions,"bytes":state.bytes,"budget":self.limit,"entries":state.entries.len()})
    }
}

#[cfg(test)]
mod tests {
    use super::Reuse;
    use std::sync::Arc;
    #[test]
    fn reuse_requires_exact_resolved_content_and_conversion_policy() {
        let cache = Reuse::new(1024);
        let value = Arc::new(7);
        cache.insert("shared.wav", 0, Arc::from(&b"base"[..]), value.clone(), 8);
        assert!(Arc::ptr_eq(
            &cache.get("shared.wav", 0, b"base").unwrap(),
            &value
        ));
        assert!(cache.get("shared.wav", 0, b"pak override").is_none());
        assert!(cache.get("shared.wav", 1, b"base").is_none());
        assert!(cache.get("other.wav", 0, b"base").is_none());
        // Even a hash collision must compare the stored bytes.
        let key = Reuse::<i32>::key("shared.wav", 0, b"base");
        cache
            .state
            .lock()
            .unwrap()
            .entries
            .get_mut(&key)
            .unwrap()
            .encoded = Arc::from(&b"collision"[..]);
        assert!(cache.get("shared.wav", 0, b"base").is_none());
    }
    #[test]
    fn reuse_budget_evicts_least_recently_used_and_keeps_live_users_valid() {
        let cache = Reuse::new(400);
        let first = Arc::new(1);
        for (name, value) in [("a", first.clone()), ("b", Arc::new(2))] {
            cache.insert(name, 0, Arc::from(&b"bytes"[..]), value, 120);
        }
        assert_eq!(*cache.get("a", 0, b"bytes").unwrap(), 1);
        cache.insert("c", 0, Arc::from(&b"bytes"[..]), Arc::new(3), 120);
        assert!(cache.get("b", 0, b"bytes").is_none());
        assert_eq!(*first, 1);
        assert!(cache.report()["bytes"].as_u64().unwrap() <= 400);
        cache.insert("huge", 0, Arc::from(&b"bytes"[..]), Arc::new(4), 401);
        assert!(cache.get("huge", 0, b"bytes").is_none());
    }
}
