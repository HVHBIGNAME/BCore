//! Native 26.1 chunk-local tick preparation, save conversion, and queue ordering.
//!
//! Targets identify canonical native types: block default-state IDs or fluid
//! registry IDs. Block and fluid containers have independent identity sets.
//! Callers supply game times and sub-tick orders; polling only returns work.
use crate::tick_request::{TickRequest, TickTarget};
use bcore_core::ChunkPos;
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::hash::Hash;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(i8)]
pub enum TickPriority {
    ExtremelyHigh = -3,
    VeryHigh = -2,
    High = -1,
    #[default]
    Normal = 0,
    Low = 1,
    VeryLow = 2,
    ExtremelyLow = 3,
}

impl TickPriority {
    /// Native `TickPriority.byValue`, including clamping outside -3..=3.
    pub const fn from_value(value: i32) -> Self {
        match value {
            ..=-3 => Self::ExtremelyHigh,
            -2 => Self::VeryHigh,
            -1 => Self::High,
            0 => Self::Normal,
            1 => Self::Low,
            2 => Self::VeryLow,
            3.. => Self::ExtremelyLow,
        }
    }

    pub const fn value(self) -> i32 {
        self as i32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavedTick<T> {
    pub block_pos: [i32; 3],
    pub target: T,
    pub delay: i32,
    pub priority: TickPriority,
}

impl<T: Clone> SavedTick<T> {
    pub fn unpack(&self, game_time: i64, sub_tick_order: i64) -> ScheduledTick<T> {
        ScheduledTick {
            block_pos: self.block_pos,
            target: self.target.clone(),
            trigger_tick: game_time.wrapping_add(i64::from(self.delay)),
            priority: self.priority,
            sub_tick_order,
        }
    }

    /// Fields emitted by native `SavedTick.codec(registry.byNameCodec())`.
    pub fn save_data(&self, type_identifier: &str) -> Value {
        let [x, y, z] = self.block_pos;
        json!({"i": type_identifier, "x": x, "y": y, "z": z,
            "t": self.delay, "p": self.priority.value()})
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledTick<T> {
    pub block_pos: [i32; 3],
    pub target: T,
    pub trigger_tick: i64,
    pub priority: TickPriority,
    pub sub_tick_order: i64,
}

impl<T> ScheduledTick<T> {
    pub fn cmp_drain_order(&self, other: &Self) -> Ordering {
        self.trigger_tick
            .cmp(&other.trigger_tick)
            .then_with(|| self.cmp_intra_tick_order(other))
    }

    pub fn cmp_intra_tick_order(&self, other: &Self) -> Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| self.sub_tick_order.cmp(&other.sub_tick_order))
    }
}

impl<T: Clone> ScheduledTick<T> {
    pub fn to_saved_tick(&self, game_time: i64) -> SavedTick<T> {
        SavedTick {
            block_pos: self.block_pos,
            target: self.target.clone(),
            // Java subtracts as a wrapping long, then narrows to a signed int.
            delay: self.trigger_tick.wrapping_sub(game_time) as i32,
            priority: self.priority,
        }
    }
}

/// Native chunk filtering uses X/Z only and retains input order and duplicates.
pub fn filter_ticks_for_chunk<T: Clone>(
    ticks: &[SavedTick<T>],
    owner: ChunkPos,
) -> Vec<SavedTick<T>> {
    ticks
        .iter()
        .filter(|tick| tick.block_pos[0] >> 4 == owner.x && tick.block_pos[2] >> 4 == owner.z)
        .cloned()
        .collect()
}

#[derive(Debug, Clone)]
pub struct ProtoTickQueue<T> {
    ticks: Vec<SavedTick<T>>,
    identities: HashSet<([i32; 3], T)>,
}

impl<T: Clone + Eq + Hash> Default for ProtoTickQueue<T> {
    fn default() -> Self {
        Self {
            ticks: Vec::new(),
            identities: HashSet::new(),
        }
    }
}

impl<T: Clone + Eq + Hash> ProtoTickQueue<T> {
    /// Native load retains the first saved delay/priority for each identity.
    pub fn load(ticks: impl IntoIterator<Item = SavedTick<T>>) -> Self {
        let mut queue = Self::default();
        for tick in ticks {
            queue.insert(tick);
        }
        queue
    }

    /// Native proto scheduling discards trigger time and sub-order, saving delay 0.
    /// Returns whether the position/type identity was newly inserted.
    pub fn schedule(&mut self, tick: ScheduledTick<T>) -> bool {
        self.insert(SavedTick {
            block_pos: tick.block_pos,
            target: tick.target,
            delay: 0,
            priority: tick.priority,
        })
    }

    fn insert(&mut self, tick: SavedTick<T>) -> bool {
        if !self
            .identities
            .insert((tick.block_pos, tick.target.clone()))
        {
            return false;
        }
        self.ticks.push(tick);
        true
    }

    pub fn has_scheduled_tick(&self, block_pos: [i32; 3], target: &T) -> bool {
        self.identities.contains(&(block_pos, target.clone()))
    }

    pub fn count(&self) -> usize {
        self.ticks.len()
    }

    pub fn scheduled_ticks(&self) -> &[SavedTick<T>] {
        &self.ticks
    }

    pub fn pack(&self, _game_time: i64) -> Vec<SavedTick<T>> {
        self.ticks.clone()
    }
}

#[derive(Debug, Clone, Default)]
pub struct PreparedTickQueues {
    pub blocks: ProtoTickQueue<u32>,
    pub fluids: ProtoTickQueue<u32>,
}

impl PreparedTickQueues {
    pub fn from_requests(requests: impl IntoIterator<Item = TickRequest>) -> Self {
        let mut queues = Self::default();
        for request in requests {
            queues.schedule_request(request);
        }
        queues
    }

    pub fn schedule_request(&mut self, request: TickRequest) -> bool {
        self.schedule_request_with_priority(request, TickPriority::Normal)
    }

    pub fn schedule_request_with_priority(
        &mut self,
        request: TickRequest,
        priority: TickPriority,
    ) -> bool {
        let (queue, target) = match request.target {
            TickTarget::Block(id) => (&mut self.blocks, id),
            TickTarget::Fluid(id) => (&mut self.fluids, id),
        };
        queue.schedule(ScheduledTick {
            block_pos: request.block_pos,
            target,
            trigger_tick: i64::from(request.delay),
            priority,
            sub_tick_order: 0,
        })
    }
}

#[derive(Debug, Clone)]
pub struct LevelTickQueue<T> {
    heap: Vec<ScheduledTick<T>>,
    pending: Option<Vec<SavedTick<T>>>,
    identities: HashSet<([i32; 3], T)>,
}

impl<T: Clone + Eq + Hash> Default for LevelTickQueue<T> {
    fn default() -> Self {
        Self {
            heap: Vec::new(),
            pending: None,
            identities: HashSet::new(),
        }
    }
}

impl<T: Clone + Eq + Hash> LevelTickQueue<T> {
    /// Loaded entries remain pending until unpack. Native load preserves duplicates.
    pub fn from_saved(ticks: Vec<SavedTick<T>>) -> Self {
        Self {
            identities: ticks
                .iter()
                .map(|tick| (tick.block_pos, tick.target.clone()))
                .collect(),
            pending: Some(ticks),
            heap: Vec::new(),
        }
    }

    pub fn count(&self) -> usize {
        self.heap.len() + self.pending.as_ref().map_or(0, Vec::len)
    }

    pub fn has_scheduled_tick(&self, block_pos: [i32; 3], target: &T) -> bool {
        self.identities.contains(&(block_pos, target.clone()))
    }

    pub fn peek(&self) -> Option<&ScheduledTick<T>> {
        self.heap.first()
    }

    /// Native getAll visits heap-array order, excluding not-yet-unpacked saves.
    pub fn get_all(&self) -> &[ScheduledTick<T>] {
        &self.heap
    }

    pub fn schedule(&mut self, tick: ScheduledTick<T>) -> bool {
        if !self
            .identities
            .insert((tick.block_pos, tick.target.clone()))
        {
            return false;
        }
        self.push_unchecked(tick);
        true
    }

    fn push_unchecked(&mut self, tick: ScheduledTick<T>) {
        let mut index = self.heap.len();
        self.heap.push(tick);
        while index > 0 {
            let parent = (index - 1) / 2;
            if self.heap[index].cmp_drain_order(&self.heap[parent]) != Ordering::Less {
                break;
            }
            self.heap.swap(index, parent);
            index = parent;
        }
    }

    pub fn poll(&mut self) -> Option<ScheduledTick<T>> {
        let last = self.heap.pop()?;
        let result = if self.heap.is_empty() {
            last
        } else {
            let first = std::mem::replace(&mut self.heap[0], last);
            let mut index = 0;
            while index < self.heap.len() / 2 {
                let mut child = index * 2 + 1;
                let right = child + 1;
                // java.util.PriorityQueue chooses the left child on comparator ties.
                if right < self.heap.len()
                    && self.heap[child].cmp_drain_order(&self.heap[right]) == Ordering::Greater
                {
                    child = right;
                }
                if self.heap[index].cmp_drain_order(&self.heap[child]) != Ordering::Greater {
                    break;
                }
                self.heap.swap(index, child);
                index = child;
            }
            first
        };
        // This also matches native loaded-duplicate behavior: polling one removes
        // the identity even if another loaded duplicate is still in the heap.
        self.identities
            .remove(&(result.block_pos, result.target.clone()));
        Some(result)
    }

    /// Pending saves precede heap-order entries. Active delays are relative to time.
    pub fn pack(&self, game_time: i64) -> Vec<SavedTick<T>> {
        let mut saved = self.pending.clone().unwrap_or_default();
        saved.extend(self.heap.iter().map(|tick| tick.to_saved_tick(game_time)));
        saved
    }

    /// Native unpack runs once, assigning pending entries sub-orders -N through -1.
    pub fn unpack(&mut self, game_time: i64) {
        if let Some(pending) = self.pending.take() {
            let first_order = -(pending.len() as i64);
            for (index, tick) in pending.into_iter().enumerate() {
                self.push_unchecked(tick.unpack(game_time, first_order + index as i64));
            }
        }
    }
}
