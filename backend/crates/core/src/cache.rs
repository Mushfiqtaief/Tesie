use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};

use crate::models::ResponseCacheRecord;

/// In-Memory L1 LRU Cache for sub-millisecond hot hits
#[derive(Clone)]
pub struct MemoryCache {
    inner: Arc<RwLock<MemoryCacheInner>>,
}

struct MemoryCacheInner {
    entries: HashMap<String, ResponseCacheRecord>,
    order: VecDeque<String>,
    max_entries: usize,
}

impl Default for MemoryCache {
    fn default() -> Self {
        Self::new(500)
    }
}

impl MemoryCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(MemoryCacheInner {
                entries: HashMap::new(),
                order: VecDeque::new(),
                max_entries,
            })),
        }
    }

    pub fn get(&self, key: &str, now_ms: i64) -> Option<ResponseCacheRecord> {
        let mut guard = self.inner.write().ok()?;
        if let Some(record) = guard.entries.get(key) {
            if record.expires_at <= now_ms {
                guard.entries.remove(key);
                return None;
            }
            let rec = record.clone();
            // Move to back of LRU queue
            if let Some(pos) = guard.order.iter().position(|k| k == key) {
                guard.order.remove(pos);
            }
            guard.order.push_back(key.to_string());
            Some(rec)
        } else {
            None
        }
    }

    pub fn put(&self, record: ResponseCacheRecord) {
        if let Ok(mut guard) = self.inner.write() {
            let key = record.cache_key.clone();
            if guard.entries.contains_key(&key) {
                guard.entries.insert(key.clone(), record);
                if let Some(pos) = guard.order.iter().position(|k| k == &key) {
                    guard.order.remove(pos);
                }
                guard.order.push_back(key);
            } else {
                if guard.entries.len() >= guard.max_entries {
                    if let Some(oldest) = guard.order.pop_front() {
                        guard.entries.remove(&oldest);
                    }
                }
                guard.order.push_back(key.clone());
                guard.entries.insert(key, record);
            }
        }
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.inner.write() {
            guard.entries.clear();
            guard.order.clear();
        }
    }

    pub fn count(&self) -> usize {
        self.inner.read().map(|g| g.entries.len()).unwrap_or(0)
    }
}
