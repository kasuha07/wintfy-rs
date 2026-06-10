use std::collections::{HashSet, VecDeque};

#[derive(Debug, Clone)]
pub struct LruIds {
    max: usize,
    ids: VecDeque<String>,
    seen: HashSet<String>,
}

impl LruIds {
    pub fn new(max: usize) -> Self {
        Self {
            max,
            ids: VecDeque::with_capacity(max.min(1024)),
            seen: HashSet::with_capacity(max.min(1024)),
        }
    }

    pub fn insert_new(&mut self, id: &str) -> bool {
        if id.is_empty() || self.max == 0 {
            return true;
        }
        if self.seen.contains(id) {
            return false;
        }
        if self.ids.len() >= self.max
            && let Some(old) = self.ids.pop_front()
        {
            self.seen.remove(&old);
        }
        let id = id.to_string();
        self.seen.insert(id.clone());
        self.ids.push_back(id);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_duplicates_and_evicts() {
        let mut ids = LruIds::new(2);
        assert!(ids.insert_new("a"));
        assert!(!ids.insert_new("a"));
        assert!(ids.insert_new("b"));
        assert!(ids.insert_new("c"));
        assert!(ids.insert_new("a"));
    }
}
