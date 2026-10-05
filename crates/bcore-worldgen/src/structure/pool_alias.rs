//! Configured jigsaw pool aliases from the pinned 26.1 binding codec.
//!
//! Resolve once at the sampled, unprojected start position, before assembly.
//! Alias randomness uses its own positional Legacy source, not the structure's
//! large-feature RNG. Bindings and nested groups retain their configuration
//! order; the resulting map is used for a single lookup, never alias chaining.

use super::template::{invalid, Result, TemplateRandom};
use crate::feature_world::{FeatureError, Pos};
use crate::random::get_seed;
use crate::simplex::JavaRandom;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Validated native `PoolAliasBinding.CODEC.listOf()` configuration.
///
/// The internal bindings are immutable so invalid weights cannot reach RNG
/// calls. Zero-weight entries are retained (including in `all_targets`), but a
/// random list must contain at least one positive weight. A selected group may
/// itself contain no bindings.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PoolAliasBindings(Vec<Binding>);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Binding {
    Direct {
        alias: String,
        target: String,
    },
    Random {
        alias: String,
        targets: Weighted<String>,
    },
    RandomGroup {
        groups: Weighted<Vec<Binding>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Weighted<T> {
    entries: Vec<(T, i32)>,
    total: i32,
}

impl PoolAliasBindings {
    pub fn from_json(value: &Value) -> Result<Self> {
        Ok(Self(parse_bindings(value)?))
    }

    /// Read the optional `pool_aliases` field of a jigsaw configuration.
    pub fn from_structure_json(value: &Value) -> Result<Self> {
        let object = value
            .as_object()
            .ok_or_else(|| invalid("pool aliases require a structure object"))?;
        match object.get("pool_aliases") {
            None | Some(Value::Null) => Ok(Self::default()),
            Some(value) => Self::from_json(value),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Native codec representation, with normalized resource identifiers.
    pub fn to_json(&self) -> Value {
        Value::Array(self.0.iter().map(Binding::to_json).collect())
    }

    /// All possible targets in native stream order, including duplicates and
    /// zero-weight alternatives. Useful for validating asset dependencies.
    pub fn all_targets(&self) -> Vec<&str> {
        let mut targets = Vec::new();
        for binding in &self.0 {
            binding.all_targets(&mut targets);
        }
        targets
    }

    /// Native `PoolAliasLookup.create(bindings, startPos, worldSeed)`.
    ///
    /// `origin` includes the sampled start height, before start-jigsaw anchoring,
    /// heightmap projection, or rotation. No caller-owned RNG is consumed.
    pub fn resolve(&self, world_seed: i64, origin: Pos) -> Result<BTreeMap<String, String>> {
        if self.is_empty() {
            return Ok(BTreeMap::new());
        }
        self.resolve_with_random(&mut positional_random(world_seed, origin))
    }

    /// Resolve on a caller-owned source, preserving native draw order and its
    /// continuation even on duplicate-key failure. Duplicate aliases, including
    /// identical pairs, fail only after every binding has been evaluated, as in
    /// native `ImmutableMap.Builder.build()`.
    pub fn resolve_with_random(
        &self,
        random: &mut (impl TemplateRandom + ?Sized),
    ) -> Result<BTreeMap<String, String>> {
        let mut ordered = Vec::new();
        self.for_each_resolved(random, |alias, target| {
            ordered.push((alias.to_owned(), target.to_owned()));
        });
        let mut resolved = BTreeMap::new();
        for (alias, target) in ordered {
            if let Some(previous) = resolved.insert(alias.clone(), target.clone()) {
                return Err(invalid(format!(
                    "duplicate pool alias {alias}: {previous} and {target}"
                )));
            }
        }
        Ok(resolved)
    }

    /// Native `forEachResolved`: visit selected pairs in configuration order.
    /// This intentionally emits duplicates; map construction validates them.
    pub fn for_each_resolved(
        &self,
        random: &mut (impl TemplateRandom + ?Sized),
        mut consumer: impl FnMut(&str, &str),
    ) {
        for binding in &self.0 {
            binding.resolve(random, &mut consumer);
        }
    }
}

impl Serialize for PoolAliasBindings {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        self.to_json().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PoolAliasBindings {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        Self::from_json(&Value::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// `RandomSource.create(seed).forkPositional().at(origin)` uses Legacy sources
/// on both sides of the fork. `nextLong` is signed-word addition, and the native
/// position hash deliberately wraps its X multiplication in 32 bits.
pub fn positional_random(world_seed: i64, origin: Pos) -> JavaRandom {
    let factory_seed = JavaRandom::new(world_seed).next_long();
    JavaRandom::new(factory_seed ^ get_seed(origin.0, origin.1, origin.2))
}

fn parse_bindings(value: &Value) -> Result<Vec<Binding>> {
    value
        .as_array()
        .ok_or_else(|| invalid("pool aliases must be a list"))?
        .iter()
        .map(Binding::from_json)
        .collect()
}

impl Binding {
    fn from_json(value: &Value) -> Result<Self> {
        Ok(match identifier(&value["type"])?.as_str() {
            "minecraft:direct" => Self::Direct {
                alias: identifier(&value["alias"])?,
                target: identifier(&value["target"])?,
            },
            "minecraft:random" => Self::Random {
                alias: identifier(&value["alias"])?,
                targets: Weighted::from_json(&value["targets"], identifier)?,
            },
            "minecraft:random_group" => Self::RandomGroup {
                groups: Weighted::from_json(&value["groups"], parse_bindings)?,
            },
            other => {
                return Err(FeatureError::Unsupported(format!(
                    "pool alias binding {other}"
                )))
            }
        })
    }

    fn to_json(&self) -> Value {
        match self {
            Self::Direct { alias, target } => {
                json!({"type": "minecraft:direct", "alias": alias, "target": target})
            }
            Self::Random { alias, targets } => {
                json!({"type": "minecraft:random", "alias": alias, "targets": targets.to_json(|s| json!(s))})
            }
            Self::RandomGroup { groups } => {
                json!({"type": "minecraft:random_group", "groups": groups.to_json(|g| Value::Array(g.iter().map(Self::to_json).collect()))})
            }
        }
    }

    fn resolve(
        &self,
        random: &mut (impl TemplateRandom + ?Sized),
        consumer: &mut impl FnMut(&str, &str),
    ) {
        match self {
            Self::Direct { alias, target } => consumer(alias, target),
            Self::Random { alias, targets } => consumer(alias, targets.choose(random)),
            Self::RandomGroup { groups } => {
                for binding in groups.choose(random) {
                    binding.resolve(random, consumer);
                }
            }
        }
    }

    fn all_targets<'a>(&'a self, targets: &mut Vec<&'a str>) {
        match self {
            Self::Direct { target, .. } => targets.push(target),
            Self::Random {
                targets: choices, ..
            } => {
                targets.extend(choices.entries.iter().map(|(target, _)| target.as_str()));
            }
            Self::RandomGroup { groups } => {
                for (group, _) in &groups.entries {
                    for binding in group {
                        binding.all_targets(targets);
                    }
                }
            }
        }
    }
}

impl<T> Weighted<T> {
    fn from_json(value: &Value, parse: impl Fn(&Value) -> Result<T>) -> Result<Self> {
        let rows = value
            .as_array()
            .ok_or_else(|| invalid("pool alias weighted choices must be a list"))?;
        let mut entries = Vec::with_capacity(rows.len());
        let mut total = 0_i32;
        for row in rows {
            let data = parse(&row["data"])?;
            let weight = codec_int(&row["weight"])?;
            if weight < 0 {
                return Err(invalid("negative pool alias weight"));
            }
            total = total
                .checked_add(weight)
                .ok_or_else(|| invalid("pool alias weight overflow"))?;
            entries.push((data, weight));
        }
        if total == 0 {
            return Err(invalid(
                "pool alias choices require a positive total weight",
            ));
        }
        Ok(Self { entries, total })
    }

    fn choose(&self, random: &mut (impl TemplateRandom + ?Sized)) -> &T {
        // WeightedList.getRandomOrThrow always draws, even for a singleton or
        // weight=1. Flat (<64) and compact selectors have identical ordering.
        let mut choice = random.next_int(self.total as usize);
        for (value, weight) in &self.entries {
            if choice < *weight as usize {
                return value;
            }
            choice -= *weight as usize;
        }
        unreachable!("validated weighted choices and bounded RNG")
    }

    fn to_json(&self, encode: impl Fn(&T) -> Value) -> Value {
        Value::Array(
            self.entries
                .iter()
                .map(|(value, weight)| json!({"data": encode(value), "weight": weight}))
                .collect(),
        )
    }
}

fn identifier(value: &Value) -> Result<String> {
    let name = value
        .as_str()
        .ok_or_else(|| invalid("pool alias identifier must be a string"))?;
    let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
    let namespace = if namespace.is_empty() {
        "minecraft"
    } else {
        namespace
    };
    let valid = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c);
    if !namespace.bytes().all(valid) || !path.bytes().all(|c| valid(c) || c == b'/') {
        return Err(invalid(format!("invalid pool alias identifier {name}")));
    }
    Ok(format!("{namespace}:{path}"))
}

fn codec_int(value: &Value) -> Result<i32> {
    // JsonOps' integer codec calls Number.intValue: it truncates fractions and
    // wraps integer overflow before NON_NEGATIVE_INT validates the result.
    let number = value
        .as_number()
        .ok_or_else(|| invalid("pool alias weight must be numeric"))?;
    let text = number.to_string();
    let (mantissa, exponent) = text
        .split_once(['e', 'E'])
        .map_or((text.as_str(), 0), |(m, e)| {
            (m, e.parse::<i32>().expect("JSON number exponent"))
        });
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.trim_start_matches('-');
    let decimals = mantissa.split_once('.').map_or(0, |(_, tail)| tail.len()) as i32;
    let digits: Vec<_> = mantissa.bytes().filter(u8::is_ascii_digit).collect();
    let shift = exponent - decimals;
    let count = if shift < 0 {
        digits.len().saturating_sub((-shift) as usize)
    } else {
        digits.len()
    };
    let mut n = digits[..count].iter().fold(0_u32, |n, d| {
        n.wrapping_mul(10).wrapping_add(u32::from(d - b'0'))
    });
    if shift >= 32 {
        n = 0;
    } else if shift > 0 {
        n = n.wrapping_mul(10_u32.wrapping_pow(shift as u32));
    }
    Ok(if negative { n.wrapping_neg() } else { n } as i32)
}
