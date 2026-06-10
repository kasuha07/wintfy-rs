use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct LruIds {
    max: usize,
    ids: VecDeque<String>,
}

impl LruIds {
    pub fn new(max: usize) -> Self {
        Self {
            max,
            ids: VecDeque::with_capacity(max.min(1024)),
        }
    }

    pub fn insert_new(&mut self, id: &str) -> bool {
        if id.is_empty() {
            return true;
        }
        if self.ids.iter().any(|existing| existing == id) {
            return false;
        }
        if self.ids.len() >= self.max {
            self.ids.pop_front();
        }
        self.ids.push_back(id.to_string());
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
