use super::{
    json,
    registry::{holders, Registry},
    rng, ItemStack, LootError, LootRng, Result,
};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug)]
pub(super) struct Enchantment {
    pub key: String,
    pub weight: i32,
    pub max_level: i32,
    supported: Vec<usize>,
    primary: Option<Vec<usize>>,
    exclusive: Vec<usize>,
    min_cost: Cost,
    max_cost: Cost,
}

#[derive(Debug)]
struct Cost {
    base: i32,
    per_level: i32,
}

impl Cost {
    fn compile(value: &Value, path: &str) -> Result<Self> {
        Ok(Self {
            base: json::int(&value["base"], path)?,
            per_level: json::int(&value["per_level_above_first"], path)?,
        })
    }

    fn at(&self, level: i32) -> i32 {
        self.base
            .wrapping_add(self.per_level.wrapping_mul(level - 1))
    }
}

impl Enchantment {
    pub fn compile(
        row: &Value,
        items: &HashMap<String, usize>,
        item_tags: &HashMap<String, Vec<usize>>,
        names: &HashMap<String, usize>,
        tags: &HashMap<String, Vec<usize>>,
    ) -> Result<Self> {
        let key = json::identifier(&row["key"], "enchantment key")?;
        let value = &row["value"];
        let weight = json::int(&value["weight"], &key)?;
        let max_level = json::int(&value["max_level"], &key)?;
        if weight < 1 || !(1..=255).contains(&max_level) {
            return Err(json::invalid(&key, "invalid weight or max_level"));
        }
        Ok(Self {
            weight,
            max_level,
            supported: holders(&value["supported_items"], items, item_tags, &key)?,
            primary: value
                .get("primary_items")
                .map(|v| holders(v, items, item_tags, &key))
                .transpose()?,
            exclusive: value
                .get("exclusive_set")
                .map(|v| holders(v, names, tags, &key))
                .transpose()?
                .unwrap_or_default(),
            min_cost: Cost::compile(&value["min_cost"], &key)?,
            max_cost: Cost::compile(&value["max_cost"], &key)?,
            key,
        })
    }

    fn primary(&self, item: usize) -> bool {
        self.supported.contains(&item) && self.primary.as_ref().is_none_or(|v| v.contains(&item))
    }
}

pub(super) fn apply(stack: &mut ItemStack, key: &str, level: i32, upgrade: bool) {
    let component = if stack.id() == "minecraft:enchanted_book" {
        "minecraft:stored_enchantments"
    } else {
        "minecraft:enchantments"
    };
    if stack.component(component).is_none() {
        return;
    }
    let mut values = stack.component_object(component);
    let previous = values.get(key).and_then(Value::as_i64).unwrap_or(0) as i32;
    let level = if upgrade { previous.max(level) } else { level }.clamp(0, 255);
    if level == 0 {
        values.remove(key);
    } else {
        values.insert(key.into(), Value::from(level));
    }
    stack.set(component, Value::Object(values));
}

pub(super) fn randomly(
    stack: &mut ItemStack,
    options: &[usize],
    only_compatible: bool,
    registry: &Registry,
    rng: &mut dyn LootRng,
) -> Result<()> {
    let book = stack.id() == "minecraft:book";
    let item = registry.item_names[stack.id()];
    let candidates: Vec<_> = options
        .iter()
        .copied()
        .filter(|&index| {
            book || !only_compatible || registry.enchantments[index].supported.contains(&item)
        })
        .collect();
    if candidates.is_empty() {
        return Ok(());
    }
    // Util.getRandomSafe draws even when the candidate list has one element.
    let choice = candidates[rng.next_int(candidates.len() as i32) as usize];
    let enchantment = &registry.enchantments[choice];
    let level = rng::between(rng, 1, enchantment.max_level)?;
    if book {
        stack.transmute(registry.enchanted_book.clone(), true);
    }
    apply(stack, &enchantment.key, level, true);
    Ok(())
}

pub(super) fn with_levels(
    stack: &mut ItemStack,
    cost: i32,
    options: &[usize],
    registry: &Registry,
    rng: &mut dyn LootRng,
) -> Result<()> {
    let selected = select(stack, cost, options, registry, rng)?;
    if stack.id() == "minecraft:book" {
        stack.transmute(registry.enchanted_book.clone(), true);
    }
    for (index, level) in selected {
        apply(stack, &registry.enchantments[index].key, level, true);
    }
    Ok(())
}

fn select(
    stack: &ItemStack,
    mut cost: i32,
    options: &[usize],
    registry: &Registry,
    rng: &mut dyn LootRng,
) -> Result<Vec<(usize, i32)>> {
    let Some(enchantable) = stack.component("minecraft:enchantable") else {
        return Ok(Vec::new());
    };
    let enchantability = json::int(&enchantable["value"], "enchantable.value")?;
    if enchantability < 0 {
        return Err(json::invalid(
            "enchantable.value",
            "negative enchantability",
        ));
    }
    let bound = enchantability / 4 + 1;
    cost = cost
        .wrapping_add(1)
        .wrapping_add(rng.next_int(bound))
        .wrapping_add(rng.next_int(bound));
    let span = (rng.next_float() + rng.next_float() - 1.0_f32) * 0.15_f32;
    let adjusted = cost as f32 + cost as f32 * span;
    // Math.round(float): exact float32 input, ties towards positive infinity.
    cost = ((f64::from(adjusted) + 0.5).floor() as i32).max(1);
    let book = stack.id() == "minecraft:book";
    let item = registry.item_names[stack.id()];
    let mut candidates = Vec::new();
    for &index in options {
        let enchantment = &registry.enchantments[index];
        if !book && !enchantment.primary(item) {
            continue;
        }
        for level in (1..=enchantment.max_level).rev() {
            if enchantment.min_cost.at(level) <= cost && cost <= enchantment.max_cost.at(level) {
                candidates.push((index, level));
                break;
            }
        }
    }
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let mut selected = vec![weighted(&candidates, registry, rng)?];
    while rng.next_int(50) <= cost {
        let last = selected.last().unwrap().0;
        candidates.retain(|&(index, _)| {
            index != last
                && !registry.enchantments[last].exclusive.contains(&index)
                && !registry.enchantments[index].exclusive.contains(&last)
        });
        if candidates.is_empty() {
            break;
        }
        selected.push(weighted(&candidates, registry, rng)?);
        cost /= 2;
    }
    Ok(selected)
}

fn weighted(
    candidates: &[(usize, i32)],
    registry: &Registry,
    rng: &mut dyn LootRng,
) -> Result<(usize, i32)> {
    let total = candidates
        .iter()
        .try_fold(0_i32, |sum, &(index, _)| {
            sum.checked_add(registry.enchantments[index].weight)
        })
        .ok_or(LootError::LimitExceeded("enchantment weight sum"))?;
    let mut choice = rng.next_int(total);
    for &candidate in candidates {
        choice -= registry.enchantments[candidate.0].weight;
        if choice < 0 {
            return Ok(candidate);
        }
    }
    unreachable!("positive weighted candidate sum")
}
