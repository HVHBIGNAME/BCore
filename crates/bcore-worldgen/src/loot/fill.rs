use super::{rng, ItemStack, LootError, LootRng, Result};

/// Optional phase observations. Default filling does not clone generated lists.
pub trait LootObserver {
    fn generated(&mut self, _items: &[ItemStack], _rng: &dyn LootRng) {}
    fn slots_shuffled(&mut self, _slots: &[usize], _rng: &dyn LootRng) {}
    fn items_shuffled(&mut self, _items: &[ItemStack], _rng: &dyn LootRng) {}
    fn placed(&mut self, _slot: usize, _stack: &ItemStack) {}
}

pub(super) struct NoObserver;
impl LootObserver for NoObserver {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FillOutcome {
    pub placed_stacks: usize,
    /// Overflow is native behavior, after all generation/shuffling RNG work.
    pub discarded_stacks: usize,
}

pub(super) fn place(
    mut items: Vec<ItemStack>,
    slots: &mut [Option<ItemStack>],
    rng: &mut dyn LootRng,
    observer: &mut dyn LootObserver,
) -> Result<FillOutcome> {
    if slots.len() > 65_536 {
        return Err(LootError::LimitExceeded("container slots"));
    }
    let mut available: Vec<_> = slots
        .iter()
        .enumerate()
        .filter(|(_, stack)| stack.as_ref().is_none_or(ItemStack::is_empty))
        .map(|(index, _)| index)
        .collect();
    rng::shuffle(&mut available, rng);
    observer.slots_shuffled(&available, rng);
    split(&mut items, available.len(), rng)?;
    observer.items_shuffled(&items, rng);
    let count = items.len();
    let mut placed_stacks = 0;
    for item in items {
        let Some(slot) = available.pop() else {
            break;
        };
        observer.placed(slot, &item);
        slots[slot] = Some(item);
        placed_stacks += 1;
    }
    Ok(FillOutcome {
        placed_stacks,
        discarded_stacks: count - placed_stacks,
    })
}

fn split(items: &mut Vec<ItemStack>, slots: usize, rng: &mut dyn LootRng) -> Result<()> {
    let mut pending = Vec::new();
    let mut done = Vec::with_capacity(items.len().max(slots));
    for item in items.drain(..) {
        if item.is_empty() {
            continue;
        }
        if item.count() > 1 {
            pending.push(item);
        } else {
            done.push(item);
        }
    }
    while slots > done.len() + pending.len() && !pending.is_empty() {
        let index = rng::between(rng, 0, pending.len() as i32 - 1)? as usize;
        let mut original = pending.remove(index);
        let amount = rng::between(rng, 1, original.count() / 2)?;
        let separated = original.with_count(amount);
        original.count -= amount;
        if original.count() > 1 && rng.next_bool() {
            pending.push(original);
        } else {
            done.push(original);
        }
        if separated.count() > 1 && rng.next_bool() {
            pending.push(separated);
        } else {
            done.push(separated);
        }
    }
    done.extend(pending);
    rng::shuffle(&mut done, rng);
    *items = done;
    Ok(())
}
