//! Vanilla placed-feature ordering and within-step indices.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::OnceLock;

/// The overworld biome source used by vanilla feature sorting.
/// The overworld biome source's possible biomes, in vanilla's parameter-list
/// order (first occurrence in the multi-noise biome source parameter list).
///
/// ORDER MATTERS: FeatureSorter assigns each placed feature a global
/// first-seen order while walking this list, and that order drives the
/// topological sort and the within-step indices. The list must therefore be
/// the parameter-list order (see SteelMC `OVERWORLD_BIOME_PARAMETERS`), NOT
/// alphabetical — sorting it alphabetically shifts every tree feature's index
/// (e.g. trees_birch_and_oak_leaf_litter 25 -> 43) and mis-seeds all trees.
pub const POSSIBLE_BIOMES: [&str; 54] = [
    "mushroom_fields",
    "deep_frozen_ocean",
    "frozen_ocean",
    "deep_cold_ocean",
    "cold_ocean",
    "deep_ocean",
    "ocean",
    "deep_lukewarm_ocean",
    "lukewarm_ocean",
    "warm_ocean",
    "stony_shore",
    "swamp",
    "mangrove_swamp",
    "snowy_slopes",
    "snowy_plains",
    "snowy_beach",
    "windswept_gravelly_hills",
    "grove",
    "windswept_hills",
    "snowy_taiga",
    "windswept_forest",
    "taiga",
    "plains",
    "meadow",
    "beach",
    "forest",
    "old_growth_spruce_taiga",
    "flower_forest",
    "birch_forest",
    "dark_forest",
    "pale_garden",
    "savanna_plateau",
    "savanna",
    "jungle",
    "badlands",
    "desert",
    "wooded_badlands",
    "jagged_peaks",
    "stony_peaks",
    "frozen_river",
    "river",
    "ice_spikes",
    "old_growth_pine_taiga",
    "sunflower_plains",
    "old_growth_birch_forest",
    "sparse_jungle",
    "bamboo_jungle",
    "eroded_badlands",
    "windswept_savanna",
    "cherry_grove",
    "frozen_peaks",
    "dripstone_caves",
    "lush_caves",
    "deep_dark",
];

type Vertex = (usize, usize, usize);

#[derive(Debug)]
pub struct FeatureSorter {
    steps: Vec<FeatureStep>,
    names: Vec<String>,
    /// biome -> per-step placed-feature names (stripped of `minecraft:`),
    /// backing the `feature_in_biome` membership check.
    biome_features: HashMap<String, Vec<Vec<String>>>,
}

#[derive(Debug)]
pub struct FeatureStep {
    features: Vec<usize>,
    indices: HashMap<usize, usize>,
    biome_indices: HashMap<String, Vec<usize>>,
}

impl FeatureSorter {
    fn build() -> Self {
        let biomes: HashMap<String, Vec<Vec<String>>> =
            serde_json::from_str(include_str!("../data/biome_features.json"))
                .expect("valid biome feature data");
        let names: Vec<String> = serde_json::from_str(include_str!("../data/placed_features.json"))
            .expect("valid placed feature data");
        let ids: HashMap<&str, usize> = names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.as_str(), i))
            .collect();
        let mut first_order = HashMap::new();
        let mut edges = BTreeMap::<Vertex, BTreeSet<Vertex>>::new();
        for biome in POSSIBLE_BIOMES {
            let Some(stages) = biomes.get(biome) else {
                continue;
            };
            let mut vertices = Vec::new();
            for (step, stage) in stages.iter().enumerate() {
                for raw in stage {
                    let key = raw.rsplit(':').next().unwrap_or(raw);
                    let Some(&id) = ids.get(key) else {
                        panic!("unknown placed feature {raw}")
                    };
                    let order = if let Some(&n) = first_order.get(&id) {
                        n
                    } else {
                        let n = first_order.len();
                        first_order.insert(id, n);
                        n
                    };
                    let v = (step, order, id);
                    edges.entry(v).or_default();
                    vertices.push(v);
                }
            }
            for pair in vertices.windows(2) {
                edges.entry(pair[0]).or_default().insert(pair[1]);
            }
        }
        let mut out = Vec::with_capacity(edges.len());
        let mut done = BTreeSet::new();
        let mut active = BTreeSet::new();
        fn visit(
            v: Vertex,
            edges: &BTreeMap<Vertex, BTreeSet<Vertex>>,
            done: &mut BTreeSet<Vertex>,
            active: &mut BTreeSet<Vertex>,
            out: &mut Vec<Vertex>,
        ) {
            if done.contains(&v) {
                return;
            }
            assert!(active.insert(v), "cycle in feature graph");
            for &next in &edges[&v] {
                visit(next, edges, done, active, out);
            }
            active.remove(&v);
            done.insert(v);
            out.push(v);
        }
        for &v in edges.keys() {
            visit(v, &edges, &mut done, &mut active, &mut out);
        }
        out.reverse();
        let max_step = out.iter().map(|v| v.0).max().unwrap_or(0);
        let mut steps: Vec<FeatureStep> = (0..=max_step)
            .map(|_| FeatureStep {
                features: Vec::new(),
                indices: HashMap::new(),
                biome_indices: HashMap::new(),
            })
            .collect();
        for &(step, _, id) in &out {
            let index = steps[step].features.len();
            steps[step].features.push(id);
            steps[step].indices.insert(id, index);
        }
        let mut biome_features = HashMap::new();
        for biome in POSSIBLE_BIOMES {
            let Some(stages) = biomes.get(biome) else {
                continue;
            };
            for (step, stage) in stages.iter().enumerate() {
                let mut ids_at_step = stage
                    .iter()
                    .filter_map(|raw| {
                        ids.get(raw.rsplit(':').next().unwrap_or(raw))
                            .and_then(|id| steps.get(step)?.indices.get(id).copied())
                    })
                    .collect::<Vec<_>>();
                ids_at_step.sort_unstable();
                ids_at_step.dedup();
                if let Some(data) = steps.get_mut(step) {
                    if !ids_at_step.is_empty() {
                        data.biome_indices.insert(biome.to_owned(), ids_at_step);
                    }
                }
            }
            // Membership check uses feature NAMES, not the within-step indices
            // above (which index a different, per-step space).
            biome_features.insert(
                biome.to_owned(),
                stages
                    .iter()
                    .map(|stage| {
                        stage
                            .iter()
                            .map(|raw| raw.rsplit(':').next().unwrap_or(raw).to_owned())
                            .collect()
                    })
                    .collect(),
            );
        }
        Self {
            steps,
            names,
            biome_features,
        }
    }

    pub fn step_count(&self) -> usize {
        self.steps.len()
    }
    pub fn step(&self, step: usize) -> Option<&FeatureStep> {
        self.steps.get(step)
    }
    pub fn within_step_index(&self, feature: &str) -> Option<(usize, usize)> {
        let key = feature.rsplit(':').next().unwrap_or(feature);
        self.steps.iter().enumerate().find_map(|(step, data)| {
            self.names
                .iter()
                .position(|n| n == key)
                .and_then(|id| data.indices.get(&id).copied().map(|i| (step, i)))
        })
    }
    pub fn feature_in_biome(&self, biome: &str, feature: &str) -> bool {
        let key = feature.rsplit(':').next().unwrap_or(feature);
        self.biome_features.get(biome).is_some_and(|stages| {
            stages
                .iter()
                .any(|stage| stage.iter().any(|name| name == key))
        })
    }
    pub fn feature_name(&self, step: usize, index: usize) -> Option<&str> {
        self.steps
            .get(step)?
            .features
            .get(index)
            .map(|&id| self.names[id].as_str())
    }
    pub fn indices_for_biome(&self, step: usize, biome: &str) -> &[usize] {
        self.steps
            .get(step)
            .and_then(|s| s.biome_indices.get(biome))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

pub fn sorter() -> &'static FeatureSorter {
    static SORTER: OnceLock<FeatureSorter> = OnceLock::new();
    SORTER.get_or_init(FeatureSorter::build)
}

#[cfg(test)]
mod tests {
    use super::sorter;
    #[test]
    fn within_step_indices_match_vanilla_anchors() {
        let s = sorter();
        for (name, expected) in [
            ("ore_copper", (6, 25)),
            ("trees_plains", (9, 52)),
            ("trees_taiga", (9, 43)),
            ("trees_birch", (9, 24)),
            ("ore_copper_large", (6, 24)),
            ("trees_birch_and_oak_leaf_litter", (9, 25)),
        ] {
            assert_eq!(s.within_step_index(name), Some(expected), "{name}");
        }
    }

    #[test]
    fn feature_in_biome_checks_names_not_indices() {
        let s = sorter();
        // Positive membership: these are real entries in each biome's vegetal
        // stage, so the gate must admit them (regression for the bug where
        // within-step indices were read as global feature-name indices).
        assert!(s.feature_in_biome("plains", "trees_plains"));
        assert!(s.feature_in_biome("taiga", "trees_taiga"));
        assert!(s.feature_in_biome("forest", "trees_birch_and_oak_leaf_litter"));
        // Negative membership: a biome must not admit another biome's feature.
        assert!(!s.feature_in_biome("plains", "trees_jungle"));
        assert!(!s.feature_in_biome("taiga", "trees_plains"));
    }
}
