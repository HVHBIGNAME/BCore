use std::sync::{Arc, Condvar, Mutex};

use super::{lock, NativeRequestError};
use crate::generation::ReturnedChunk;

pub(super) type Outcome = Result<Option<ReturnedChunk>, NativeRequestError>;

#[derive(Debug)]
struct Completion {
    result: Mutex<Option<Outcome>>,
    ready: Condvar,
}

/// An immutable future identity. FULL installs new lower-status slot futures;
/// callers holding this instance still receive the original proto object.
#[derive(Debug, Clone)]
pub struct ChunkFuture {
    owner: Arc<()>,
    completion: Arc<Completion>,
}

impl ChunkFuture {
    pub(super) fn pending(owner: &Arc<()>) -> Self {
        Self {
            owner: owner.clone(),
            completion: Arc::new(Completion {
                result: Mutex::new(None),
                ready: Condvar::new(),
            }),
        }
    }

    pub(super) fn completed(owner: &Arc<()>, result: Outcome) -> Self {
        let future = Self::pending(owner);
        future.complete(result);
        future
    }

    pub fn same_future(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.completion, &other.completion)
    }

    pub fn is_done(&self) -> bool {
        lock(&self.completion.result).is_some()
    }

    pub fn get_now(&self) -> Option<Outcome> {
        lock(&self.completion.result).clone()
    }

    /// Wait without pumping the main executor. The owner thread should use
    /// NativeChunkRequests::wait instead when the requested stage needs main work.
    pub fn join(&self) -> Outcome {
        let mut result = lock(&self.completion.result);
        while result.is_none() {
            result = self
                .completion
                .ready
                .wait(result)
                .unwrap_or_else(|e| e.into_inner());
        }
        result.as_ref().unwrap().clone()
    }

    pub(super) fn belongs_to(&self, owner: &Arc<()>) -> bool {
        Arc::ptr_eq(&self.owner, owner)
    }

    pub(super) fn complete(&self, result: Outcome) {
        let mut slot = lock(&self.completion.result);
        if slot.is_none() {
            *slot = Some(result);
            self.completion.ready.notify_all();
        }
    }
}
