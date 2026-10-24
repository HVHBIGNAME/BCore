//! Ordered Minecraft Java loot generation and container filling.
//!
//! Tables, pools, entries, providers and functions are compiled once. Runtime
//! evaluation uses the table graph and native-order registries, independently of
//! the test fixtures. Item defaults and unchanged patches are shared by `Arc`.
//! The built-in catalog uses 26.2 runtime exports. Historical native fixtures use
//! [`CatalogSources::VANILLA_26_1`] explicitly.
//!
//! ```
//! use bcore_worldgen::loot::LootEngine;
//! let engine = LootEngine::vanilla()?;
//! let slots = engine.fill_chest("minecraft:chests/simple_dungeon", 42)?;
//! assert_eq!(slots.len(), 27);
//! # Ok::<(), bcore_worldgen::loot::LootError>(())
//! ```
//!
//! Explicit nonzero seeds use [`LegacyRandom`]. Seed zero requires an external
//! server-sequence/level-RNG owner: it is rejected by seed-taking APIs. The
//! caller-RNG API neither seeds nor selects a sequence, including in nested tables.
//! Native coverage includes null-player, positive-zero luck, built-in enabled
//! item features and biome-only location checks with caller-supplied world data.
//! Unsupported functions/conditions/codecs,
//! world-dependent maps and recursive graphs return explicit errors.

mod enchant;
mod evaluate;
mod fill;
mod function;
mod item;
mod json;
mod location;
mod number;
mod persistence;
mod registry;
mod rng;
mod table;

pub use fill::{FillOutcome, LootObserver};
pub use item::{Components, ItemDefinition, ItemStack};
pub use location::{LootLocation, LootWorld};
pub use rng::{LegacyRandom, LootRng};

use serde_json::Value;
use std::{collections::HashMap, sync::OnceLock};

pub type Result<T> = std::result::Result<T, LootError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LootError {
    InvalidData { path: String, message: String },
    Unsupported { path: String, kind: String },
    UnsupportedContext(&'static str),
    ZeroSeedRequiresCallerRng,
    RecursiveTable(String),
    LimitExceeded(&'static str),
}

impl std::fmt::Display for LootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidData { path, message } => {
                write!(f, "invalid loot data at {path}: {message}")
            }
            Self::Unsupported { path, kind } => {
                write!(f, "unsupported loot behavior at {path}: {kind}")
            }
            Self::UnsupportedContext(kind) => write!(f, "unsupported loot context: {kind}"),
            Self::ZeroSeedRequiresCallerRng => {
                f.write_str("seed zero requires caller-owned server-sequence or level RNG")
            }
            Self::RecursiveTable(key) => write!(f, "recursive loot table: {key}"),
            Self::LimitExceeded(kind) => write!(f, "loot evaluation limit exceeded: {kind}"),
        }
    }
}
impl std::error::Error for LootError {}

/// Input exports for a datapack-derived catalog. Arrays retain native registry
/// and holder-tag order. Reload by compiling a new engine and swapping its owner.
#[derive(Clone, Copy)]
pub struct CatalogSources<'a> {
    pub tables: &'a str,
    pub items: &'a str,
    pub enchantments: &'a str,
    pub instruments: &'a str,
    pub potions: &'a str,
    pub registry_types: &'a str,
}

impl CatalogSources<'static> {
    /// Current vanilla data, exported through the initialized 26.2 server's
    /// reloadable registry lookup and native codecs.
    pub const VANILLA: Self = Self {
        tables: include_str!("../../data/loot_26_2/tables.json"),
        items: include_str!("../../data/loot_26_2/items.json"),
        enchantments: include_str!("../../data/loot_26_2/enchantment.json"),
        instruments: include_str!("../../data/loot_26_2/instrument.json"),
        potions: include_str!("../../data/loot_26_2/potions.json"),
        registry_types: include_str!("../../data/loot_26_2/registry-types.json"),
    };

    /// Preserved catalog for the independently captured 26.1 native fixtures.
    pub const VANILLA_26_1: Self = Self {
        tables: include_str!("../../data/loot_26_1/tables.json"),
        items: include_str!("../../data/loot_26_1/items.json"),
        enchantments: include_str!("../../data/loot_26_1/enchantment.json"),
        instruments: include_str!("../../data/loot_26_1/instrument.json"),
        potions: include_str!("../../data/loot_26_1/potions.json"),
        registry_types: include_str!("../../data/loot_26_1/registry-types.json"),
    };
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LootContext<'a> {
    pub luck: f32,
    pub player_present: bool,
    pub location: Option<LootLocation<'a>>,
}

impl<'a> LootContext<'a> {
    pub const CHEST: Self = Self {
        luck: 0.0,
        player_present: false,
        location: None,
    };

    /// Supply an explicit origin and a borrowed loaded-world lookup. None is an
    /// intentionally absent native ORIGIN, distinct from missing world support.
    pub fn with_location<'w>(
        self,
        origin: Option<[f64; 3]>,
        world: &'w dyn LootWorld,
    ) -> LootContext<'w> {
        LootContext {
            luck: self.luck,
            player_present: self.player_present,
            location: Some(LootLocation { origin, world }),
        }
    }

    /// Preflight the supported context before a container clears deferred loot.
    pub fn validate(self) -> Result<()> {
        if self.player_present {
            return Err(LootError::UnsupportedContext("player"));
        }
        if self.luck.to_bits() != 0 {
            return Err(LootError::UnsupportedContext("luck other than +0.0"));
        }
        if self.location.is_some_and(|location| {
            location
                .origin
                .is_some_and(|origin| origin.iter().any(|value| !value.is_finite()))
        }) {
            return Err(LootError::UnsupportedContext("nonfinite origin"));
        }
        Ok(())
    }
}

/// Immutable compiled catalog. Failed/unsupported tables retain their diagnostic
/// instead of being confused with native missing-table EMPTY resolution.
#[derive(Debug)]
pub struct LootEngine {
    registry: registry::Registry,
    tables: Vec<Result<table::Table>>,
    keys: Vec<String>,
    names: HashMap<String, usize>,
    support: Vec<Result<table::ContextRequirements>>,
}

impl LootEngine {
    /// Process-wide built-in catalog. Both success and initialization failure are
    /// cached: repeated fills never parse the full JSON inputs.
    pub fn vanilla() -> Result<&'static Self> {
        static ENGINE: OnceLock<Result<LootEngine>> = OnceLock::new();
        ENGINE
            .get_or_init(|| Self::from_sources(CatalogSources::VANILLA))
            .as_ref()
            .map_err(Clone::clone)
    }

    pub fn from_sources(sources: CatalogSources<'_>) -> Result<Self> {
        let registry = registry::Registry::load(&sources)?;
        let values = json::parse(sources.tables, "tables")?;
        let values = json::object(&values, "tables")?;
        let mut keys = Vec::with_capacity(values.len());
        let mut names = HashMap::new();
        for key in values.keys() {
            let key = json::resource(key, "tables")?;
            if names.insert(key.clone(), keys.len()).is_some() {
                return Err(json::invalid("tables", "duplicate normalized table key"));
            }
            keys.push(key);
        }
        let tables: Vec<_> = values
            .iter()
            .zip(&keys)
            .map(|((_, value), key)| table::Table::compile(value, &registry, &names, key))
            .collect();
        let support = table::validate(&tables, &keys);
        Ok(Self {
            registry,
            tables,
            keys,
            names,
            support,
        })
    }

    /// Resolve once and retain the borrowed handle for repeated fills. Missing
    /// keys use native EMPTY behavior, including subsequent empty-slot shuffling.
    pub fn table(&self, key: &str) -> Result<LootTable<'_>> {
        let key = json::resource(key, "loot table")?;
        let index = self.names.get(&key).copied();
        if let Some(index) = index {
            self.support[index].clone()?;
        }
        Ok(LootTable {
            engine: self,
            key,
            index,
        })
    }

    pub fn table_keys(&self) -> impl Iterator<Item = &str> {
        self.keys.iter().map(String::as_str)
    }
    pub fn component_numeric_id(&self, key: &str) -> Option<u32> {
        self.registry.component_ids.get(key).copied()
    }
    pub fn potion_numeric_id(&self, key: &str) -> Option<u32> {
        self.registry.potions.get(key).copied()
    }

    /// Construct a transient stack, including empty/over-maximum counts used by
    /// loot functions. Persistent input should use `item_stack_from_json`.
    pub fn item_stack(&self, key: &str, count: i32) -> Result<ItemStack> {
        Ok(ItemStack::new(self.registry.item(key)?, count))
    }

    /// Validate, normalize and apply a component against this catalog. A failure
    /// leaves the stack unchanged. Unknown codecs/registry IDs are rejected.
    pub fn set_component(&self, stack: &mut ItemStack, key: &str, value: Value) -> Result<()> {
        let (key, value) = persistence::normalize(&self.registry, key, &value)?;
        stack.set(&key, value);
        Ok(())
    }

    /// Strict persistent-input admission: null is empty; non-null input requires
    /// a non-air item, count 1..=99 (omission means 1), and supported, registered
    /// components. Invalid explicit counts are errors, not native codec recovery.
    pub fn item_stack_from_json(&self, value: &Value) -> Result<Option<ItemStack>> {
        if value.is_null() {
            return Ok(None);
        }
        json::fields(value, &["id", "count", "components"], "item stack")?;
        let id = json::identifier(&value["id"], "item stack.id")?;
        let count = value
            .get("count")
            .map(|v| json::int(v, "item stack.count"))
            .transpose()?
            .unwrap_or(1);
        if id == "minecraft:air" || !(1..=99).contains(&count) {
            return Err(json::invalid(
                "item stack",
                "expected non-air item and count 1..=99",
            ));
        }
        let mut stack = self.item_stack(&id, count)?;
        if let Some(patch) = value.get("components") {
            let mut seen = std::collections::HashSet::new();
            for (key, value) in json::object(patch, "item stack.components")? {
                let (key, value) = persistence::normalize(&self.registry, key, value)?;
                if !seen.insert(key.clone()) {
                    return Err(json::invalid(
                        "item stack.components",
                        "duplicate normalized component key",
                    ));
                }
                stack.set(&key, value);
            }
        }
        Ok(Some(stack))
    }

    /// Canonical single-chest adapter: 27 slots, no inventory/menu dependency.
    pub fn fill_chest(&self, key: &str, seed: i64) -> Result<[Option<ItemStack>; 27]> {
        let mut slots = std::array::from_fn(|_| None);
        self.table(key)?.fill(&mut slots, seed)?;
        Ok(slots)
    }
}

#[derive(Debug)]
pub struct LootTable<'a> {
    engine: &'a LootEngine,
    key: String,
    index: Option<usize>,
}

impl LootTable<'_> {
    pub fn key(&self) -> &str {
        &self.key
    }
    pub fn is_missing(&self) -> bool {
        self.index.is_none()
    }
    pub fn random_sequence(&self) -> Option<&str> {
        self.index
            .and_then(|index| self.engine.tables[index].as_ref().ok())
            .and_then(|table| table.random_sequence.as_deref())
    }

    /// Check every reachable branch's required world context before draws or
    /// deferred-key consumption, including branches a particular seed skips.
    pub fn validate_context(&self, context: LootContext<'_>) -> Result<()> {
        context.validate()?;
        if let Some(index) = self.index {
            let requirements = self.engine.support[index].as_ref().map_err(Clone::clone)?;
            if requirements.location && context.location.is_none() {
                return Err(LootError::UnsupportedContext("loot location lookup"));
            }
        }
        Ok(())
    }

    pub fn generate(&self, seed: i64) -> Result<Vec<ItemStack>> {
        if seed == 0 {
            return Err(LootError::ZeroSeedRequiresCallerRng);
        }
        self.generate_with_rng(LootContext::CHEST, &mut LegacyRandom::new(seed))
    }

    pub fn generate_with_rng(
        &self,
        context: LootContext<'_>,
        rng: &mut dyn LootRng,
    ) -> Result<Vec<ItemStack>> {
        self.validate_context(context)?;
        let mut evaluation = evaluate::Evaluation {
            engine: self.engine,
            context,
            rng,
            output: Vec::new(),
            rolls: 0,
        };
        if let Some(index) = self.index {
            evaluation.table(
                self.engine.tables[index].as_ref().map_err(Clone::clone)?,
                &[],
                0,
            )?;
        }
        Ok(evaluation.output)
    }

    pub fn fill(&self, slots: &mut [Option<ItemStack>], seed: i64) -> Result<FillOutcome> {
        self.fill_with_context(slots, LootContext::CHEST, seed)
    }

    pub fn fill_with_context(
        &self,
        slots: &mut [Option<ItemStack>],
        context: LootContext<'_>,
        seed: i64,
    ) -> Result<FillOutcome> {
        if seed == 0 {
            return Err(LootError::ZeroSeedRequiresCallerRng);
        }
        self.fill_with_rng(slots, context, &mut LegacyRandom::new(seed))
    }

    /// Consume the caller's RNG in place. The caller owns named sequence selection,
    /// creation and persistence. On evaluation error, slots are unchanged but the
    /// RNG may have advanced; callers must not retry using the advanced source.
    pub fn fill_with_rng(
        &self,
        slots: &mut [Option<ItemStack>],
        context: LootContext<'_>,
        rng: &mut dyn LootRng,
    ) -> Result<FillOutcome> {
        self.fill_observed(slots, context, rng, &mut fill::NoObserver)
    }

    pub fn fill_observed(
        &self,
        slots: &mut [Option<ItemStack>],
        context: LootContext<'_>,
        rng: &mut dyn LootRng,
        observer: &mut dyn LootObserver,
    ) -> Result<FillOutcome> {
        if slots.len() > 65_536 {
            return Err(LootError::LimitExceeded("container slots"));
        }
        let items = self.generate_with_rng(context, rng)?;
        observer.generated(&items, rng);
        fill::place(items, slots, rng, observer)
    }
}
