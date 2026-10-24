use super::{enchant::Enchantment, item::ItemDefinition, json, CatalogSources, Result};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

/// Effect metadata admitted by the captured chest tables. Other effect classes
/// need registry authority before they can participate in this subset.
pub(super) fn stew_instantaneous(id: &str) -> Option<bool> {
    match id {
        "minecraft:saturation" => Some(true),
        "minecraft:night_vision"
        | "minecraft:jump_boost"
        | "minecraft:weakness"
        | "minecraft:blindness"
        | "minecraft:poison" => Some(false),
        _ => None,
    }
}

#[derive(Deserialize)]
struct Tag {
    tag: String,
    members_native_order: Vec<String>,
}

#[derive(Debug)]
pub(super) struct Registry {
    pub items: Vec<Arc<ItemDefinition>>,
    pub item_names: HashMap<String, usize>,
    pub enchantments: Vec<Enchantment>,
    pub enchantment_names: HashMap<String, usize>,
    pub enchantment_tags: HashMap<String, Vec<usize>>,
    pub instruments: Vec<String>,
    pub instrument_names: HashMap<String, usize>,
    pub instrument_tags: HashMap<String, Vec<usize>>,
    pub potions: HashMap<String, u32>,
    pub component_ids: HashMap<String, u32>,
    pub enchanted_book: Arc<ItemDefinition>,
}

fn names<'a>(keys: impl Iterator<Item = &'a str>, path: &str) -> Result<HashMap<String, usize>> {
    let mut names = HashMap::new();
    for (index, key) in keys.enumerate() {
        let key = json::resource(key, path)?;
        if names.insert(key, index).is_some() {
            return Err(json::invalid(path, "duplicate registry key"));
        }
    }
    Ok(names)
}

fn tags(
    value: &Value,
    names: &HashMap<String, usize>,
    path: &str,
) -> Result<HashMap<String, Vec<usize>>> {
    let rows: Vec<Tag> =
        serde_json::from_value(value.clone()).map_err(|e| json::invalid(path, e.to_string()))?;
    let mut result = HashMap::new();
    for row in rows {
        let mut members = Vec::new();
        for member in row.members_native_order {
            members.push(
                *names
                    .get(&member)
                    .ok_or_else(|| json::invalid(path, format!("unknown tag member {member}")))?,
            );
        }
        if result.insert(row.tag, members).is_some() {
            return Err(json::invalid(path, "duplicate tag"));
        }
    }
    Ok(result)
}

/// HolderSet order is data, not the order of a hash map or a sorted tag set.
pub(super) fn holders(
    value: &Value,
    names: &HashMap<String, usize>,
    tags: &HashMap<String, Vec<usize>>,
    path: &str,
) -> Result<Vec<usize>> {
    if let Some(name) = value.as_str() {
        if let Some(tag) = name.strip_prefix('#') {
            let tag = json::resource(tag, path)?;
            return tags
                .get(&tag)
                .cloned()
                .ok_or_else(|| json::invalid(path, format!("unknown tag #{tag}")));
        }
        let key = json::resource(name, path)?;
        return names
            .get(&key)
            .map(|v| vec![*v])
            .ok_or_else(|| json::invalid(path, format!("unknown registry key {key}")));
    }
    let mut result = Vec::new();
    for member in json::array(value, path)? {
        let key = json::identifier(member, path)?;
        let index = *names
            .get(&key)
            .ok_or_else(|| json::invalid(path, format!("unknown registry key {key}")))?;
        if result.contains(&index) {
            return Err(json::invalid(path, "duplicate holder"));
        }
        result.push(index);
    }
    Ok(result)
}

impl Registry {
    pub fn load(sources: &CatalogSources<'_>) -> Result<Self> {
        let item_data = json::parse(sources.items, "items")?;
        let mut items = Vec::new();
        for row in json::array(&item_data["items_native_order"], "items")? {
            let key = json::identifier(&row["key"], "item key")?;
            let id = json::int(&row["id"], &key)?;
            let max_stack_size = json::int(&row["max_stack_size"], &key)?;
            if id < 0 || max_stack_size < 1 {
                return Err(json::invalid(&key, "invalid item ID or stack size"));
            }
            let defaults = serde_json::from_value(row["components"]["persistent"].clone())
                .map_err(|e| json::invalid(&key, format!("component defaults: {e}")))?;
            let transient_types = serde_json::from_value(
                row["components"]["unserialized_transient_types"].clone(),
            )
            .map_err(|e| json::invalid(&key, format!("transient component inventory: {e}")))?;
            items.push(Arc::new(ItemDefinition {
                key,
                numeric_id: id as u32,
                defaults,
                max_stack_size,
                transient_types,
            }));
        }
        let item_names = names(items.iter().map(|v| v.id()), "items")?;
        let item_tags = tags(&item_data["tags"], &item_names, "item tags")?;
        let book = *item_names
            .get("minecraft:enchanted_book")
            .ok_or_else(|| json::invalid("items", "missing enchanted_book"))?;
        let enchantment_data = json::parse(sources.enchantments, "enchantments")?;
        let enchantment_rows =
            json::array(&enchantment_data["values_native_order"], "enchantments")?;
        let enchantment_names = names(
            enchantment_rows
                .iter()
                .map(|v| v["key"].as_str().unwrap_or("")),
            "enchantments",
        )?;
        let enchantment_tags = tags(
            &enchantment_data["tags"],
            &enchantment_names,
            "enchantment tags",
        )?;
        let mut enchantments = Vec::new();
        for row in enchantment_rows {
            enchantments.push(Enchantment::compile(
                row,
                &item_names,
                &item_tags,
                &enchantment_names,
                &enchantment_tags,
            )?);
        }
        let instrument_data = json::parse(sources.instruments, "instruments")?;
        let instruments = json::array(&instrument_data["values_native_order"], "instruments")?
            .iter()
            .map(|v| json::identifier(&v["key"], "instrument"))
            .collect::<Result<Vec<_>>>()?;
        let instrument_names = names(instruments.iter().map(String::as_str), "instruments")?;
        let instrument_tags = tags(
            &instrument_data["tags"],
            &instrument_names,
            "instrument tags",
        )?;
        let potion_data = json::parse(sources.potions, "potions")?;
        let potions = registry_ids(json::array(&potion_data, "potions")?, "potions")?;
        let component_data = json::parse(sources.registry_types, "registry types")?;
        let component_ids = registry_ids(
            json::array(&component_data["DATA_COMPONENT_TYPE"], "component types")?,
            "component types",
        )?;
        Ok(Self {
            enchanted_book: items[book].clone(),
            items,
            item_names,
            enchantments,
            enchantment_names,
            enchantment_tags,
            instruments,
            instrument_names,
            instrument_tags,
            potions,
            component_ids,
        })
    }

    pub fn item(&self, key: &str) -> Result<Arc<ItemDefinition>> {
        let key = json::resource(key, "item")?;
        let index = self
            .item_names
            .get(&key)
            .ok_or_else(|| json::invalid(&key, "unknown item"))?;
        let item = &self.items[*index];
        if !item.transient_types.is_empty() {
            return Err(json::unsupported(
                &key,
                "unserialized transient item defaults",
            ));
        }
        Ok(item.clone())
    }

    pub fn enchantment_options(&self, value: Option<&Value>, path: &str) -> Result<Vec<usize>> {
        match value {
            Some(v) => holders(v, &self.enchantment_names, &self.enchantment_tags, path),
            None => Ok((0..self.enchantments.len()).collect()),
        }
    }
}

fn registry_ids(rows: &[Value], path: &str) -> Result<HashMap<String, u32>> {
    let mut result = HashMap::new();
    for row in rows {
        let key = json::identifier(&row["key"], path)?;
        let id = json::int(&row["id"], path)?;
        if id < 0 || result.insert(key, id as u32).is_some() {
            return Err(json::invalid(path, "invalid or duplicate registry ID"));
        }
    }
    Ok(result)
}
