use super::{
    function::Function,
    number,
    table::{Entry, EntryKind, Singleton, SingletonKind, Table},
    ItemStack, LootContext, LootEngine, LootError, LootRng, Result,
};

pub(super) struct Evaluation<'a> {
    pub engine: &'a LootEngine,
    pub context: LootContext<'a>,
    pub rng: &'a mut dyn LootRng,
    pub output: Vec<ItemStack>,
    pub rolls: usize,
}

impl Evaluation<'_> {
    pub fn table(&mut self, table: &Table, outer: &[&[Function]], depth: usize) -> Result<()> {
        if depth >= 64 {
            return Err(LootError::LimitExceeded("nested loot table depth"));
        }
        let chain = prepend(&table.functions, outer);
        for pool in &table.pools {
            if !number::test_conditions(&pool.conditions, self.context, self.rng)? {
                continue;
            }
            let rolls = pool.rolls.int(self.rng)?;
            let bonus = number::floor(pool.bonus_rolls.float(self.rng) * 0.0_f32);
            let count = rolls.wrapping_add(bonus).max(0) as usize;
            self.rolls = self
                .rolls
                .checked_add(count)
                .ok_or(LootError::LimitExceeded("pool rolls"))?;
            if self.rolls > 1_000_000 {
                return Err(LootError::LimitExceeded("pool rolls"));
            }
            let pool_chain = prepend(&pool.functions, &chain);
            let mut expanded = Vec::with_capacity(pool.entries.len());
            for _ in 0..count {
                expanded.clear();
                for entry in &pool.entries {
                    expand(entry, self.context, self.rng, &mut expanded)?;
                }
                let total = expanded
                    .iter()
                    .try_fold(0_i32, |sum, single| sum.checked_add(single.weight))
                    .ok_or(LootError::LimitExceeded("pool weight sum"))?;
                if total == 0 {
                    continue;
                }
                let selected = if expanded.len() == 1 {
                    expanded[0]
                } else {
                    let mut choice = self.rng.next_int(total);
                    *expanded
                        .iter()
                        .find(|single| {
                            choice -= single.weight;
                            choice < 0
                        })
                        .expect("positive expanded pool weight")
                };
                self.emit(selected, &pool_chain, depth)?;
            }
        }
        Ok(())
    }

    fn emit(&mut self, selected: &Singleton, outer: &[&[Function]], depth: usize) -> Result<()> {
        let chain = prepend(&selected.functions, outer);
        match &selected.kind {
            SingletonKind::Empty | SingletonKind::Reference(None) => {}
            SingletonKind::Reference(Some(index)) => {
                let table = self.engine.tables[*index].as_ref().map_err(Clone::clone)?;
                self.table(table, &chain, depth + 1)?;
            }
            SingletonKind::Inline(table) => self.table(table, &chain, depth + 1)?,
            SingletonKind::Item(item) => {
                let mut stack = ItemStack::new(item.clone(), 1);
                // Each child emission runs its entire enclosing function chain
                // before the next child roll. Buffering a nested table first
                // would reorder RNG-consuming parent functions.
                for functions in chain {
                    for function in functions {
                        function.apply(
                            &mut stack,
                            &self.engine.registry,
                            self.context,
                            self.rng,
                        )?;
                    }
                }
                let max = stack.max_stack_size();
                if max < 1 {
                    return Err(LootError::LimitExceeded("nonpositive stack maximum"));
                }
                let chunks = if stack.count() < max {
                    1
                } else {
                    ((stack.count() - 1) / max + 1) as usize
                };
                if self.output.len() + chunks > 65_536 {
                    return Err(LootError::LimitExceeded("generated stacks"));
                }
                if stack.count() < max {
                    self.output.push(stack);
                } else {
                    let mut remaining = stack.count();
                    while remaining > 0 {
                        self.output.push(stack.with_count(remaining.min(max)));
                        remaining -= max;
                    }
                }
            }
        }
        Ok(())
    }
}

fn prepend<'a>(functions: &'a [Function], outer: &[&'a [Function]]) -> Vec<&'a [Function]> {
    let mut chain = Vec::with_capacity(outer.len() + 1);
    if !functions.is_empty() {
        chain.push(functions);
    }
    chain.extend_from_slice(outer);
    chain
}

fn expand<'a>(
    entry: &'a Entry,
    context: LootContext<'_>,
    rng: &mut dyn LootRng,
    result: &mut Vec<&'a Singleton>,
) -> Result<bool> {
    if !number::test_conditions(&entry.conditions, context, rng)? {
        return Ok(false);
    }
    match &entry.kind {
        EntryKind::Singleton(single) => {
            if single.weight > 0 {
                result.push(single);
            }
            Ok(true)
        }
        EntryKind::Alternatives(children) => {
            for child in children {
                if expand(child, context, rng, result)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        EntryKind::Sequence(children) => {
            for child in children {
                if !expand(child, context, rng, result)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        EntryKind::Group(children) => {
            for child in children {
                expand(child, context, rng, result)?;
            }
            Ok(true)
        }
    }
}
