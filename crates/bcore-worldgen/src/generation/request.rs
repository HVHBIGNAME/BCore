//! Native-style request admission for a fresh, quiescent overworld.
//!
//! One owner thread runs admission and a conditional FIFO main executor. An
//! owned worker runs the existing generation layers independently. UNKNOWN
//! tickets are retained; expiry, unload/reload and gameplay are not implemented.
//! The generation-only API remains available for callers without this lifecycle.
mod cache;
mod future;
mod tickets;
mod worker;

pub use future::ChunkFuture;
pub use tickets::{generation_status, maximum_loading_level, ticket_level, FULL_CHUNK_LEVEL};

use std::collections::{BTreeMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle, ThreadId};

use bcore_core::ChunkPos;

use super::{validate_generation_pos, ChunkStatus, GenerationError, GenerationWorld, Key};
use cache::RequestCache;
use future::Outcome;
use tickets::LoadingTickets;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeRequestError {
    Generation(GenerationError),
    WrongThread,
    ForeignFuture,
    Closed,
    WorkerStart(String),
    WorkerPanicked(String),
    MainPanicked(String),
    Admission { pos: ChunkPos, status: ChunkStatus },
}

impl From<GenerationError> for NativeRequestError {
    fn from(error: GenerationError) -> Self {
        Self::Generation(error)
    }
}

impl std::fmt::Display for NativeRequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Generation(error) => error.fmt(f),
            Self::WrongThread => {
                f.write_str("native request admission/pumping requires its owner thread")
            }
            Self::ForeignFuture => f.write_str("future belongs to another native request runtime"),
            Self::Closed => f.write_str("native request runtime is closed"),
            Self::WorkerStart(detail) => write!(f, "cannot start generation worker: {detail}"),
            Self::WorkerPanicked(detail) => write!(f, "generation worker panicked: {detail}"),
            Self::MainPanicked(detail) => write!(f, "main request executor panicked: {detail}"),
            Self::Admission { pos, status } => write!(f, "holder {pos:?} does not admit {status}"),
        }
    }
}
impl std::error::Error for NativeRequestError {}

/// A status ceiling is permission, not an instruction to generate to that status.
#[derive(Debug, Clone)]
pub struct RequestHolderSnapshot {
    pub pos: ChunkPos,
    pub ticket_level: u8,
    pub highest_allowed_status: Option<ChunkStatus>,
    pub generation_target: Option<ChunkStatus>,
    pub full_accessible: bool,
    pub full_body_queued: bool,
    /// The radius-one accessibility aggregate, separate from every status slot.
    pub full_accessibility_future: Option<ChunkFuture>,
    /// Completion of the queued FULL supplier; absent until it is submitted.
    pub full_supplier_future: Option<ChunkFuture>,
    pub promotion_callback_queued: bool,
    /// Scheduler notification only; world registration/ticking remains Partial.
    pub promotion_callback_executed: bool,
    pub ready_statuses: Vec<ChunkStatus>,
    pub saved_data_admitted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainTaskKind {
    FullConversion,
    /// The scheduler notification; entity-manager integration is still missing.
    FullPromotion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MainTaskSnapshot {
    pub pos: ChunkPos,
    pub kind: MainTaskKind,
}

#[derive(Debug, Clone)]
pub struct RequestRuntimeSnapshot {
    pub holders: usize,
    pub pending_generation_tasks: usize,
    pub queued_worker_tasks: usize,
    pub worker_active: bool,
    /// Positions of all queued main tasks, including promotion callbacks.
    pub main_queue: Vec<ChunkPos>,
    pub main_tasks: Vec<MainTaskSnapshot>,
    pub cache: Vec<(ChunkPos, ChunkStatus)>,
    pub cache_invalidations: u64,
    pub main_bodies_executed: u64,
    pub promotion_callbacks_executed: u64,
    pub closed: bool,
}

/// Retain one instance per native request history. Its creating thread is the
/// main executor owner. Views and completed futures can be read on other threads;
/// off-thread request dispatch is explicitly outside this slice.
pub struct NativeChunkRequests {
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
    main_thread: ThreadId,
}

impl std::fmt::Debug for NativeChunkRequests {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeChunkRequests")
            .field("main_thread", &self.main_thread)
            .field("state", &self.snapshot())
            .finish()
    }
}

impl NativeChunkRequests {
    /// Validate before a lazy caller allocates a runtime or binds its owner thread.
    pub fn validate_position(pos: ChunkPos) -> Result<(), NativeRequestError> {
        validate_generation_pos(pos).map_err(NativeRequestError::from)
    }

    pub fn new(world: Arc<GenerationWorld>) -> Result<Self, NativeRequestError> {
        let shared = Arc::new(Shared {
            world,
            state: Mutex::new(RequestState::new()),
            wake: Condvar::new(),
            #[cfg(test)]
            control: tests::WorkerControl::default(),
        });
        let worker_shared = shared.clone();
        let worker = thread::Builder::new()
            .name("bcore-native-worldgen".into())
            .spawn(move || worker::run(worker_shared))
            .map_err(|error| NativeRequestError::WorkerStart(error.to_string()))?;
        Ok(Self {
            shared,
            worker: Mutex::new(Some(worker)),
            main_thread: thread::current().id(),
        })
    }

    fn check_main_thread(&self) -> Result<(), NativeRequestError> {
        if thread::current().id() != self.main_thread {
            Err(NativeRequestError::WrongThread)
        } else {
            Ok(())
        }
    }

    /// Storage owners validate this boundary before doing admission work.
    pub fn check_admission(&self) -> Result<(), NativeRequestError> {
        self.check_main_thread()?;
        lock(&self.shared.state).check_open()
    }

    /// Publish already admitted storage data through the existing holder futures
    /// and four-slot cache. No generation, tickets or promotion are scheduled.
    pub fn admit_saved_chunk(&self, pos: ChunkPos) -> Result<(), NativeRequestError> {
        Self::validate_position(pos)?;
        self.check_admission()?;
        if !self.shared.world.is_saved_chunk(pos)? {
            return Err(NativeRequestError::Admission {
                pos,
                status: ChunkStatus::Full,
            });
        }
        let level = self
            .shared
            .world
            .returned_chunk(pos, ChunkStatus::Full)?
            .ok_or(NativeRequestError::Admission {
                pos,
                status: ChunkStatus::Full,
            })?;
        let imposter = self
            .shared
            .world
            .returned_chunk(pos, ChunkStatus::Empty)?
            .ok_or(NativeRequestError::Admission {
                pos,
                status: ChunkStatus::Empty,
            })?;
        let mut state = lock(&self.shared.state);
        state.check_open()?;
        let owner = state.owner.clone();
        let holder = state
            .holders
            .entry((pos.x, pos.z))
            .or_insert_with(RequestHolder::new);
        if holder.saved {
            return Ok(());
        }
        holder.saved = true;
        holder.allowed = Some(ChunkStatus::Full);
        for future in holder.futures[..ChunkStatus::Full.index()].iter().flatten() {
            future.complete(Ok(Some(imposter.clone())));
        }
        if let Some(future) = &holder.futures[ChunkStatus::Full.index()] {
            future.complete(Ok(Some(level.clone())));
        }
        if let Some(supplier) = &holder.full_supplier {
            supplier.complete(Ok(Some(level.clone())));
        }
        holder.target = None;
        holder.revision += 1;
        holder.main_queued = false;
        let lower = ChunkFuture::completed(&owner, Ok(Some(imposter)));
        for slot in &mut holder.futures[..ChunkStatus::Full.index()] {
            *slot = Some(lower.clone());
        }
        holder.futures[ChunkStatus::Full.index()] =
            Some(ChunkFuture::completed(&owner, Ok(Some(level))));
        state.pending.retain(|task| task.pos != pos);
        state.worker_queue.retain(|task| task.pos != pos);
        state
            .main
            .retain(|task| !matches!(task, MainTask::Full(task) if task.pos == pos));
        state.refresh_accessibility_near(pos);
        state.cache.clear();
        state.cache_invalidations += 1;
        self.shared.wake.notify_all();
        Ok(())
    }

    /// ServerChunkCache.getChunk: exact four-slot lookup, requested-future wait,
    /// then a store. Neither cache hits nor completed futures drain main work.
    pub fn get_chunk(&self, pos: ChunkPos, status: ChunkStatus, create: bool) -> Outcome {
        Self::validate_position(pos)?;
        self.check_main_thread()?;
        {
            let state = lock(&self.shared.state);
            state.check_open()?;
            if let Some(hit) = state.cache.lookup(pos, status, create) {
                return Ok(hit);
            }
        }
        let future = self.request_future(pos, status, create)?;
        let object = self.wait(&future)?;
        let mut state = lock(&self.shared.state);
        state.check_open()?;
        state.cache.store(pos, status, object.clone());
        Ok(object)
    }

    /// Nonblocking getChunkFutureMainThread admission. The future belongs to the
    /// requested status even if admission has already scheduled a FULL target.
    /// Call `wait` or `poll_task` to flush any newly pending generation tasks.
    pub fn request_future(
        &self,
        pos: ChunkPos,
        status: ChunkStatus,
        create: bool,
    ) -> Result<ChunkFuture, NativeRequestError> {
        Self::validate_position(pos)?;
        self.check_main_thread()?;
        let level = ticket_level(status);
        let absent = {
            let mut state = lock(&self.shared.state);
            state.check_open()?;
            if create {
                state.tickets.add(pos, level);
            }
            state
                .holders
                .get(&(pos.x, pos.z))
                .is_none_or(|holder| !holder.saved && holder.level > level)
        };
        if create && absent {
            self.run_distance_updates()?;
        }
        {
            let state = lock(&self.shared.state);
            state.check_open()?;
            if state
                .holders
                .get(&(pos.x, pos.z))
                .is_none_or(|holder| !holder.saved && holder.level > level)
            {
                return if create {
                    Err(NativeRequestError::Admission { pos, status })
                } else {
                    Ok(state.unloaded.clone())
                };
            }
        }
        self.shared.schedule(pos, status)
    }

    /// BlockableEventLoop.managedBlock checks readiness before the first poll
    /// and after every poll/wakeup. No runtime mutex is held while running a body.
    pub fn wait(&self, future: &ChunkFuture) -> Outcome {
        self.check_main_thread()?;
        if !future.belongs_to(&lock(&self.shared.state).owner) {
            return Err(NativeRequestError::ForeignFuture);
        }
        loop {
            if let Some(result) = future.get_now() {
                return result;
            }
            if self.poll_task()? {
                continue;
            }
            let mut state = lock(&self.shared.state);
            while !future.is_done() && !state.has_main_work() {
                state.check_open()?;
                state = self
                    .shared
                    .wake
                    .wait(state)
                    .unwrap_or_else(|e| e.into_inner());
            }
        }
    }

    /// Process distance/map updates first, otherwise run one FIFO main task.
    /// Generation-task flushing happens even when the update returns false.
    pub fn poll_task(&self) -> Result<bool, NativeRequestError> {
        self.check_main_thread()?;
        if self.run_distance_updates()? {
            return Ok(true);
        }
        let job = {
            let mut state = lock(&self.shared.state);
            state.check_open()?;
            state.main.pop_front()
        };
        let Some(job) = job else { return Ok(false) };
        let result = catch_unwind(AssertUnwindSafe(|| match job {
            MainTask::Full(task) => {
                let _order = self
                    .shared
                    .world
                    .generation_order
                    .lock()
                    .map_err(|_| GenerationError::Poisoned)?;
                self.shared
                    .world
                    .state
                    .lock()
                    .map_err(|_| GenerationError::Poisoned)?
                    .run_stage(self.shared.world.generator, task.pos, ChunkStatus::Full)?;
                self.shared.publish_full(task.pos)
            }
            MainTask::Promotion(pos) => {
                let mut state = lock(&self.shared.state);
                state.check_open()?;
                let accessibility = state
                    .holders
                    .get_mut(&(pos.x, pos.z))
                    .unwrap()
                    .accessibility
                    .as_mut()
                    .unwrap();
                accessibility.callback_queued = false;
                accessibility.callback_executed = true;
                state.promotion_callbacks_executed += 1;
                Ok(())
            }
        }));
        let result = match result {
            Ok(result) => result,
            Err(payload) => {
                let error = NativeRequestError::MainPanicked(super::panic_detail(payload));
                self.shared.close(error.clone());
                return Err(error);
            }
        };
        if let Err(error) = result {
            self.shared.fail_holder(job.snapshot().pos, error);
        }
        let mut state = lock(&self.shared.state);
        if let MainTask::Full(task) = job {
            if let Some(holder) = state.holders.get_mut(&(task.pos.x, task.pos.z)) {
                holder.main_queued = false;
                holder.target = None;
            }
            state.main_bodies_executed += 1;
        }
        self.shared.wake.notify_all();
        Ok(true)
    }

    fn run_distance_updates(&self) -> Result<bool, NativeRequestError> {
        let (changed, admissions) = {
            let mut state = lock(&self.shared.state);
            state.check_open()?;
            let changes = state.tickets.settle();
            // Pass one: every changed holder receives its new ceiling before
            // any accessibility transition can request its neighbours.
            for &(pos, level) in &changes {
                let holder = state
                    .holders
                    .entry((pos.x, pos.z))
                    .or_insert_with(RequestHolder::new);
                holder.level = level;
                holder.allowed = if holder.saved {
                    Some(ChunkStatus::Full)
                } else {
                    generation_status(level)
                };
            }
            let mut admissions = Vec::new();
            let owner = state.owner.clone();
            // Pass two: update futures/accessibility, preserving range z/x order.
            for &(pos, _) in &changes {
                let holder = state.holders.get_mut(&(pos.x, pos.z)).unwrap();
                let accessible = holder.level <= FULL_CHUNK_LEVEL;
                if accessible && !holder.accessible {
                    holder.accessibility = Some(Accessibility::new(&owner));
                    let mut demands = Vec::with_capacity(9);
                    for dz in -1i32..=1 {
                        for dx in -1i32..=1 {
                            let status =
                                generation_status(FULL_CHUNK_LEVEL + dx.abs().max(dz.abs()) as u8)
                                    .expect("accessible range status");
                            demands.push((ChunkPos::new(pos.x + dx, pos.z + dz), status));
                        }
                    }
                    admissions.push((pos, demands));
                }
                holder.accessible = accessible;
            }
            #[cfg(test)]
            state.admission_batches.push((
                changes.clone(),
                admissions
                    .iter()
                    .flat_map(|(_, demands)| demands.iter().copied())
                    .collect(),
            ));
            (!changes.is_empty(), admissions)
        };
        for (centre, demands) in admissions {
            let mut dependencies = Vec::with_capacity(demands.len());
            for (pos, status) in demands {
                match self.shared.schedule(pos, status) {
                    Ok(future) => dependencies.push(future),
                    Err(error) => {
                        self.shared.close(error.clone());
                        return Err(error);
                    }
                }
            }
            let mut state = lock(&self.shared.state);
            state.check_open()?;
            state
                .holders
                .get_mut(&(centre.x, centre.z))
                .unwrap()
                .accessibility
                .as_mut()
                .unwrap()
                .dependencies = Some(dependencies);
            state.complete_accessibility(centre);
        }
        let mut state = lock(&self.shared.state);
        state.check_open()?;
        while let Some(task) = state.pending.pop_front() {
            state.worker_queue.push_back(task);
        }
        if changed {
            state.cache.clear();
            state.cache_invalidations += 1;
        }
        self.shared.wake.notify_all();
        Ok(changed)
    }

    pub fn holder(&self, pos: ChunkPos) -> Option<RequestHolderSnapshot> {
        let state = lock(&self.shared.state);
        state
            .holders
            .get(&(pos.x, pos.z))
            .map(|holder| RequestHolderSnapshot {
                pos,
                ticket_level: holder.level,
                highest_allowed_status: holder.allowed,
                generation_target: holder.target,
                full_accessible: holder.accessible,
                full_body_queued: holder.main_queued,
                full_accessibility_future: holder.accessibility.as_ref().map(|a| a.future.clone()),
                full_supplier_future: holder.full_supplier.clone(),
                promotion_callback_queued: holder
                    .accessibility
                    .as_ref()
                    .is_some_and(|a| a.callback_queued),
                promotion_callback_executed: holder
                    .accessibility
                    .as_ref()
                    .is_some_and(|a| a.callback_executed),
                ready_statuses: ChunkStatus::ALL
                    .into_iter()
                    .filter(|s| {
                        holder.futures[s.index()]
                            .as_ref()
                            .is_some_and(ChunkFuture::is_done)
                    })
                    .collect(),
                saved_data_admitted: holder.saved,
            })
    }

    pub fn snapshot(&self) -> RequestRuntimeSnapshot {
        let state = lock(&self.shared.state);
        RequestRuntimeSnapshot {
            holders: state.holders.len(),
            pending_generation_tasks: state.pending.len(),
            queued_worker_tasks: state.worker_queue.len(),
            worker_active: state.worker_active,
            main_queue: state.main.iter().map(|task| task.snapshot().pos).collect(),
            main_tasks: state.main.iter().map(|task| task.snapshot()).collect(),
            cache: state.cache.keys(),
            cache_invalidations: state.cache_invalidations,
            main_bodies_executed: state.main_bodies_executed,
            promotion_callbacks_executed: state.promotion_callbacks_executed,
            closed: state.closed.is_some(),
        }
    }

    /// Resolve every outstanding waiter, cancel queued work and join the owned
    /// worker. An already executing stage finishes at its next stage boundary.
    pub fn shutdown(&self) {
        self.shared.close(NativeRequestError::Closed);
        let worker = {
            let mut worker = lock(&self.worker);
            // A generation-input callback can request cancellation from the
            // worker itself. Leave its join handle for the owner instead of
            // attempting to join the current thread.
            if worker
                .as_ref()
                .is_some_and(|w| w.thread().id() == thread::current().id())
            {
                return;
            }
            worker.take()
        };
        if let Some(worker) = worker {
            if let Err(payload) = worker.join() {
                self.shared
                    .close(NativeRequestError::WorkerPanicked(super::panic_detail(
                        payload,
                    )));
            }
        }
    }
}

impl Drop for NativeChunkRequests {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Shared {
    world: Arc<GenerationWorld>,
    state: Mutex<RequestState>,
    wake: Condvar,
    #[cfg(test)]
    control: tests::WorkerControl,
}

#[derive(Clone, Copy)]
struct Task {
    pos: ChunkPos,
    target: ChunkStatus,
    revision: u64,
}

#[derive(Clone, Copy)]
enum MainTask {
    Full(Task),
    Promotion(ChunkPos),
}

impl MainTask {
    fn snapshot(self) -> MainTaskSnapshot {
        match self {
            Self::Full(task) => MainTaskSnapshot {
                pos: task.pos,
                kind: MainTaskKind::FullConversion,
            },
            Self::Promotion(pos) => MainTaskSnapshot {
                pos,
                kind: MainTaskKind::FullPromotion,
            },
        }
    }
}

struct Accessibility {
    future: ChunkFuture,
    // Keep the selected futures, not holder slots that FULL can replace later.
    dependencies: Option<Vec<ChunkFuture>>,
    callback_queued: bool,
    callback_executed: bool,
}

impl Accessibility {
    fn new(owner: &Arc<()>) -> Self {
        Self {
            future: ChunkFuture::pending(owner),
            dependencies: None,
            callback_queued: false,
            callback_executed: false,
        }
    }

    fn ready_result(&self) -> Option<Outcome> {
        let dependencies = self.dependencies.as_ref()?;
        let results = dependencies
            .iter()
            .map(ChunkFuture::get_now)
            .collect::<Option<Vec<_>>>()?;
        let mut centre = None;
        for (index, result) in results.into_iter().enumerate() {
            match result {
                Ok(Some(object)) => {
                    if index == dependencies.len() / 2 {
                        centre = Some(object);
                    }
                }
                other => return Some(other),
            }
        }
        Some(Ok(centre))
    }
}

struct RequestHolder {
    saved: bool,
    level: u8,
    allowed: Option<ChunkStatus>,
    accessible: bool,
    accessibility: Option<Accessibility>,
    full_supplier: Option<ChunkFuture>,
    futures: [Option<ChunkFuture>; 12],
    target: Option<ChunkStatus>,
    revision: u64,
    main_queued: bool,
    failure: Option<NativeRequestError>,
}

impl RequestHolder {
    fn new() -> Self {
        Self {
            saved: false,
            level: maximum_loading_level() + 1,
            allowed: None,
            accessible: false,
            accessibility: None,
            full_supplier: None,
            futures: Default::default(),
            target: None,
            revision: 0,
            main_queued: false,
            failure: None,
        }
    }
}

struct RequestState {
    owner: Arc<()>,
    unloaded: ChunkFuture,
    tickets: LoadingTickets,
    holders: BTreeMap<Key, RequestHolder>,
    pending: VecDeque<Task>,
    worker_queue: VecDeque<Task>,
    worker_active: bool,
    main: VecDeque<MainTask>,
    cache: RequestCache,
    cache_invalidations: u64,
    main_bodies_executed: u64,
    promotion_callbacks_executed: u64,
    closed: Option<NativeRequestError>,
    #[cfg(test)]
    admission_batches: Vec<(Vec<(ChunkPos, u8)>, Vec<(ChunkPos, ChunkStatus)>)>,
}

impl RequestState {
    fn new() -> Self {
        let owner = Arc::new(());
        Self {
            unloaded: ChunkFuture::completed(&owner, Ok(None)),
            owner,
            tickets: LoadingTickets::default(),
            holders: BTreeMap::new(),
            pending: VecDeque::new(),
            worker_queue: VecDeque::new(),
            worker_active: false,
            main: VecDeque::new(),
            cache: RequestCache::default(),
            cache_invalidations: 0,
            main_bodies_executed: 0,
            promotion_callbacks_executed: 0,
            closed: None,
            #[cfg(test)]
            admission_batches: Vec::new(),
        }
    }

    fn check_open(&self) -> Result<(), NativeRequestError> {
        self.closed
            .as_ref()
            .map_or(Ok(()), |error| Err(error.clone()))
    }

    fn has_main_work(&self) -> bool {
        self.tickets.has_pending() || !self.pending.is_empty() || !self.main.is_empty()
    }

    fn current(&self, task: Task) -> bool {
        self.closed.is_none()
            && self
                .holders
                .get(&(task.pos.x, task.pos.z))
                .is_some_and(|holder| {
                    holder.revision == task.revision && holder.target == Some(task.target)
                })
    }

    fn complete_accessibility(&mut self, pos: ChunkPos) {
        let Some(accessibility) = self
            .holders
            .get_mut(&(pos.x, pos.z))
            .and_then(|holder| holder.accessibility.as_mut())
        else {
            return;
        };
        if accessibility.future.is_done() {
            return;
        }
        let Some(result) = accessibility.ready_result() else {
            return;
        };
        let success = matches!(result, Ok(Some(_)));
        accessibility.future.complete(result);
        accessibility.dependencies = None;
        if success {
            accessibility.callback_queued = true;
            self.main.push_back(MainTask::Promotion(pos));
        }
    }

    fn refresh_accessibility_near(&mut self, pos: ChunkPos) {
        for dz in -1..=1 {
            for dx in -1..=1 {
                self.complete_accessibility(ChunkPos::new(pos.x + dx, pos.z + dz));
            }
        }
    }
}

impl Shared {
    fn schedule(
        &self,
        pos: ChunkPos,
        status: ChunkStatus,
    ) -> Result<ChunkFuture, NativeRequestError> {
        {
            let state = lock(&self.state);
            state.check_open()?;
            if let Some(future) = state
                .holders
                .get(&(pos.x, pos.z))
                .and_then(|holder| holder.futures[status.index()].as_ref())
                .filter(|future| future.is_done())
            {
                return Ok(future.clone());
            }
        }
        // Core inspection is outside the scheduler mutex: generation callbacks
        // must never invert the core/scheduler lock order.
        let available = self.world.returned_chunk(pos, status).map_err(|error| {
            let error = NativeRequestError::from(error);
            self.fail_holder(pos, error.clone());
            error
        })?;
        let mut state = lock(&self.state);
        state.check_open()?;
        let owner = state.owner.clone();
        let Some(holder) = state.holders.get_mut(&(pos.x, pos.z)) else {
            return Err(NativeRequestError::Admission { pos, status });
        };
        if !holder.allowed.is_some_and(|ceiling| status <= ceiling) {
            return Err(NativeRequestError::Admission { pos, status });
        }
        let future = holder.futures[status.index()]
            .get_or_insert_with(|| ChunkFuture::pending(&owner))
            .clone();
        if let Some(object) = available {
            future.complete(Ok(Some(object)));
            state.refresh_accessibility_near(pos);
            self.wake.notify_all();
            return Ok(future);
        }
        if future.is_done() {
            return Ok(future);
        }
        if let Some(error) = &holder.failure {
            future.complete(Err(error.clone()));
            state.refresh_accessibility_near(pos);
            self.wake.notify_all();
            return Ok(future);
        }
        if holder.target.is_none_or(|target| status > target) {
            holder.target = Some(status);
            holder.revision += 1;
            let revision = holder.revision;
            state.pending.push_back(Task {
                pos,
                target: status,
                revision,
            });
        }
        self.wake.notify_all();
        Ok(future)
    }

    fn publish(&self, pos: ChunkPos, status: ChunkStatus) -> Result<(), NativeRequestError> {
        let object = self
            .world
            .returned_chunk(pos, status)?
            .ok_or(NativeRequestError::Admission { pos, status })?;
        let mut state = lock(&self.state);
        if state.closed.is_some() {
            return Ok(());
        }
        let owner = state.owner.clone();
        let holder = state
            .holders
            .get_mut(&(pos.x, pos.z))
            .ok_or(NativeRequestError::Admission { pos, status })?;
        holder.futures[status.index()]
            .get_or_insert_with(|| ChunkFuture::pending(&owner))
            .complete(Ok(Some(object)));
        if status >= ChunkStatus::InitializeLight {
            state.refresh_accessibility_near(pos);
        }
        self.wake.notify_all();
        Ok(())
    }

    fn publish_full(&self, pos: ChunkPos) -> Result<(), NativeRequestError> {
        let level = self.world.returned_chunk(pos, ChunkStatus::Full)?.ok_or(
            NativeRequestError::Admission {
                pos,
                status: ChunkStatus::Full,
            },
        )?;
        let imposter = self.world.returned_chunk(pos, ChunkStatus::Light)?.ok_or(
            NativeRequestError::Admission {
                pos,
                status: ChunkStatus::Light,
            },
        )?;
        let mut state = lock(&self.state);
        if state.closed.is_some() {
            return Ok(());
        }
        let owner = state.owner.clone();
        let replacement = ChunkFuture::completed(&owner, Ok(Some(imposter)));
        let holder = state.holders.get_mut(&(pos.x, pos.z)).unwrap();
        // Replace the slot references. Never complete old lower-status futures
        // with another object or rewrite entries in the returned-object cache.
        for slot in &mut holder.futures[..ChunkStatus::Full.index()] {
            *slot = Some(replacement.clone());
        }
        if let Some(supplier) = &holder.full_supplier {
            supplier.complete(Ok(Some(level.clone())));
        }
        holder.futures[ChunkStatus::Full.index()]
            .get_or_insert_with(|| ChunkFuture::pending(&owner))
            .complete(Ok(Some(level)));
        state.refresh_accessibility_near(pos);
        self.wake.notify_all();
        Ok(())
    }

    fn fail_holder(&self, pos: ChunkPos, error: NativeRequestError) {
        if matches!(
            error,
            NativeRequestError::Generation(GenerationError::Poisoned)
        ) {
            self.close(error);
            return;
        }
        let mut state = lock(&self.state);
        if let Some(holder) = state.holders.get_mut(&(pos.x, pos.z)) {
            holder.failure = Some(error.clone());
            holder.target = None;
            holder.main_queued = false;
            for future in holder.futures.iter().flatten() {
                future.complete(Err(error.clone()));
            }
            if let Some(future) = &holder.full_supplier {
                future.complete(Err(error.clone()));
            }
        }
        state.refresh_accessibility_near(pos);
        self.wake.notify_all();
    }

    fn close(&self, error: NativeRequestError) {
        let mut state = lock(&self.state);
        if state.closed.is_none() {
            state.closed = Some(error.clone());
            for holder in state.holders.values_mut() {
                holder.target = None;
                holder.main_queued = false;
                for future in holder.futures.iter().flatten() {
                    future.complete(Err(error.clone()));
                }
                if let Some(future) = &holder.full_supplier {
                    future.complete(Err(error.clone()));
                }
                if let Some(accessibility) = &mut holder.accessibility {
                    accessibility.future.complete(Err(error.clone()));
                    accessibility.dependencies = None;
                    accessibility.callback_queued = false;
                }
            }
            state.pending.clear();
            state.worker_queue.clear();
            state.main.clear();
        }
        drop(state);
        #[cfg(test)]
        self.control.cancel();
        self.wake.notify_all();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
#[path = "request/tests.rs"]
mod tests;
