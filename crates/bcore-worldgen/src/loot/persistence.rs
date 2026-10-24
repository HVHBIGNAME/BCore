//! Component-specific codecs keep NBT types explicit. JSON's numeric shape is
//! insufficient to infer the codec's integer, float, boolean and list tags.
use super::{json, registry::Registry, Result};
use crate::structure::template::Nbt;
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) fn component(key: &str, value: &Value) -> Result<Nbt> {
    match key {
        "minecraft:damage" | "minecraft:ominous_bottle_amplifier" => {
            let number = json::int(value, key)?;
            let max = if key.ends_with("amplifier") {
                4
            } else {
                i32::MAX
            };
            if !(0..=max).contains(&number) {
                return Err(json::invalid(key, "component out of range"));
            }
            Ok(Nbt::Int(number))
        }
        "minecraft:enchantments" | "minecraft:stored_enchantments" => {
            let mut result = BTreeMap::new();
            for (enchantment, level) in json::object(value, key)? {
                let id = json::resource(enchantment, key)?;
                let level = json::int(level, key)?;
                if !(1..=255).contains(&level) {
                    return Err(json::invalid(key, "enchantment level out of range"));
                }
                if result.insert(id, Nbt::Int(level)).is_some() {
                    return Err(json::invalid(key, "duplicate normalized enchantment key"));
                }
            }
            Ok(Nbt::Compound(result))
        }
        "minecraft:potion_contents" => {
            // Rich custom potion effects/colors need their own native codec
            // authority; rejecting them prevents lossy preexisting-stack loads.
            json::fields(value, &["potion"], key)?;
            let mut result = BTreeMap::new();
            if let Some(potion) = value.get("potion") {
                result.insert("potion".into(), Nbt::String(json::identifier(potion, key)?));
            }
            Ok(Nbt::Compound(result))
        }
        "minecraft:instrument" => Ok(Nbt::String(json::identifier(value, key)?)),
        "minecraft:suspicious_stew_effects" => {
            let mut values = Vec::new();
            for effect in json::array(value, key)? {
                json::fields(effect, &["id", "duration"], key)?;
                let id = json::identifier(&effect["id"], key)?;
                let mut fields = BTreeMap::from([("id".into(), Nbt::String(id))]);
                if let Some(duration) = effect.get("duration") {
                    let duration = json::int(duration, key)?;
                    if duration != 160 {
                        fields.insert("duration".into(), Nbt::Int(duration));
                    }
                }
                values.push(Nbt::Compound(fields));
            }
            let element_type = if values.is_empty() { 0 } else { 10 };
            Ok(Nbt::List {
                element_type,
                values,
            })
        }
        _ => Err(json::unsupported(key, "persistent component codec")),
    }
}

/// Use the same typed codec for input normalization and persistence, then bind
/// references to the actual catalog. No default registry substitutions occur.
pub(super) fn normalize(registry: &Registry, key: &str, value: &Value) -> Result<(String, Value)> {
    let key = json::resource(key, "component key")?;
    let value = component_json(component(&key, value)?);
    match key.as_str() {
        "minecraft:enchantments" | "minecraft:stored_enchantments" => {
            for id in json::object(&value, &key)?.keys() {
                if !registry.enchantment_names.contains_key(id) {
                    return Err(json::invalid(&key, format!("unknown enchantment {id}")));
                }
            }
        }
        "minecraft:potion_contents" => {
            if let Some(id) = value.get("potion") {
                let id = json::string(id, &key)?;
                if !registry.potions.contains_key(id) {
                    return Err(json::invalid(&key, format!("unknown potion {id}")));
                }
            }
        }
        "minecraft:instrument" => {
            let id = json::string(&value, &key)?;
            if !registry.instrument_names.contains_key(id) {
                return Err(json::invalid(&key, format!("unknown instrument {id}")));
            }
        }
        "minecraft:suspicious_stew_effects" => {
            for effect in json::array(&value, &key)? {
                let id = json::string(&effect["id"], &key)?;
                if super::registry::stew_instantaneous(id).is_none() {
                    return Err(json::unsupported(
                        &key,
                        &format!("stew effect metadata {id}"),
                    ));
                }
            }
        }
        "minecraft:damage" | "minecraft:ominous_bottle_amplifier" => {}
        _ => return Err(json::unsupported(&key, "component registry validation")),
    }
    Ok((key, value))
}

fn component_json(value: Nbt) -> Value {
    match value {
        Nbt::Int(value) => Value::from(value),
        Nbt::String(value) => Value::String(value),
        Nbt::List { values, .. } => Value::Array(values.into_iter().map(component_json).collect()),
        Nbt::Compound(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, component_json(value)))
                .collect(),
        ),
        _ => unreachable!("supported component codecs only produce int/string/list/compound"),
    }
}
