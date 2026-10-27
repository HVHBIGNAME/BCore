use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use super::{lock, ChunkFuture, ChunkStatus, MainTask, NativeRequestError, Shared, Task};
use crate::generation::{graph::layer_positions, ChunkPyramid, GenerationError};

pub(super) fn run(shared: Arc<Shared>) {
    let result = catch_unwind(AssertUnwindSafe(|| run_queue(&shared)));
    if let Err(payload) = result {
        shared.close(NativeRequestError::WorkerPanicked(
            crate::generation::panic_detail(payload),
        ));
    }
    lock(&shared.state).worker_active = false;
    shared.wake.notify_all();
}

fn run_queue(shared: &Shared) {
    loop {
        let task = {
            let mut state = lock(&shared.state);
            state.worker_active = false;
            shared.wake.notify_all();
            while state.closed.is_none() && state.worker_queue.is_empty() {
                state = shared.wake.wait(state).unwrap_or_else(|e| e.into_inner());
            }
            if state.closed.is_some() {
                return;
            }
            // Native dispatcher priority comes from the holder's queue level.
            // Among equal levels preserve submission order; layer order is the
            // existing pyramid's X-outer/Z-inner source order.
            let index = state
                .worker_queue
                .iter()
                .enumerate()
                .min_by_key(|(index, task)| {
                    (state.holders[&(task.pos.x, task.pos.z)].level, *index)
                })
                .map(|(index, _)| index)
                .unwrap();
            let task = state.worker_queue.remove(index).unwrap();
            if !state.current(task) {
                continue;
            }
            state.worker_active = true;
            task
        };
        if let Err(error) = run_task(shared, task) {
            shared.fail_holder(task.pos, error);
        }
        let mut state = lock(&shared.state);
        if state.current(task) {
            let holder = state.holders.get_mut(&(task.pos.x, task.pos.z)).unwrap();
            if !holder.main_queued {
                holder.target = None;
            }
        }
    }
}

fn run_task(shared: &Shared, task: Task) -> Result<(), NativeRequestError> {
    let step = ChunkPyramid::Generation.step(task.target);
    for status in ChunkStatus::ALL {
        let Some(radius) = step.layer_radius(status) else {
            continue;
        };
        for pos in layer_positions(task.pos, radius) {
            {
                let state = lock(&shared.state);
                if !state.current(task) {
                    return Ok(());
                }
                if !state
                    .holders
                    .get(&(pos.x, pos.z))
                    .and_then(|holder| holder.allowed)
                    .is_some_and(|ceiling| status <= ceiling)
                {
                    return Err(NativeRequestError::Admission { pos, status });
                }
            }
            #[cfg(test)]
            shared.control.before(pos, status);
            if !lock(&shared.state).current(task) {
                return Ok(());
            }
            if status == ChunkStatus::Full {
                let mut state = lock(&shared.state);
                if state.current(task) {
                    let owner = state.owner.clone();
                    let holder = state.holders.get_mut(&(pos.x, pos.z)).unwrap();
                    if !holder.main_queued {
                        holder.main_queued = true;
                        holder.full_supplier = Some(ChunkFuture::pending(&owner));
                        state.main.push_back(MainTask::Full(task));
                        shared.wake.notify_all();
                    }
                }
                return Ok(());
            }
            let result = shared
                .world
                .generation_order
                .lock()
                .map_err(|_| GenerationError::Poisoned)
                .and_then(|_order| {
                    shared
                        .world
                        .state
                        .lock()
                        .map_err(|_| GenerationError::Poisoned)
                        .and_then(|mut state| state.run_stage(shared.world.generator, pos, status))
                });
            if let Err(error) = result {
                shared.fail_holder(pos, error.clone().into());
                return Err(error.into());
            }
            shared.world.yield_to_runtime()?;
            shared.publish(pos, status)?;
            #[cfg(test)]
            shared.control.after(pos, status);
        }
    }
    Ok(())
}
