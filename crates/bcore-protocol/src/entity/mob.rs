//! LOAD and initial pairing for freshly generated 26.1 creatures.
//!
//! Registry IDs, serializer definitions and defaults are original native data.
//! Per-entity values are derived from the saved proto NBT, never recorded packets.
use std::collections::BTreeMap;
use std::sync::OnceLock;

use bcore_core::varint::{encode_varint, encode_varlong};
use bcore_worldgen::generated_entity::GeneratedMob;
use bcore_worldgen::spawn::{spawn_type, MobKind};
use bcore_worldgen::structure::template::Nbt;
use serde::Deserialize;

use super::{
    encode_entity_metadata, encode_spawn_entity_rotated, MetadataEntry, Position,
    CB_ENTITY_ATTRIBUTES, CB_ENTITY_EQUIPMENT,
};
use crate::packet::{write_packet, write_string, PacketError};

type Result<T> = std::result::Result<T, PacketError>;

#[derive(Deserialize)]
struct Definition {
    name: String,
    index: u8,
    serializer: i32,
    hex: String,
    default_hex: String,
}

#[derive(Deserialize)]
struct AttributeDefault {
    base: f64,
    syncable: bool,
}

#[derive(Deserialize)]
struct Catalog {
    metadata: BTreeMap<String, Vec<Definition>>,
    default_attributes: BTreeMap<String, BTreeMap<String, AttributeDefault>>,
    registries: BTreeMap<String, BTreeMap<String, i32>>,
}

fn catalog() -> &'static Catalog {
    static DATA: OnceLock<Catalog> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../../data/generation_mob_catalog_26_1.json"))
            .expect("native generation-mob registry catalog")
    })
}

fn hex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn invalid(field: &'static str) -> PacketError {
    PacketError::Malformed(field)
}
fn integer(nbt: &Nbt, key: &str, default: i32) -> Result<i32> {
    nbt.get(key).map_or(Ok(default), |v| {
        v.int().ok_or_else(|| invalid("mob integer"))
    })
}
fn boolean(nbt: &Nbt, key: &str, default: bool) -> Result<bool> {
    Ok(integer(nbt, key, i32::from(default))? != 0)
}
fn string<'a>(nbt: &'a Nbt, key: &str, default: &'a str) -> Result<&'a str> {
    nbt.get(key).map_or(Ok(default), |v| {
        v.string().ok_or_else(|| invalid("mob string"))
    })
}
fn number(nbt: &Nbt, key: &str, default: f64) -> Result<f64> {
    nbt.get(key).map_or(Ok(default), |v| {
        v.number()
            .filter(|v| v.is_finite())
            .ok_or_else(|| invalid("mob number"))
    })
}
fn varint(value: i32) -> Vec<u8> {
    let mut out = Vec::new();
    encode_varint(value, &mut out);
    out
}
fn registry_id(registry: &str, name: &str) -> Result<i32> {
    catalog()
        .registries
        .get(registry)
        .and_then(|r| r.get(name))
        .copied()
        .ok_or_else(|| invalid("unregistered generation-mob value"))
}

pub(super) fn loaded_head_yaw(kind: MobKind, yaw: f32) -> f32 {
    if kind != MobKind::Goat {
        return yaw;
    }
    // Entity.load calls setYHeadRot BEFORE setYBodyRot. Goat overrides the
    // former and clamps relative to the constructor's still-zero body yaw.
    let mut difference = yaw % 360.0;
    if difference >= 180.0 {
        difference -= 360.0;
    }
    if difference < -180.0 {
        difference += 360.0;
    }
    difference.clamp(-15.0, 15.0)
}

#[derive(Debug, Clone)]
struct Modifier {
    name: String,
    amount: f64,
    operation: u8,
}

#[derive(Debug, Clone)]
struct Attribute {
    base: f64,
    modifiers: Vec<Modifier>,
}

impl Attribute {
    fn from_nbt(nbt: &Nbt) -> Result<Self> {
        let mut modifiers = Vec::new();
        if let Some(list) = nbt.get("modifiers") {
            for modifier in list.list().ok_or_else(|| invalid("attribute modifiers"))? {
                let name = string(modifier, "id", "")?.to_owned();
                let operation = match string(modifier, "operation", "")? {
                    "add_value" => 0,
                    "add_multiplied_base" => 1,
                    "add_multiplied_total" => 2,
                    _ => return Err(invalid("attribute modifier operation")),
                };
                if name.is_empty() || modifiers.iter().any(|m: &Modifier| m.name == name) {
                    return Err(invalid("attribute modifier identity"));
                }
                modifiers.push(Modifier {
                    name,
                    amount: number(modifier, "amount", 0.0)?,
                    operation,
                });
            }
        }
        Ok(Self {
            base: number(nbt, "base", 0.0)?,
            modifiers,
        })
    }

    fn value(&self) -> f64 {
        let base = self
            .modifiers
            .iter()
            .filter(|m| m.operation == 0)
            .fold(self.base, |value, m| value + m.amount);
        let value = self
            .modifiers
            .iter()
            .filter(|m| m.operation == 1)
            .fold(base, |value, m| value + base * m.amount);
        self.modifiers
            .iter()
            .filter(|m| m.operation == 2)
            .fold(value, |value, m| value * (1.0 + m.amount))
    }

    fn encode(&self, name: &str, out: &mut Vec<u8>) -> Result<()> {
        encode_varint(registry_id("attribute", name)?, out);
        out.extend_from_slice(&self.base.to_be_bytes());
        encode_varint(self.modifiers.len() as i32, out);
        for m in &self.modifiers {
            write_string(&m.name, out);
            out.extend_from_slice(&m.amount.to_be_bytes());
            out.push(m.operation);
        }
        Ok(())
    }
}

/// Loaded initial state. The original proto save remains untouched; notably,
/// loading may clamp health and initialize an attribute absent from that save.
#[derive(Debug, Clone)]
pub struct LoadedGeneratedMob {
    source: GeneratedMob,
    nbt: Nbt,
    attributes: BTreeMap<String, Attribute>,
}

impl LoadedGeneratedMob {
    pub fn load(source: &GeneratedMob) -> Result<Self> {
        let mut nbt = source.nbt();
        let defaults = &catalog().default_attributes[source.kind().name()];
        let mut attributes = BTreeMap::new();
        if let Some(list) = nbt.get("attributes") {
            for row in list.list().ok_or_else(|| invalid("mob attributes"))? {
                let name = string(row, "id", "")?;
                if !defaults.contains_key(name) || attributes.contains_key(name) {
                    return Err(invalid("mob attribute identity"));
                }
                attributes.insert(name.to_owned(), Attribute::from_nbt(row)?);
            }
        }
        let max_health = attributes
            .get("minecraft:max_health")
            .map_or(defaults["minecraft:max_health"].base, Attribute::value)
            .clamp(1.0, 1024.0) as f32;
        let health = (number(&nbt, "Health", f64::from(max_health))? as f32).clamp(0.0, max_health);
        nbt.compound_mut()
            .unwrap()
            .insert("Health".into(), Nbt::Float(health));

        // TamableAnimal.readAdditionalSaveData invokes Wolf's taming side effects
        // even for a wild wolf. getAttribute creates a syncable max_health instance.
        if source.kind() == MobKind::Wolf {
            if nbt.get("Owner").is_some() {
                return Err(invalid("tamed wolf is not a fresh generation mob"));
            }
            let attribute = attributes
                .entry("minecraft:max_health".into())
                .or_insert_with(|| Attribute {
                    base: 8.0,
                    modifiers: Vec::new(),
                });
            attribute.base = 8.0;
            let fields = nbt.compound_mut().unwrap();
            let list = fields
                .entry("attributes".into())
                .or_insert_with(|| Nbt::List {
                    element_type: 10,
                    values: Vec::new(),
                });
            let Nbt::List { values, .. } = list else {
                return Err(invalid("wolf attributes"));
            };
            if let Some(value) = values
                .iter_mut()
                .find(|v| v.get("id").and_then(Nbt::string) == Some("minecraft:max_health"))
            {
                value
                    .compound_mut()
                    .unwrap()
                    .insert("base".into(), Nbt::Double(8.0));
            } else {
                values.push(Nbt::Compound(BTreeMap::from([
                    ("id".into(), Nbt::String("minecraft:max_health".into())),
                    ("base".into(), Nbt::Double(8.0)),
                ])));
            }
        }
        Ok(Self {
            source: source.clone(),
            nbt,
            attributes,
        })
    }

    pub fn nbt(&self) -> &Nbt {
        &self.nbt
    }

    /// Spawn -> nondefault metadata -> instantiated syncable attributes ->
    /// nonempty equipment, the native ServerEntity.sendPairingData order.
    pub fn pairing_packets(&self, id: i32) -> Result<Vec<u8>> {
        let [x, y, z] = self.source.position();
        let rotation = self.source.rotation();
        let mut out = encode_spawn_entity_rotated(
            id,
            self.source.uuid(),
            spawn_type(self.source.kind()).registry_id as i32,
            Position { x, y, z },
            rotation,
            loaded_head_yaw(self.source.kind(), rotation[0]),
        );
        let metadata = self.metadata()?;
        if !metadata.is_empty() {
            out.extend_from_slice(&encode_entity_metadata(id, &metadata));
        }
        let defaults = &catalog().default_attributes[self.source.kind().name()];
        let sync: Vec<_> = self
            .attributes
            .iter()
            .filter(|(name, _)| defaults[*name].syncable)
            .collect();
        if !sync.is_empty() {
            let mut bytes = varint(id);
            encode_varint(sync.len() as i32, &mut bytes);
            // Java's AttributeMap uses identity-keyed iteration; record order is
            // not stable across JVMs. Each attribute's native codec is unchanged.
            for (name, value) in sync {
                value.encode(name, &mut bytes)?;
            }
            write_packet(&mut out, CB_ENTITY_ATTRIBUTES, &bytes);
        }
        if let Some(bytes) = self.equipment(id)? {
            write_packet(&mut out, CB_ENTITY_EQUIPMENT, &bytes);
        }
        Ok(out)
    }

    fn metadata(&self) -> Result<Vec<MetadataEntry>> {
        let mut entries = Vec::new();
        for field in &catalog().metadata[self.source.kind().name()] {
            let value = self.metadata_value(field)?;
            if value != hex(&field.default_hex) {
                entries.push(MetadataEntry {
                    index: field.index,
                    type_id: field.serializer,
                    value,
                });
            }
        }
        Ok(entries)
    }

    fn metadata_value(&self, field: &Definition) -> Result<Vec<u8>> {
        let n = &self.nbt;
        let kind = self.source.kind();
        let bit = |key, bit| -> Result<u8> { Ok(if boolean(n, key, false)? { bit } else { 0 }) };
        let registry = |key: &str, suffix: &str| -> Result<Vec<u8>> {
            let reg = format!(
                "{}_{}",
                kind.name().strip_prefix("minecraft:").unwrap(),
                suffix
            );
            match n.get(key) {
                Some(v) => Ok(varint(registry_id(
                    &reg,
                    v.string().ok_or_else(|| invalid("mob variant"))?,
                )?)),
                None => Ok(hex(&field.hex)),
            }
        };
        let value = match field.name.as_str() {
            "DATA_HEALTH_ID" => (number(n, "Health", 1.0)? as f32).to_be_bytes().to_vec(),
            "DATA_AIR_SUPPLY_ID" => varint(integer(n, "Air", 300)?),
            "DATA_SILENT" => vec![u8::from(boolean(n, "Silent", false)?)],
            "DATA_NO_GRAVITY" => vec![u8::from(boolean(n, "NoGravity", false)?)],
            "DATA_MOB_FLAGS_ID" => vec![bit("NoAI", 1)? | bit("LeftHanded", 2)?],
            // AgeableMob.load writes this flag even on Frog, whose isBaby()
            // override itself always returns false. Use saved age, not that method.
            "DATA_BABY_ID" => vec![u8::from(integer(n, "Age", 0)? < 0)],
            "AGE_LOCKED" => vec![u8::from(boolean(n, "AgeLocked", false)?)],
            "DATA_WOOL_ID" => vec![(integer(n, "Color", 0)? as u8 & 15) | bit("Sheared", 16)?],
            "DATA_VARIANT_ID"
                if matches!(
                    kind,
                    MobKind::Cow | MobKind::Pig | MobKind::Chicken | MobKind::Wolf | MobKind::Frog
                ) =>
            {
                registry("variant", "variant")?
            }
            "DATA_SOUND_VARIANT_ID" => registry("sound_variant", "sound_variant")?,
            "DATA_VARIANT_ID" => varint(integer(n, "Variant", 0)?),
            "DATA_ID_TYPE_VARIANT" => varint(integer(n, "Variant", 0)?),
            "DATA_TYPE_ID" if kind == MobKind::Rabbit => varint(integer(n, "RabbitType", 0)?),
            "DATA_TYPE_ID" if kind == MobKind::Fox => {
                varint(i32::from(string(n, "Type", "red")? == "snow"))
            }
            "DATA_TYPE" if kind == MobKind::Mooshroom => {
                varint(i32::from(string(n, "Type", "red")? == "brown"))
            }
            "DATA_ID_FLAGS"
                if matches!(
                    kind,
                    MobKind::Horse | MobKind::Donkey | MobKind::Llama | MobKind::Camel
                ) =>
            {
                vec![bit("Tame", 2)? | bit("Bred", 8)? | bit("EatingHaystack", 16)?]
            }
            "DATA_ID_CHEST" => vec![u8::from(boolean(n, "ChestedHorse", false)?)],
            "DATA_STRENGTH_ID" => varint(integer(n, "Strength", 0)?.clamp(1, 5)),
            "HAS_EGG" => vec![u8::from(boolean(n, "HasEgg", false)?)],
            "DATA_COLLAR_COLOR" => varint(integer(n, "CollarColor", 14)?),
            "DATA_ANGER_END_TIME" => {
                let time = n
                    .get("anger_end_time")
                    .and_then(|v| if let Nbt::Long(t) = v { Some(*t) } else { None })
                    .unwrap_or(-1);
                let mut bytes = Vec::new();
                encode_varlong(time, &mut bytes);
                bytes
            }
            "MAIN_GENE_ID" | "HIDDEN_GENE_ID" => {
                let key = if field.name == "MAIN_GENE_ID" {
                    "MainGene"
                } else {
                    "HiddenGene"
                };
                vec![match string(n, key, "normal")? {
                    "normal" => 0,
                    "lazy" => 1,
                    "worried" => 2,
                    "playful" => 3,
                    "brown" => 4,
                    "weak" => 5,
                    "aggressive" => 6,
                    _ => return Err(invalid("panda gene")),
                }]
            }
            "LAST_POSE_CHANGE_TICK" => {
                let time = match n.get("LastPoseTick") {
                    Some(Nbt::Long(t)) => *t,
                    None => 0,
                    _ => return Err(invalid("camel pose time")),
                };
                let mut bytes = Vec::new();
                encode_varlong(time, &mut bytes);
                bytes
            }
            "DATA_IS_SCREAMING_GOAT" => vec![u8::from(boolean(n, "IsScreamingGoat", false)?)],
            "DATA_HAS_LEFT_HORN" => vec![u8::from(boolean(n, "HasLeftHorn", true)?)],
            "DATA_HAS_RIGHT_HORN" => vec![u8::from(boolean(n, "HasRightHorn", true)?)],
            _ => hex(&field.hex),
        };
        Ok(value)
    }

    fn equipment(&self, id: i32) -> Result<Option<Vec<u8>>> {
        let Some(equipment) = self.nbt.get("equipment") else {
            return Ok(None);
        };
        let fields = equipment.compound().map_err(|_| invalid("mob equipment"))?;
        let mut items = Vec::new();
        for (slot, key) in [
            "mainhand", "offhand", "feet", "legs", "chest", "head", "body", "saddle",
        ]
        .into_iter()
        .enumerate()
        {
            let Some(item) = fields.get(key) else {
                continue;
            };
            let count = integer(item, "count", 1)?;
            if count <= 0 {
                continue;
            }
            let name = string(item, "id", "minecraft:air")?;
            if name == "minecraft:air" {
                continue;
            }
            if let Some(components) = item.get("components") {
                if !components.compound().is_ok_and(BTreeMap::is_empty) {
                    return Err(invalid("generation equipment components are unsupported"));
                }
            }
            let mut bytes = Vec::new();
            encode_varint(count, &mut bytes);
            encode_varint(registry_id("item", name)?, &mut bytes);
            encode_varint(0, &mut bytes); // added data components
            encode_varint(0, &mut bytes); // removed data components
            items.push((slot as u8, bytes));
        }
        if items.is_empty() {
            return Ok(None);
        }
        let mut bytes = varint(id);
        for (index, (slot, item)) in items.iter().enumerate() {
            bytes.push(*slot | if index + 1 < items.len() { 0x80 } else { 0 });
            bytes.extend_from_slice(item);
        }
        Ok(Some(bytes))
    }
}
