use std::collections::{HashMap, HashSet, VecDeque};

use crate::types::{SemanticUnit, UnitType};

/// A local, in-memory collection of [`SemanticUnit`]s connected by references.
///
/// The graph is not a storage engine — it is a traversal structure. Load units
/// from wherever you store them, add them here, and use the query methods to
/// navigate relationships.
///
/// Units are indexed by `id`. Duplicate `id`s replace the earlier entry.
#[derive(Debug, Default)]
pub struct Graph {
    units: HashMap<String, SemanticUnit>,
}

impl Graph {
    /// Create an empty graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a graph from an iterator of units.
    pub fn from_units(iter: impl IntoIterator<Item = SemanticUnit>) -> Self {
        let mut g = Self::new();
        for u in iter {
            g.add(u);
        }
        g
    }

    /// Insert a unit. If a unit with the same `id` already exists, it is replaced.
    pub fn add(&mut self, unit: SemanticUnit) {
        self.units.insert(unit.id.clone(), unit);
    }

    /// Retrieve a unit by id.
    pub fn get(&self, id: &str) -> Option<&SemanticUnit> {
        self.units.get(id)
    }

    /// Total number of units in the graph.
    pub fn len(&self) -> usize {
        self.units.len()
    }

    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    /// Iterate over all units in unspecified order.
    pub fn units(&self) -> impl Iterator<Item = &SemanticUnit> {
        self.units.values()
    }

    /// All units of a given type.
    pub fn by_type(&self, unit_type: &UnitType) -> Vec<&SemanticUnit> {
        self.units
            .values()
            .filter(|u| &u.unit_type == unit_type)
            .collect()
    }

    /// The units that `id` directly references (outgoing edges).
    ///
    /// Units referenced but not present in this graph are silently omitted —
    /// the spec requires receivers not to reject units with unknown references.
    pub fn outgoing(&self, id: &str) -> Vec<&SemanticUnit> {
        let Some(unit) = self.units.get(id) else {
            return vec![];
        };
        let Some(refs) = &unit.references else {
            return vec![];
        };
        refs.iter()
            .filter_map(|r| self.units.get(&r.id))
            .collect()
    }

    /// The units that directly reference `id` (incoming edges).
    pub fn incoming(&self, id: &str) -> Vec<&SemanticUnit> {
        self.units
            .values()
            .filter(|u| {
                u.references
                    .as_ref()
                    .map_or(false, |refs| refs.iter().any(|r| r.id == id))
            })
            .collect()
    }

    /// All ancestors of `id` — units reachable by following outgoing edges
    /// recursively (i.e., what this unit is derived from, transitively).
    pub fn ancestors(&self, id: &str) -> Vec<&SemanticUnit> {
        self.bfs(id, Direction::Outgoing)
    }

    /// All descendants of `id` — units that reference `id`, transitively.
    pub fn descendants(&self, id: &str) -> Vec<&SemanticUnit> {
        self.bfs(id, Direction::Incoming)
    }

    /// The connected subgraph containing `id`: all ancestors, descendants,
    /// and the unit itself.
    pub fn subgraph(&self, id: &str) -> Graph {
        let mut seen: HashSet<&str> = HashSet::new();
        seen.insert(id);
        for u in self.ancestors(id) {
            seen.insert(&u.id);
        }
        for u in self.descendants(id) {
            seen.insert(&u.id);
        }
        Graph::from_units(
            seen.into_iter()
                .filter_map(|i| self.units.get(i))
                .cloned(),
        )
    }

    // BFS traversal in the given direction, excluding the start node.
    fn bfs(&self, start: &str, direction: Direction) -> Vec<&SemanticUnit> {
        let mut visited: HashSet<&str> = HashSet::new();
        let mut queue: VecDeque<&str> = VecDeque::new();
        let mut result: Vec<&SemanticUnit> = Vec::new();

        visited.insert(start);
        for neighbour in self.neighbours(start, direction) {
            if visited.insert(neighbour.id.as_str()) {
                queue.push_back(&neighbour.id);
                result.push(neighbour);
            }
        }

        while let Some(current) = queue.pop_front() {
            for neighbour in self.neighbours(current, direction) {
                if visited.insert(neighbour.id.as_str()) {
                    queue.push_back(&neighbour.id);
                    result.push(neighbour);
                }
            }
        }

        result
    }

    fn neighbours(&self, id: &str, direction: Direction) -> Vec<&SemanticUnit> {
        match direction {
            Direction::Outgoing => self.outgoing(id),
            Direction::Incoming => self.incoming(id),
        }
    }
}

#[derive(Clone, Copy)]
enum Direction {
    Outgoing,
    Incoming,
}

// --- tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Reference, RelType, UnitType};
    use std::collections::HashMap;

    fn unit(id: &str, unit_type: UnitType, refs: Vec<(&str, RelType)>) -> SemanticUnit {
        SemanticUnit {
            id: id.into(),
            unit_type,
            content: "test".into(),
            created_at: "2026-02-18T12:00:00Z".into(),
            author: "test-agent".into(),
            confidence: None,
            assumptions: None,
            source: None,
            references: if refs.is_empty() {
                None
            } else {
                Some(
                    refs.into_iter()
                        .map(|(id, rel)| Reference { id: id.into(), rel })
                        .collect(),
                )
            },
            extensions: HashMap::new(),
        }
    }

    #[test]
    fn add_and_get() {
        let mut g = Graph::new();
        let u = unit(
            "019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6c",
            UnitType::Assertion,
            vec![],
        );
        g.add(u.clone());
        assert_eq!(g.len(), 1);
        assert_eq!(
            g.get("019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6c").map(|u| u.id.as_str()),
            Some("019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6c")
        );
    }

    #[test]
    fn by_type() {
        let mut g = Graph::new();
        g.add(unit("019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6c", UnitType::Assertion, vec![]));
        g.add(unit("019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6d", UnitType::Inference, vec![]));
        assert_eq!(g.by_type(&UnitType::Assertion).len(), 1);
        assert_eq!(g.by_type(&UnitType::Question).len(), 0);
    }

    #[test]
    fn outgoing_and_incoming() {
        let id_a = "019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6c";
        let id_b = "019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6d";

        let mut g = Graph::new();
        g.add(unit(id_a, UnitType::Assertion, vec![]));
        g.add(unit(id_b, UnitType::Inference, vec![(id_a, RelType::DerivesFrom)]));

        assert_eq!(g.outgoing(id_b).len(), 1);
        assert_eq!(g.outgoing(id_b)[0].id, id_a);
        assert_eq!(g.incoming(id_a).len(), 1);
        assert_eq!(g.incoming(id_a)[0].id, id_b);
        assert_eq!(g.outgoing(id_a).len(), 0);
        assert_eq!(g.incoming(id_b).len(), 0);
    }

    #[test]
    fn ancestors_and_descendants() {
        let ids = [
            "019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6c",
            "019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6d",
            "019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6e",
        ];

        // chain: ids[2] -> ids[1] -> ids[0]
        let mut g = Graph::new();
        g.add(unit(ids[0], UnitType::Assertion, vec![]));
        g.add(unit(ids[1], UnitType::Inference, vec![(ids[0], RelType::DerivesFrom)]));
        g.add(unit(ids[2], UnitType::Inference, vec![(ids[1], RelType::DerivesFrom)]));

        let anc = g.ancestors(ids[2]);
        assert_eq!(anc.len(), 2);

        let desc = g.descendants(ids[0]);
        assert_eq!(desc.len(), 2);
    }

    #[test]
    fn subgraph() {
        let ids = [
            "019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6c",
            "019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6d",
            "019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6e",
            "019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6f", // disconnected
        ];

        let mut g = Graph::new();
        g.add(unit(ids[0], UnitType::Assertion, vec![]));
        g.add(unit(ids[1], UnitType::Inference, vec![(ids[0], RelType::DerivesFrom)]));
        g.add(unit(ids[2], UnitType::Challenge, vec![(ids[1], RelType::Rebuts)]));
        g.add(unit(ids[3], UnitType::Question, vec![]));

        let sg = g.subgraph(ids[1]);
        assert_eq!(sg.len(), 3); // ids[0], ids[1], ids[2] — not ids[3]
    }
}
