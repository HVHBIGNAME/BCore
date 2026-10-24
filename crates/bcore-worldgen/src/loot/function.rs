use super::{
    enchant, json,
    number::{self, Condition, Number},
    registry::{holders, Registry},
    ItemStack, LootContext, LootRng, Result,
};
use serde_json::{json, Value};

#[derive(Debug)]
pub(super) struct Function {
    conditions: Vec<Condition>,
    kind: Kind,
}

#[derive(Debug)]
enum Kind {
    Count {
        count: Number,
        add: bool,
    },
    Damage {
        damage: Number,
        add: bool,
    },
    EnchantRandomly {
        options: Vec<usize>,
        only_compatible: bool,
    },
    EnchantLevels {
        options: Vec<usize>,
        levels: Number,
    },
    Enchantments {
        entries: Vec<(usize, Number)>,
        add: bool,
    },
    Potion(String),
    Instrument(Vec<usize>),
    Stew(Vec<StewEffect>),
    Ominous(Number),
}

#[derive(Debug)]
struct StewEffect {
    id: String,
    duration: Number,
    instantaneous: bool,
}

impl Function {
    fn compile(value: &Value, registry: &Registry, path: &str) -> Result<Self> {
        let name = json::identifier(&value["function"], path)?;
        let allowed: &[&str] = match name.as_str() {
            "minecraft:set_count" => &["count", "add"],
            "minecraft:set_damage" => &["damage", "add"],
            "minecraft:enchant_randomly" => &[
                "options",
                "only_compatible",
                "include_additional_cost_component",
            ],
            "minecraft:enchant_with_levels" => {
                &["options", "levels", "include_additional_cost_component"]
            }
            "minecraft:set_enchantments" => &["enchantments", "add"],
            "minecraft:set_potion" => &["id"],
            "minecraft:set_instrument" => &["options"],
            "minecraft:set_stew_effect" => &["effects"],
            "minecraft:set_ominous_bottle_amplifier" => &["amplifier"],
            _ => return Err(json::unsupported(path, &name)),
        };
        let mut allowed = allowed.to_vec();
        allowed.extend(["function", "conditions"]);
        json::fields(value, &allowed, path)?;
        if json::boolean(value, "include_additional_cost_component", false, path)? {
            return Err(json::unsupported(
                path,
                "additional trade-cost component context",
            ));
        }
        let add = json::boolean(value, "add", false, path)?;
        let kind = match name.as_str() {
            "minecraft:set_count" => Kind::Count {
                count: Number::compile(&value["count"], path)?,
                add,
            },
            "minecraft:set_damage" => Kind::Damage {
                damage: Number::compile(&value["damage"], path)?,
                add,
            },
            "minecraft:enchant_randomly" => Kind::EnchantRandomly {
                options: registry.enchantment_options(value.get("options"), path)?,
                only_compatible: json::boolean(value, "only_compatible", true, path)?,
            },
            "minecraft:enchant_with_levels" => Kind::EnchantLevels {
                options: registry.enchantment_options(value.get("options"), path)?,
                levels: Number::compile(&value["levels"], path)?,
            },
            "minecraft:set_enchantments" => compile_enchantments(value, registry, add, path)?,
            "minecraft:set_potion" => {
                let id = json::identifier(&value["id"], path)?;
                if !registry.potions.contains_key(&id) {
                    return Err(json::invalid(path, format!("unknown potion {id}")));
                }
                Kind::Potion(id)
            }
            "minecraft:set_instrument" => Kind::Instrument(holders(
                &value["options"],
                &registry.instrument_names,
                &registry.instrument_tags,
                path,
            )?),
            "minecraft:set_stew_effect" => Kind::Stew(compile_stew(value, path)?),
            "minecraft:set_ominous_bottle_amplifier" => {
                Kind::Ominous(Number::compile(&value["amplifier"], path)?)
            }
            _ => unreachable!("validated function kind"),
        };
        Ok(Self {
            conditions: number::conditions(value, path)?,
            kind,
        })
    }

    pub fn apply(
        &self,
        stack: &mut ItemStack,
        registry: &Registry,
        context: LootContext<'_>,
        rng: &mut dyn LootRng,
    ) -> Result<()> {
        if !number::test_conditions(&self.conditions, context, rng)? {
            return Ok(());
        }
        match &self.kind {
            Kind::Count { count, add } => {
                let base = if *add { stack.count() } else { 0 };
                stack.count = base.wrapping_add(count.int(rng)?);
            }
            Kind::Damage { damage, add } => set_damage(stack, damage, *add, rng),
            Kind::EnchantRandomly {
                options,
                only_compatible,
            } => enchant::randomly(stack, options, *only_compatible, registry, rng)?,
            Kind::EnchantLevels { options, levels } => {
                let cost = levels.int(rng)?;
                enchant::with_levels(stack, cost, options, registry, rng)?;
            }
            Kind::Enchantments { entries, add } => {
                if stack.id() == "minecraft:book" {
                    stack.transmute(registry.enchanted_book.clone(), false);
                }
                let component = if stack.id() == "minecraft:enchanted_book" {
                    "minecraft:stored_enchantments"
                } else {
                    "minecraft:enchantments"
                };
                // EnchantmentHelper.updateEnchantments skips its consumer (and
                // therefore its number providers) when the component is absent.
                if stack.component(component).is_none() {
                    return Ok(());
                }
                for (index, level) in entries {
                    let key = &registry.enchantments[*index].key;
                    let base = if *add {
                        stack
                            .component(component)
                            .and_then(|v| v.get(key))
                            .and_then(Value::as_i64)
                            .unwrap_or(0) as i32
                    } else {
                        0
                    };
                    enchant::apply(
                        stack,
                        key,
                        base.wrapping_add(level.int(rng)?).clamp(0, 255),
                        false,
                    );
                }
            }
            Kind::Potion(potion) => {
                let mut contents = stack.component_object("minecraft:potion_contents");
                contents.insert("potion".into(), Value::String(potion.clone()));
                stack.set("minecraft:potion_contents", Value::Object(contents));
            }
            Kind::Instrument(options) => {
                if !options.is_empty() {
                    let index = options[rng.next_int(options.len() as i32) as usize];
                    stack.set(
                        "minecraft:instrument",
                        Value::String(registry.instruments[index].clone()),
                    );
                }
            }
            Kind::Stew(effects) => set_stew(stack, effects, rng)?,
            Kind::Ominous(amplifier) => stack.set(
                "minecraft:ominous_bottle_amplifier",
                Value::from(amplifier.int(rng)?.clamp(0, 4)),
            ),
        }
        Ok(())
    }

    pub fn needs_location(&self) -> bool {
        self.conditions.iter().any(Condition::needs_location)
    }
}

pub(super) fn compile(value: &Value, registry: &Registry, path: &str) -> Result<Vec<Function>> {
    json::optional_array(value, "functions", path)?
        .iter()
        .enumerate()
        .map(|(i, v)| Function::compile(v, registry, &format!("{path}.functions[{i}]")))
        .collect()
}

fn compile_enchantments(value: &Value, registry: &Registry, add: bool, path: &str) -> Result<Kind> {
    let mut entries = Vec::new();
    if let Some(values) = value.get("enchantments") {
        for (key, number) in json::object(values, path)? {
            let key = json::resource(key, path)?;
            let index = *registry
                .enchantment_names
                .get(&key)
                .ok_or_else(|| json::invalid(path, format!("unknown enchantment {key}")))?;
            entries.push((index, Number::compile(number, path)?));
        }
    }
    // Native Map.copyOf holder-key iteration is not JSON order. The delivered
    // authority has singleton maps; multiple deterministic values commute.
    if entries.len() > 1 && entries.iter().any(|(_, number)| !number.is_constant()) {
        return Err(json::unsupported(
            path,
            "multi-enchantment random provider map iteration",
        ));
    }
    Ok(Kind::Enchantments { entries, add })
}

fn compile_stew(value: &Value, path: &str) -> Result<Vec<StewEffect>> {
    json::array(&value["effects"], path)?
        .iter()
        .map(|effect| {
            json::fields(effect, &["type", "duration"], path)?;
            let id = json::identifier(&effect["type"], path)?;
            let instantaneous = super::registry::stew_instantaneous(&id)
                .ok_or_else(|| json::unsupported(path, &format!("stew effect metadata {id}")))?;
            Ok(StewEffect {
                id,
                duration: Number::compile(&effect["duration"], path)?,
                instantaneous,
            })
        })
        .collect()
}

fn set_damage(stack: &mut ItemStack, damage: &Number, add: bool, rng: &mut dyn LootRng) {
    let Some(max) = stack
        .component_int("minecraft:max_damage")
        .filter(|v| *v > 0)
    else {
        return;
    };
    if stack.component("minecraft:unbreakable").is_some()
        || stack.component("minecraft:damage").is_none()
    {
        return;
    }
    let current = stack
        .component_int("minecraft:damage")
        .unwrap_or(0)
        .clamp(0, max);
    let base = if add {
        1.0_f32 - current as f32 / max as f32
    } else {
        0.0
    };
    let remaining = (damage.float(rng) + base).clamp(0.0, 1.0);
    let amount = number::floor((1.0_f32 - remaining) * max as f32);
    stack.set("minecraft:damage", Value::from(amount));
}

fn set_stew(stack: &mut ItemStack, effects: &[StewEffect], rng: &mut dyn LootRng) -> Result<()> {
    if stack.id() != "minecraft:suspicious_stew" || effects.is_empty() {
        return Ok(());
    }
    let effect = &effects[rng.next_int(effects.len() as i32) as usize];
    let mut duration = effect.duration.int(rng)?;
    if !effect.instantaneous {
        duration = duration.wrapping_mul(20);
    }
    let mut values = stack
        .component("minecraft:suspicious_stew_effects")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut value = json!({"id": effect.id});
    if duration != 160 {
        value["duration"] = Value::from(duration);
    }
    values.push(value);
    stack.set("minecraft:suspicious_stew_effects", Value::Array(values));
    Ok(())
}
