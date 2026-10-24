use super::{json, persistence, Result};
use crate::structure::template::Nbt;
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc};

pub type Components = BTreeMap<String, Value>;

static EMPTY_COMPONENTS: Components = BTreeMap::new();

/// One shared native item definition. IDs belong to its source catalog version.
#[derive(Debug, PartialEq)]
pub struct ItemDefinition {
    pub(super) key: String,
    pub(super) numeric_id: u32,
    pub(super) defaults: Components,
    pub(super) max_stack_size: i32,
    pub(super) transient_types: Vec<String>,
}

impl ItemDefinition {
    pub fn id(&self) -> &str {
        &self.key
    }
    pub fn numeric_id(&self) -> u32 {
        self.numeric_id
    }
    pub fn defaults(&self) -> &Components {
        &self.defaults
    }
}

/// An item identity, count and persistent component patch. Definitions are shared
/// across all fills; clones/splits share patches until a function changes them.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemStack {
    pub(super) item: Arc<ItemDefinition>,
    pub(super) count: i32,
    patch: Arc<Components>,
}

impl ItemStack {
    pub(super) fn new(item: Arc<ItemDefinition>, count: i32) -> Self {
        Self {
            item,
            count,
            patch: Arc::default(),
        }
    }

    pub fn id(&self) -> &str {
        if self.is_empty() {
            "minecraft:air"
        } else {
            self.item.id()
        }
    }
    pub fn numeric_id(&self) -> u32 {
        if self.is_empty() {
            0
        } else {
            self.item.numeric_id()
        }
    }
    pub fn definition(&self) -> &Arc<ItemDefinition> {
        &self.item
    }
    pub fn count(&self) -> i32 {
        if self.is_empty() {
            0
        } else {
            self.count
        }
    }
    pub fn is_empty(&self) -> bool {
        self.count <= 0 || self.item.id() == "minecraft:air"
    }
    pub fn persistent_patch(&self) -> &Components {
        if self.is_empty() {
            &EMPTY_COMPONENTS
        } else {
            &self.patch
        }
    }

    pub fn component(&self, key: &str) -> Option<&Value> {
        if self.is_empty() {
            return None;
        }
        self.patch.get(key).or_else(|| self.item.defaults.get(key))
    }

    /// Zero-copy effective view: native defaults with the stack patch overlaid.
    pub fn effective_components(&self) -> impl Iterator<Item = (&str, &Value)> {
        let defaults = if self.is_empty() {
            &EMPTY_COMPONENTS
        } else {
            &self.item.defaults
        };
        defaults
            .iter()
            .filter(|(key, _)| !self.patch.contains_key(*key))
            .chain(self.persistent_patch().iter())
            .map(|(key, value)| (key.as_str(), value))
    }

    pub fn max_stack_size(&self) -> i32 {
        if self.is_empty() {
            return 1;
        }
        self.component("minecraft:max_stack_size")
            .and_then(Value::as_i64)
            .map_or(self.item.max_stack_size, |v| v as i32)
    }

    pub fn with_count(&self, count: i32) -> Self {
        if self.is_empty() {
            return self.clone();
        }
        Self {
            count,
            ..self.clone()
        }
    }

    pub(super) fn set(&mut self, key: &str, value: Value) {
        if self.item.defaults.get(key) == Some(&value) {
            if self.patch.contains_key(key) {
                Arc::make_mut(&mut self.patch).remove(key);
            }
        } else {
            Arc::make_mut(&mut self.patch).insert(key.into(), value);
        }
    }

    pub(super) fn transmute(&mut self, target: Arc<ItemDefinition>, fresh: bool) {
        if fresh {
            *self = Self::new(target, 1);
        } else {
            self.item = target;
            Arc::make_mut(&mut self.patch)
                .retain(|key, value| self.item.defaults.get(key) != Some(value));
        }
    }

    pub(super) fn component_int(&self, key: &str) -> Option<i32> {
        self.component(key)
            .and_then(Value::as_i64)
            .and_then(|v| i32::try_from(v).ok())
    }

    pub(super) fn component_object(&self, key: &str) -> serde_json::Map<String, Value> {
        self.component(key)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    }

    /// Persistent ItemStack shape; effective defaults are omitted. Empty stacks
    /// have the portable oracle's null representation. Transient over-99 counts
    /// can be represented here; `to_nbt` checks their persistence eligibility.
    pub fn to_json(&self) -> Value {
        if self.is_empty() {
            return Value::Null;
        }
        let mut result = json!({"id": self.id(), "count": self.count});
        if !self.patch.is_empty() {
            result["components"] =
                serde_json::to_value(self.patch.as_ref()).expect("JSON component map");
        }
        result
    }

    /// Native persistent patch as typed NBT, ready for the protocol NBT encoder.
    /// Compounds have canonical key order, with native numeric/list tag types.
    pub fn to_nbt(&self) -> Result<Nbt> {
        if self.is_empty() {
            return Err(json::invalid(
                self.id(),
                "empty stack has no persistent ItemStack encoding",
            ));
        }
        if self.count > 99 {
            return Err(json::invalid(
                self.id(),
                "persistent ItemStack count exceeds 99",
            ));
        }
        let mut root = BTreeMap::from([
            ("id".into(), Nbt::String(self.id().into())),
            ("count".into(), Nbt::Int(self.count)),
        ]);
        if !self.patch.is_empty() {
            let components = self
                .patch
                .iter()
                .map(|(key, value)| Ok((key.clone(), persistence::component(key, value)?)))
                .collect::<Result<_>>()?;
            root.insert("components".into(), Nbt::Compound(components));
        }
        Ok(Nbt::Compound(root))
    }
}
