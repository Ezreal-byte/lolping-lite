use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

struct Entry<T> {
    value: T,
    bytes: usize,
    used: Instant,
}
pub struct Cache<T> {
    entries: HashMap<String, Entry<T>>,
    limit: usize,
    ttl: Duration,
}
impl<T> Cache<T> {
    pub fn new(limit: usize, ttl: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            limit,
            ttl,
        }
    }
    pub fn get(&mut self, key: &str, now: Instant) -> Option<&T> {
        self.trim(now);
        let e = self.entries.get_mut(key)?;
        e.used = now;
        Some(&e.value)
    }
    pub fn insert(&mut self, key: String, value: T, bytes: usize, now: Instant) {
        self.entries.remove(&key);
        self.trim(now);
        if bytes > self.limit {
            return;
        }
        while self.bytes() + bytes > self.limit {
            let Some(old) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            self.entries.remove(&old);
        }
        self.entries.insert(
            key,
            Entry {
                value,
                bytes,
                used: now,
            },
        );
    }
    pub fn trim(&mut self, now: Instant) {
        self.entries
            .retain(|_, e| now.duration_since(e.used) < self.ttl);
    }
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        self.entries.retain(|k, _| keep(k));
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn bytes(&self) -> usize {
        self.entries.values().map(|e| e.bytes).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_lru_expiry_and_invalidation() {
        let now = Instant::now();
        let mut c = Cache::new(10, Duration::from_secs(120));
        c.insert("a".into(), 1, 5, now);
        c.insert("b".into(), 2, 5, now + Duration::from_secs(1));
        assert_eq!(c.get("a", now + Duration::from_secs(2)), Some(&1));
        c.insert("c".into(), 3, 5, now + Duration::from_secs(3));
        assert!(c.get("b", now + Duration::from_secs(3)).is_none());
        assert_eq!(c.bytes(), 10);
        c.insert("huge".into(), 4, 11, now + Duration::from_secs(3));
        assert_eq!(c.bytes(), 10);
        c.retain(|k| k != "c");
        assert_eq!(c.bytes(), 5);
        c.trim(now + Duration::from_secs(123));
        assert_eq!(c.bytes(), 0);
    }
}
