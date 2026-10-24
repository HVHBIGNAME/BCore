use super::{
    function::{self, Function},
    item::ItemDefinition,
    json,
    number::{self, Condition, Number},
    registry::Registry,
    LootError, Result,
};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

#[derive(Debug)]
pub(super) struct Table {
    pub pools: Vec<Pool>,
    pub functions: Vec<Function>,
    pub random_sequence: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ContextRequirements {
    pub location: bool,
}

#[derive(Debug)]
pub(super) struct Pool {
    pub conditions: Vec<Condition>,
    pub rolls: Number,
    pub bonus_rolls: Number,
    pub entries: Vec<Entry>,
    pub functions: Vec<Function>,
}

#[derive(Debug)]
pub(super) struct Entry {
    pub conditions: Vec<Condition>,
    pub kind: EntryKind,
}

#[derive(Debug)]
pub(super) enum EntryKind {
    Singleton(Singleton),
    Alternatives(Vec<Entry>),
    Group(Vec<Entry>),
    Sequence(Vec<Entry>),
}

#[derive(Debug)]
pub(super) struct Singleton {
    pub weight: i32,
    pub functions: Vec<Function>,
    pub kind: SingletonKind,
}

#[derive(Debug)]
pub(super) enum SingletonKind {
    Item(Arc<ItemDefinition>),
    Empty,
    Reference(Option<usize>),
    Inline(Box<Table>),
}

impl Table {
    pub fn compile(
        value: &Value,
        registry: &Registry,
        names: &HashMap<String, usize>,
        path: &str,
    ) -> Result<Self> {
        json::fields(
            value,
            &["type", "pools", "functions", "random_sequence"],
            path,
        )?;
        if let Some(kind) = value.get("type") {
            json::identifier(kind, path)?;
        }
        Ok(Self {
            pools: json::optional_array(value, "pools", path)?
                .iter()
                .enumerate()
                .map(|(i, value)| {
                    Pool::compile(value, registry, names, &format!("{path}.pools[{i}]"))
                })
                .collect::<Result<_>>()?,
            functions: function::compile(value, registry, path)?,
            random_sequence: value
                .get("random_sequence")
                .map(|v| json::identifier(v, path))
                .transpose()?,
        })
    }

    fn references(&self, result: &mut Vec<usize>) {
        for pool in &self.pools {
            for entry in &pool.entries {
                entry.references(result);
            }
        }
    }

    fn needs_location(&self) -> bool {
        self.functions.iter().any(Function::needs_location)
            || self.pools.iter().any(|pool| {
                pool.conditions.iter().any(Condition::needs_location)
                    || pool.functions.iter().any(Function::needs_location)
                    || pool.entries.iter().any(Entry::needs_location)
            })
    }
}

impl Pool {
    fn compile(
        value: &Value,
        registry: &Registry,
        names: &HashMap<String, usize>,
        path: &str,
    ) -> Result<Self> {
        json::fields(
            value,
            &["conditions", "rolls", "bonus_rolls", "entries", "functions"],
            path,
        )?;
        Ok(Self {
            conditions: number::conditions(value, path)?,
            rolls: Number::compile(&value["rolls"], &format!("{path}.rolls"))?,
            bonus_rolls: value
                .get("bonus_rolls")
                .map(|v| Number::compile(v, path))
                .transpose()?
                .unwrap_or(Number::Constant(0.0)),
            entries: json::array(&value["entries"], path)?
                .iter()
                .enumerate()
                .map(|(i, v)| Entry::compile(v, registry, names, &format!("{path}.entries[{i}]")))
                .collect::<Result<_>>()?,
            functions: function::compile(value, registry, path)?,
        })
    }
}

impl Entry {
    fn compile(
        value: &Value,
        registry: &Registry,
        names: &HashMap<String, usize>,
        path: &str,
    ) -> Result<Self> {
        let name = json::identifier(&value["type"], path)?;
        let conditions = number::conditions(value, path)?;
        if matches!(
            name.as_str(),
            "minecraft:alternatives" | "minecraft:group" | "minecraft:sequence"
        ) {
            json::fields(value, &["type", "conditions", "children"], path)?;
            let children = json::array(&value["children"], path)?
                .iter()
                .enumerate()
                .map(|(i, v)| Self::compile(v, registry, names, &format!("{path}.children[{i}]")))
                .collect::<Result<_>>()?;
            let kind = match name.as_str() {
                "minecraft:alternatives" => EntryKind::Alternatives(children),
                "minecraft:group" => EntryKind::Group(children),
                _ => EntryKind::Sequence(children),
            };
            return Ok(Self { conditions, kind });
        }
        let kind = match name.as_str() {
            "minecraft:item" => {
                json::fields(
                    value,
                    &[
                        "type",
                        "conditions",
                        "weight",
                        "quality",
                        "functions",
                        "name",
                    ],
                    path,
                )?;
                SingletonKind::Item(registry.item(json::string(&value["name"], path)?)?)
            }
            "minecraft:empty" => {
                json::fields(
                    value,
                    &["type", "conditions", "weight", "quality", "functions"],
                    path,
                )?;
                SingletonKind::Empty
            }
            "minecraft:loot_table" => {
                json::fields(
                    value,
                    &[
                        "type",
                        "conditions",
                        "weight",
                        "quality",
                        "functions",
                        "value",
                    ],
                    path,
                )?;
                if value["value"].is_string() {
                    let key = json::identifier(&value["value"], path)?;
                    SingletonKind::Reference(names.get(&key).copied())
                } else {
                    SingletonKind::Inline(Box::new(Table::compile(
                        &value["value"],
                        registry,
                        names,
                        &format!("{path}.value"),
                    )?))
                }
            }
            _ => return Err(json::unsupported(path, &name)),
        };
        let weight = value
            .get("weight")
            .map(|v| json::int(v, path))
            .transpose()?
            .unwrap_or(1);
        let quality = value
            .get("quality")
            .map(|v| json::int(v, path))
            .transpose()?
            .unwrap_or(0);
        // Luck is explicitly restricted to +0; cache the native float32 weight.
        let weight = number::floor(weight as f32 + quality as f32 * 0.0_f32).max(0);
        Ok(Self {
            conditions,
            kind: EntryKind::Singleton(Singleton {
                weight,
                functions: function::compile(value, registry, path)?,
                kind,
            }),
        })
    }

    fn references(&self, result: &mut Vec<usize>) {
        match &self.kind {
            EntryKind::Singleton(single) => match &single.kind {
                SingletonKind::Reference(Some(index)) => result.push(*index),
                SingletonKind::Inline(table) => table.references(result),
                _ => {}
            },
            EntryKind::Alternatives(children)
            | EntryKind::Group(children)
            | EntryKind::Sequence(children) => {
                for child in children {
                    child.references(result);
                }
            }
        }
    }

    fn needs_location(&self) -> bool {
        self.conditions.iter().any(Condition::needs_location)
            || match &self.kind {
                EntryKind::Singleton(single) => {
                    single.functions.iter().any(Function::needs_location)
                        || matches!(&single.kind, SingletonKind::Inline(table) if table.needs_location())
                }
                EntryKind::Alternatives(children)
                | EntryKind::Group(children)
                | EntryKind::Sequence(children) => children.iter().any(Self::needs_location),
            }
    }
}

/// Validate the whole reachable graph once, before any RNG is consumed. A table
/// containing an unsupported branch remains unavailable even if a seed skips it.
pub(super) fn validate(
    tables: &[Result<Table>],
    keys: &[String],
) -> Vec<Result<ContextRequirements>> {
    fn visit(
        index: usize,
        tables: &[Result<Table>],
        keys: &[String],
        active: &mut Vec<usize>,
        cache: &mut [Option<Result<ContextRequirements>>],
    ) -> Result<ContextRequirements> {
        if let Some(result) = &cache[index] {
            return result.clone();
        }
        if active.contains(&index) {
            return Err(LootError::RecursiveTable(keys[index].clone()));
        }
        if active.len() >= 64 {
            return Err(LootError::LimitExceeded("nested loot table depth"));
        }
        let table = tables[index].as_ref().map_err(Clone::clone)?;
        let mut references = Vec::new();
        table.references(&mut references);
        active.push(index);
        let result = references.into_iter().try_fold(
            ContextRequirements {
                location: table.needs_location(),
            },
            |mut requirements, child| {
                requirements.location |= visit(child, tables, keys, active, cache)?.location;
                Ok(requirements)
            },
        );
        active.pop();
        cache[index] = Some(result.clone());
        result
    }
    let mut cache = vec![None; tables.len()];
    (0..tables.len())
        .map(|index| visit(index, tables, keys, &mut Vec::new(), &mut cache))
        .collect()
}
