//! The actual proto-chunk entity save produced by generation spawning.
//!
//! WorldGenRegion serializes a finalized mob and discards that live object.
//! Constructor head yaw, sensor timers and RNG state are not in that native save;
//! LOAD creates them anew. Preserve the saved UUID and typed NBT without treating
//! transient constructor state as a world-seeded, persistent entity identity.
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::spawn::{MobKind, SpawnError, SpawnResult, SpawnedMob};
use crate::structure::template::Nbt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GeneratedMob {
    saved_data: Value,
}

impl<'de> Deserialize<'de> for GeneratedMob {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Stored {
            saved_data: Value,
        }
        let data = Stored::deserialize(deserializer)?;
        Self::from_typed_data(data.saved_data).map_err(serde::de::Error::custom)
    }
}

impl GeneratedMob {
    pub fn from_spawn(mob: &SpawnedMob) -> SpawnResult<Self> {
        let nbt = Nbt::from_logical_typed_json(&mob.metadata().native_json())
            .map_err(|e| SpawnError::InvalidSettings(e.to_string()))?;
        Self::from_typed_data(nbt.typed_json())
    }

    pub fn from_typed_data(saved_data: Value) -> SpawnResult<Self> {
        let invalid = || SpawnError::InvalidSettings("generated mob save".into());
        if serde_json::to_vec(&saved_data)
            .map_err(|_| invalid())?
            .len()
            > 1024 * 1024
        {
            return Err(invalid());
        }
        let nbt = Nbt::from_typed_json(&saved_data).map_err(|_| invalid())?;
        let kind = MobKind::from_name(nbt.get("id").and_then(Nbt::string).ok_or_else(invalid)?)?;
        if !kind.supports_finalization() {
            return Err(SpawnError::Unsupported(format!(
                "generated {}",
                kind.name()
            )));
        }
        let position = nbt.get("Pos").and_then(Nbt::list).ok_or_else(invalid)?;
        let rotation = nbt
            .get("Rotation")
            .and_then(Nbt::list)
            .ok_or_else(invalid)?;
        let motion = nbt.get("Motion").and_then(Nbt::list).ok_or_else(invalid)?;
        if position.len() != 3 || rotation.len() != 2 || motion.len() != 3 {
            return Err(invalid());
        }
        for value in position.iter().chain(motion) {
            if !matches!(value, Nbt::Double(v) if v.is_finite()) {
                return Err(invalid());
            }
        }
        for (index, value) in position.iter().enumerate() {
            let value = value.number().unwrap();
            let bound = if index == 1 {
                20_000_000.0
            } else {
                f64::from(i32::MAX) - 32.0
            };
            if value.abs() > bound {
                return Err(invalid());
            }
        }
        if !rotation
            .iter()
            .all(|v| matches!(v, Nbt::Float(f) if f.is_finite()))
            || !matches!(nbt.get("UUID"), Some(Nbt::IntArray(values)) if values.len() == 4)
            || !matches!(nbt.get("Age"), Some(Nbt::Int(_)))
            || !matches!(nbt.get("Health"), Some(Nbt::Float(v)) if *v >= 0.0 && v.is_finite())
            || !matches!(nbt.get("LeftHanded"), Some(Nbt::Byte(0 | 1)))
        {
            return Err(invalid());
        }
        Ok(Self { saved_data })
    }

    pub fn typed_data(&self) -> &Value {
        &self.saved_data
    }

    pub fn nbt(&self) -> Nbt {
        Nbt::from_typed_json(&self.saved_data).expect("validated generated mob NBT")
    }

    pub fn data(&self) -> Value {
        self.nbt().to_json()
    }

    pub fn kind(&self) -> MobKind {
        MobKind::from_name(self.saved_data[1]["id"][1].as_str().unwrap())
            .expect("validated generated mob kind")
    }

    pub fn position(&self) -> [f64; 3] {
        std::array::from_fn(|i| {
            self.saved_data[1]["Pos"][1]["values"][i][1]
                .as_f64()
                .unwrap()
        })
    }

    /// Saved body yaw/pitch. LOAD restores the head through each kind's setter;
    /// in particular, Goat clamps it before the body yaw is restored.
    pub fn rotation(&self) -> [f32; 2] {
        std::array::from_fn(|i| {
            self.saved_data[1]["Rotation"][1]["values"][i][1]
                .as_f64()
                .unwrap() as f32
        })
    }

    pub fn uuid(&self) -> [u8; 16] {
        let values = self.saved_data[1]["UUID"][1].as_array().unwrap();
        let mut uuid = [0; 16];
        for (bytes, value) in uuid.chunks_exact_mut(4).zip(values) {
            bytes.copy_from_slice(&(value.as_i64().unwrap() as i32).to_be_bytes());
        }
        uuid
    }
}
