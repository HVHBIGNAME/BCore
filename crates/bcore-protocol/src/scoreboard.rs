//! Минимальное детерминированное состояние scoreboard.
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Default)]
pub struct Scoreboard {
    pub objectives: BTreeSet<String>,
    pub scores: BTreeMap<(String, String), i32>,
    pub display: BTreeMap<String, String>,
}
impl Scoreboard {
    pub fn add_objective(&mut self, n: &str) {
        self.objectives.insert(n.to_string());
    }
    pub fn remove_objective(&mut self, n: &str) {
        self.objectives.remove(n);
        self.scores.retain(|(o, _), _| o != n);
    }
    pub fn set(&mut self, p: &str, o: &str, v: i32) {
        if self.objectives.contains(o) {
            self.scores.insert((o.to_string(), p.to_string()), v);
        }
    }
    pub fn get(&self, p: &str, o: &str) -> Option<i32> {
        self.scores.get(&(o.to_string(), p.to_string())).copied()
    }
    pub fn add(&mut self, p: &str, o: &str, v: i32) {
        let n = self.get(p, o).unwrap_or(0) + v;
        self.set(p, o, n);
    }
    pub fn remove(&mut self, p: &str, o: &str, v: i32) {
        self.add(p, o, -v);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn score_operations() {
        let mut s = Scoreboard::default();
        s.add_objective("kills");
        s.set("A", "kills", 2);
        s.add("A", "kills", 3);
        assert_eq!(s.get("A", "kills"), Some(5));
        s.remove_objective("kills");
        assert_eq!(s.get("A", "kills"), None);
    }
}
