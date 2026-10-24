use super::{json, location::LocationCheck, rng, LootContext, LootRng, Result};
use serde_json::Value;

#[derive(Clone, Debug)]
pub(super) enum Number {
    Constant(f32),
    Uniform(Box<Number>, Box<Number>),
}

impl Number {
    pub fn compile(value: &Value, path: &str) -> Result<Self> {
        if value.is_number() {
            return Ok(Self::Constant(json::float(value, path)?));
        }
        json::object(value, path)?;
        match json::identifier(&value["type"], path)?.as_str() {
            "minecraft:constant" => {
                json::fields(value, &["type", "value"], path)?;
                Ok(Self::Constant(json::float(&value["value"], path)?))
            }
            "minecraft:uniform" => {
                json::fields(value, &["type", "min", "max"], path)?;
                Ok(Self::Uniform(
                    Box::new(Self::compile(&value["min"], &format!("{path}.min"))?),
                    Box::new(Self::compile(&value["max"], &format!("{path}.max"))?),
                ))
            }
            kind => Err(json::unsupported(path, kind)),
        }
    }

    pub fn int(&self, rng: &mut dyn LootRng) -> Result<i32> {
        match self {
            Self::Constant(value) => Ok(floor(*value)),
            Self::Uniform(min, max) => {
                let min = min.int(rng)?;
                let max = max.int(rng)?;
                rng::between(rng, min, max)
            }
        }
    }

    pub fn float(&self, rng: &mut dyn LootRng) -> f32 {
        match self {
            Self::Constant(value) => *value,
            Self::Uniform(min, max) => {
                let min = min.float(rng);
                let max = max.float(rng);
                if min >= max {
                    min
                } else {
                    rng.next_float() * (max - min) + min
                }
            }
        }
    }

    pub fn is_constant(&self) -> bool {
        matches!(self, Self::Constant(_))
    }
}

/// `Mth.floor(float)`, including Java's saturating float-to-int conversion.
pub(super) fn floor(value: f32) -> i32 {
    let integer = value as i32;
    if value < integer as f32 {
        integer.wrapping_sub(1)
    } else {
        integer
    }
}

#[derive(Clone, Debug)]
pub(super) enum Condition {
    RandomChance(Number),
    All(Vec<Condition>),
    Any(Vec<Condition>),
    Inverted(Box<Condition>),
    Location(LocationCheck),
}

impl Condition {
    pub fn compile(value: &Value, path: &str) -> Result<Self> {
        match json::identifier(&value["condition"], path)?.as_str() {
            "minecraft:random_chance" => {
                json::fields(value, &["condition", "chance"], path)?;
                Ok(Self::RandomChance(Number::compile(&value["chance"], path)?))
            }
            "minecraft:all_of" | "minecraft:any_of" => {
                json::fields(value, &["condition", "terms"], path)?;
                let terms = json::array(&value["terms"], path)?
                    .iter()
                    .enumerate()
                    .map(|(i, v)| Self::compile(v, &format!("{path}.terms[{i}]")))
                    .collect::<Result<_>>()?;
                if value["condition"].as_str().unwrap().ends_with("all_of") {
                    Ok(Self::All(terms))
                } else {
                    Ok(Self::Any(terms))
                }
            }
            "minecraft:inverted" => {
                json::fields(value, &["condition", "term"], path)?;
                Ok(Self::Inverted(Box::new(Self::compile(
                    &value["term"],
                    path,
                )?)))
            }
            "minecraft:location_check" => Ok(Self::Location(LocationCheck::compile(value, path)?)),
            kind => Err(json::unsupported(path, kind)),
        }
    }

    pub fn needs_location(&self) -> bool {
        match self {
            Self::Location(_) => true,
            Self::All(terms) | Self::Any(terms) => terms.iter().any(Self::needs_location),
            Self::Inverted(term) => term.needs_location(),
            Self::RandomChance(_) => false,
        }
    }

    pub fn test(&self, context: LootContext<'_>, rng: &mut dyn LootRng) -> Result<bool> {
        Ok(match self {
            Self::RandomChance(chance) => {
                // The native test evaluates the provider BEFORE nextFloat.
                let threshold = chance.float(rng);
                rng.next_float() < threshold
            }
            Self::All(terms) => test_conditions(terms, context, rng)?,
            Self::Any(terms) => {
                for term in terms {
                    if term.test(context, rng)? {
                        return Ok(true);
                    }
                }
                false
            }
            Self::Inverted(term) => !term.test(context, rng)?,
            Self::Location(condition) => condition.test(context)?,
        })
    }
}

pub(super) fn test_conditions(
    conditions: &[Condition],
    context: LootContext<'_>,
    rng: &mut dyn LootRng,
) -> Result<bool> {
    for condition in conditions {
        if !condition.test(context, rng)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn conditions(value: &Value, path: &str) -> Result<Vec<Condition>> {
    json::optional_array(value, "conditions", path)?
        .iter()
        .enumerate()
        .map(|(i, v)| Condition::compile(v, &format!("{path}.conditions[{i}]")))
        .collect()
}
