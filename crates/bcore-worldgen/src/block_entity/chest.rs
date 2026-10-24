//! Single-chest backing slots and the native deferred-loot state transition.
use crate::loot::{
    self, FillOutcome, ItemStack, LootContext, LootEngine, LootError, LootRng, LootTable,
};
use crate::structure::template::Nbt;
use std::collections::BTreeMap;

/// A 27-slot chest inventory. The canonical owner supplies the chest's base NBT,
/// validates its single-chest block state, and owns synchronization/persistence.
#[derive(Debug, Clone, PartialEq)]
pub struct ChestInventory {
    slots: [Option<ItemStack>; 27],
    loot_table: Option<String>,
    loot_seed: i64,
}

impl ChestInventory {
    /// Read a chest compound or metadata-free chest payload. A present `id` must
    /// be `minecraft:chest`. Pending loot skips `Items`, as native chest load does.
    /// The seed is loaded independently of the key, defaulting to zero if absent.
    /// Saved items must round-trip through the supported codec with exact NBT types.
    pub fn from_nbt(engine: &LootEngine, tag: &Nbt) -> loot::Result<Self> {
        let fields = tag
            .compound()
            .map_err(|_| invalid("chest", "expected compound"))?;
        validate_id(fields)?;
        let loot_table = match fields.get("LootTable") {
            None => None,
            Some(Nbt::String(key)) => Some(table_key(key)?),
            Some(_) => return Err(invalid("chest.LootTable", "expected String tag")),
        };
        let loot_seed = match fields.get("LootTableSeed") {
            None => 0,
            Some(Nbt::Long(seed)) => *seed,
            Some(_) => return Err(invalid("chest.LootTableSeed", "expected Long tag")),
        };
        let slots = if loot_table.is_some() {
            std::array::from_fn(|_| None)
        } else {
            read_items(engine, fields.get("Items"))?
        };
        Ok(Self {
            slots,
            loot_table,
            loot_seed,
        })
    }

    /// Construct runtime backing state, including items installed before a loot
    /// key was set. Loading deferred saved NBT cannot reconstruct those items.
    /// Every occupied slot must be persistent and belong to the supplied catalog.
    /// A retained seed without a key is permitted but will not be saved.
    pub fn with_slots(
        engine: &LootEngine,
        slots: [Option<ItemStack>; 27],
        loot_table: Option<&str>,
        loot_seed: i64,
    ) -> loot::Result<Self> {
        let loot_table = loot_table.map(table_key).transpose()?;
        for (slot, stack) in slots.iter().enumerate() {
            if let Some(stack) = stack {
                let path = format!("chest.slots[{slot}]");
                let admitted = read_item(engine, &stack.to_nbt()?, &path)?;
                if &admitted != stack {
                    return Err(invalid(
                        &path,
                        "stack differs from the supplied item catalog",
                    ));
                }
            }
        }
        let mut inventory = Self {
            slots,
            loot_table,
            loot_seed: 0,
        };
        inventory.restore_retained_seed(loot_seed);
        Ok(inventory)
    }

    pub fn slots(&self) -> &[Option<ItemStack>; 27] {
        &self.slots
    }

    pub fn loot_table(&self) -> Option<&str> {
        self.loot_table.as_deref()
    }

    pub fn loot_seed(&self) -> i64 {
        self.loot_seed
    }

    /// Reattach the owner's runtime seed after reading materialized saved NBT,
    /// whose native representation intentionally omits the retained seed field.
    pub(crate) fn restore_retained_seed(&mut self, seed: i64) {
        self.loot_seed = seed;
    }

    /// Unpack with the stored nonzero seed. No key means an unconditional no-op.
    /// Resolution/source errors preserve the key. Once fill starts, an error
    /// leaves the key consumed and the seed unchanged; the owner must commit that
    /// state even on error, rather than implicitly retrying the deferred table.
    pub fn unpack(&mut self, engine: &LootEngine) -> loot::Result<Option<FillOutcome>> {
        self.unpack_with_context(engine, LootContext::CHEST)
    }

    pub fn unpack_with_context(
        &mut self,
        engine: &LootEngine,
        context: LootContext<'_>,
    ) -> loot::Result<Option<FillOutcome>> {
        let Some(table) = self.resolve(engine, context)? else {
            return Ok(None);
        };
        if self.loot_seed == 0 {
            return Err(LootError::ZeroSeedRequiresCallerRng);
        }
        self.loot_table = None;
        table
            .fill_with_context(&mut self.slots, context, self.loot_seed)
            .map(Some)
    }

    /// Use exactly the caller's source, independently of the retained seed.
    /// No-key calls bypass context validation and consume no RNG. Preflight
    /// errors preserve state; evaluation errors consume the key, keep backing
    /// slots unchanged, and may advance the RNG. No state or source is restored.
    pub fn unpack_with_rng(
        &mut self,
        engine: &LootEngine,
        context: LootContext<'_>,
        rng: &mut dyn LootRng,
    ) -> loot::Result<Option<FillOutcome>> {
        let Some(table) = self.resolve(engine, context)? else {
            return Ok(None);
        };
        self.loot_table = None;
        table.fill_with_rng(&mut self.slots, context, rng).map(Some)
    }

    fn resolve<'a>(
        &self,
        engine: &'a LootEngine,
        context: LootContext<'_>,
    ) -> loot::Result<Option<LootTable<'a>>> {
        let Some(key) = &self.loot_table else {
            return Ok(None);
        };
        let table = engine.table(key)?;
        table.validate_context(context)?;
        Ok(Some(table))
    }

    /// Replace only `LootTable`, `LootTableSeed` and `Items`; preserve base fields.
    /// Pending loot omits items and zero seeds. Materialized loot omits the key
    /// and retained seed, writing ascending Byte slots (an empty list uses End).
    /// All item encoding completes before the target compound is changed.
    pub fn write_nbt(&self, tag: &mut Nbt) -> loot::Result<()> {
        let fields = tag
            .compound_mut()
            .map_err(|_| invalid("chest", "expected compound"))?;
        validate_id(fields)?;
        let mut updates = BTreeMap::new();
        if let Some(key) = &self.loot_table {
            updates.insert("LootTable".into(), Nbt::String(key.clone()));
            if self.loot_seed != 0 {
                updates.insert("LootTableSeed".into(), Nbt::Long(self.loot_seed));
            }
        } else {
            let mut values = Vec::new();
            for (slot, stack) in self.slots.iter().enumerate() {
                if let Some(stack) = stack {
                    let mut item = stack.to_nbt()?;
                    item.compound_mut()
                        .expect("ItemStack::to_nbt returns a compound")
                        .insert("Slot".into(), Nbt::Byte(slot as i8));
                    values.push(item);
                }
            }
            updates.insert(
                "Items".into(),
                Nbt::List {
                    element_type: if values.is_empty() { 0 } else { 10 },
                    values,
                },
            );
        }
        for key in ["LootTable", "LootTableSeed", "Items"] {
            fields.remove(key);
        }
        fields.extend(updates);
        Ok(())
    }
}

fn invalid(path: &str, message: &str) -> LootError {
    LootError::InvalidData {
        path: path.into(),
        message: message.into(),
    }
}

fn validate_id(fields: &BTreeMap<String, Nbt>) -> loot::Result<()> {
    match fields.get("id") {
        None => Ok(()),
        Some(Nbt::String(id)) if id == "minecraft:chest" => Ok(()),
        Some(Nbt::String(id)) => Err(LootError::Unsupported {
            path: "chest.id".into(),
            kind: format!("block entity {id}"),
        }),
        Some(_) => Err(invalid("chest.id", "expected String tag")),
    }
}

// Syntax admission must not resolve/compile a table: unsupported pending keys
// remain loadable and saveable until unpack performs its capability preflight.
fn table_key(key: &str) -> loot::Result<String> {
    let (namespace, name) = key.split_once(':').unwrap_or(("minecraft", key));
    let identifier =
        |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_.-".contains(&byte);
    if namespace.is_empty()
        || name.is_empty()
        || !namespace.bytes().all(identifier)
        || !name.bytes().all(|byte| identifier(byte) || byte == b'/')
    {
        return Err(invalid("chest.LootTable", "invalid resource identifier"));
    }
    Ok(format!("{namespace}:{name}"))
}

fn read_items(engine: &LootEngine, value: Option<&Nbt>) -> loot::Result<[Option<ItemStack>; 27]> {
    let mut slots = std::array::from_fn(|_| None);
    let Some(value) = value else {
        return Ok(slots);
    };
    let values = match value {
        Nbt::List {
            element_type: 10,
            values,
        } => values,
        Nbt::List {
            element_type: 0,
            values,
        } if values.is_empty() => values,
        _ => {
            return Err(invalid(
                "chest.Items",
                "expected compound list or empty End list",
            ))
        }
    };
    if values.len() > slots.len() {
        return Err(invalid("chest.Items", "more than 27 slot entries"));
    }
    for (index, value) in values.iter().enumerate() {
        let path = format!("chest.Items[{index}]");
        let mut item = value
            .compound()
            .map_err(|_| invalid(&path, "expected item compound"))?
            .clone();
        let slot = match item.remove("Slot") {
            Some(Nbt::Byte(slot)) if (0..27).contains(&slot) => slot as usize,
            _ => return Err(invalid(&path, "expected Slot Byte in 0..=26")),
        };
        if slots[slot].is_some() {
            return Err(invalid(&path, "duplicate Slot"));
        }
        slots[slot] = Some(read_item(engine, &Nbt::Compound(item), &path)?);
    }
    Ok(slots)
}

fn read_item(engine: &LootEngine, tag: &Nbt, path: &str) -> loot::Result<ItemStack> {
    let item = engine
        .item_stack_from_json(&tag.to_json())?
        .ok_or_else(|| invalid(path, "empty item must be represented by an absent slot"))?;
    if item.to_nbt()? != *tag {
        return Err(invalid(
            path,
            "item NBT differs from its supported persistent encoding",
        ));
    }
    Ok(item)
}
