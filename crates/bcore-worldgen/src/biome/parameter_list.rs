// SPDX-License-Identifier: MIT
use super::{quantize, tree::Tree, BiomeId, BiomeParameters};
use std::cell::RefCell;
use std::sync::{Arc, Weak};

type Row = (BiomeId, BiomeParameters);

/// An immutable native climate parameter list and its search tree.
///
/// Each object owns a distinct cache identity, even for identical rows. Thread
/// histories persist across queries/chunks and never cross parameter-list objects.
/// Share this object (or an `Arc` of it) between workers instead of rebuilding it.
/// Thread bookkeeping is O(live lists used on that thread), with weak ownership
/// and cleanup on insertion. Live histories are never evicted by an LRU limit.
pub struct ParameterList {
    rows: Box<[Row]>,
    pub(super) tree: Tree,
    identity: Arc<()>,
}

impl ParameterList {
    /// A fresh list/cache identity using the pinned 26.1 JAR's exact integer rows
    /// and resource keys mapped to BCore's wire registry. No float round trip.
    pub fn overworld() -> Self {
        Self::new(super::canonical::rows().to_vec())
    }

    /// Builds the tree in native order. Panics for an empty parameter list.
    pub fn new(rows: Vec<Row>) -> Self {
        Self {
            tree: Tree::new(&rows),
            rows: rows.into_boxed_slice(),
            identity: Arc::new(()),
        }
    }

    pub fn values(&self) -> &[Row] {
        &self.rows
    }

    /// Finds a biome using the native per-thread last-result cache.
    pub fn find(&self, climate: [f64; 6]) -> BiomeId {
        self.find_biome(climate.map(quantize))
    }

    /// Creates an explicit cold stream bound to this list. Keep the sampler for
    /// the lifetime of the desired stream; creating one per chunk resets ties.
    pub fn sampler(&self) -> BiomeSampler<'_> {
        BiomeSampler {
            parameters: self,
            last: None,
        }
    }

    /// Explicitly starts a new native search history on the calling thread only.
    pub fn reset_thread_cache(&self) {
        LAST_RESULTS.with_borrow_mut(|entries| {
            entries.retain(|entry| entry.owner.as_ptr() != Arc::as_ptr(&self.identity));
        });
    }

    /// Resident row/tree storage, excluding allocator and per-thread bookkeeping.
    pub fn storage_bytes(&self) -> usize {
        std::mem::size_of_val(self.rows.as_ref()) + std::mem::size_of_val(self.tree.nodes.as_ref())
    }

    pub fn node_count(&self) -> usize {
        self.tree.nodes.len()
    }
}

struct LastResult {
    owner: Weak<()>,
    leaf: usize,
}

thread_local! {
    // Weak owners release dead lists; live entries must retain tie history.
    static LAST_RESULTS: RefCell<Vec<LastResult>> = const { RefCell::new(Vec::new()) };
    static SLICE_TREE: RefCell<Option<ParameterList>> = const { RefCell::new(None) };
}

/// Accepted sources for [`super::biome_at`].
///
/// [`ParameterList`] supplies exact native per-tree/per-thread history. A bare
/// slice has no stable lifetime identity, so its compatibility implementation
/// searches from a cold cache. It memoizes ONE tree per thread, validates every
/// row before reuse, and never carries a previous leaf between slice calls.
/// Use an owning list on generation hot paths to avoid that O(n) validation.
pub trait BiomeLookup {
    fn find_biome(&self, target: [i64; 6]) -> BiomeId;
}

impl BiomeLookup for ParameterList {
    fn find_biome(&self, target: [i64; 6]) -> BiomeId {
        LAST_RESULTS.with_borrow_mut(|entries| {
            let identity = Arc::as_ptr(&self.identity);
            if let Some(entry) = entries
                .iter_mut()
                .find(|entry| entry.owner.as_ptr() == identity)
            {
                entry.leaf = self.tree.search(target, Some(entry.leaf));
                return self.rows[self.tree.row(entry.leaf)].0;
            }
            entries.retain(|entry| entry.owner.strong_count() != 0);
            if entries.capacity() > 8 && entries.len() < entries.capacity() / 4 {
                entries.shrink_to(entries.len().max(8));
            }
            let leaf = self.tree.search(target, None);
            entries.push(LastResult {
                owner: Arc::downgrade(&self.identity),
                leaf,
            });
            self.rows[self.tree.row(leaf)].0
        })
    }
}

impl BiomeLookup for [Row] {
    fn find_biome(&self, target: [i64; 6]) -> BiomeId {
        SLICE_TREE.with_borrow_mut(|cached| {
            let parameters = cached.get_or_insert_with(|| ParameterList::new(self.to_vec()));
            if parameters.values() != self {
                *parameters = ParameterList::new(self.to_vec());
            }
            let leaf = parameters.tree.search(target, None);
            parameters.rows[parameters.tree.row(leaf)].0
        })
    }
}

impl BiomeLookup for Vec<Row> {
    fn find_biome(&self, target: [i64; 6]) -> BiomeId {
        self.as_slice().find_biome(target)
    }
}

impl<const N: usize> BiomeLookup for [Row; N] {
    fn find_biome(&self, target: [i64; 6]) -> BiomeId {
        self.as_slice().find_biome(target)
    }
}

impl<T: BiomeLookup + ?Sized> BiomeLookup for Arc<T> {
    fn find_biome(&self, target: [i64; 6]) -> BiomeId {
        self.as_ref().find_biome(target)
    }
}

impl<T: BiomeLookup + ?Sized> BiomeLookup for Box<T> {
    fn find_biome(&self, target: [i64; 6]) -> BiomeId {
        self.as_ref().find_biome(target)
    }
}

impl<T: BiomeLookup + ?Sized> BiomeLookup for &T {
    fn find_biome(&self, target: [i64; 6]) -> BiomeId {
        (*self).find_biome(target)
    }
}

/// A native last-result stream borrowing its exact parameter-list object.
/// It cannot accidentally warm-start another list with a row from this one.
pub struct BiomeSampler<'a> {
    parameters: &'a ParameterList,
    last: Option<usize>,
}

impl BiomeSampler<'_> {
    pub fn find(&mut self, climate: [f64; 6]) -> BiomeId {
        self.find_quantized(climate.map(quantize))
    }

    pub fn find_quantized(&mut self, target: [i64; 6]) -> BiomeId {
        let leaf = self.parameters.tree.search(target, self.last);
        self.last = Some(leaf);
        self.parameters.rows[self.parameters.tree.row(leaf)].0
    }

    pub fn reset(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biome::ClimateRange;

    fn rows() -> Vec<Row> {
        let point = ClimateRange { min: 0, max: 0 };
        let first = BiomeParameters {
            temperature: ClimateRange { min: -10, max: 0 },
            humidity: point,
            continentalness: point,
            erosion: point,
            depth: point,
            weirdness: point,
            offset: 0,
        };
        let mut second = first.clone();
        second.temperature = ClimateRange { min: 0, max: 10 };
        vec![(0, first), (1, second)]
    }

    #[test]
    fn live_contexts_survive_churn_and_dead_contexts_do_not_accumulate() {
        // No fixed-size LRU may discard these live, independently warmed trees.
        let lists: Vec<_> = (0..96).map(|_| ParameterList::new(rows())).collect();
        for (index, list) in lists.iter().enumerate() {
            let x = if index % 2 == 0 { -10 } else { 10 };
            assert_eq!(list.find_biome([x, 0, 0, 0, 0, 0]), (index % 2) as u32);
        }
        for _ in 0..1024 {
            ParameterList::new(rows()).find_biome([10, 0, 0, 0, 0, 0]);
        }
        for (index, list) in lists.iter().enumerate().rev() {
            assert_eq!(list.find_biome([0; 6]), (index % 2) as u32);
        }
        LAST_RESULTS.with_borrow(|entries| assert!(entries.len() <= lists.len() + 1));
        drop(lists);
        ParameterList::new(rows()).find_biome([0; 6]);
        LAST_RESULTS.with_borrow(|entries| {
            assert_eq!(entries.len(), 1);
            assert!(entries.capacity() <= 8);
        });
    }

    #[test]
    fn borrowed_rows_are_validated_after_in_place_mutation() {
        let mut values = rows();
        let address = values.as_ptr();
        assert_eq!(values.find_biome([10, 0, 0, 0, 0, 0]), 1);
        values[1].0 = 99;
        assert_eq!(values.find_biome([10, 0, 0, 0, 0, 0]), 99);
        values[1].1.temperature = ClimateRange { min: 30, max: 40 };
        assert_eq!(values.find_biome([10, 0, 0, 0, 0, 0]), 0);
        assert_eq!(values.as_ptr(), address);
    }
}
