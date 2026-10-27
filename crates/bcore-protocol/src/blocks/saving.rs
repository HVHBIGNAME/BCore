//! One ordered file writer per gameplay owner; completions carry durability acks.
use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::thread::{self, JoinHandle};

use crate::chunk_store::ChunkStore;
use crate::shared::PlayerId;
use crate::world_state::{DirtyBatch, WorldError};

pub(super) struct SaveJob {
    pub batch: DirtyBatch,
    pub acks: BTreeMap<PlayerId, i32>,
}

pub(super) struct SaveCompletion {
    pub job: SaveJob,
    pub result: Result<(), WorldError>,
}

pub(super) struct SaveWorker {
    sender: Option<SyncSender<SaveJob>>,
    receiver: Receiver<SaveCompletion>,
    worker: Option<JoinHandle<()>>,
    active: bool,
}

impl SaveWorker {
    pub(super) fn new(store: Option<ChunkStore>) -> Result<Self, WorldError> {
        Self::with_writer(move |batch| batch.write(store.as_ref()))
    }

    fn with_writer(
        mut write: impl FnMut(&DirtyBatch) -> Result<(), WorldError> + Send + 'static,
    ) -> Result<Self, WorldError> {
        let (sender, jobs) = mpsc::sync_channel::<SaveJob>(1);
        let (completed, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("bcore-save".into())
            .spawn(move || {
                for job in jobs {
                    let result = write(&job.batch);
                    if completed.send(SaveCompletion { job, result }).is_err() {
                        break;
                    }
                }
            })
            .map_err(crate::chunk_store::ChunkStoreError::from)?;
        Ok(Self {
            sender: Some(sender),
            receiver,
            worker: Some(worker),
            active: false,
        })
    }

    pub(super) fn active(&self) -> bool {
        self.active
    }

    pub(super) fn start(&mut self, job: SaveJob) -> Result<(), WorldError> {
        assert!(!self.active, "single in-flight save batch");
        self.sender
            .as_ref()
            .expect("running save worker")
            .try_send(job)
            .map_err(|_| WorldError::Mutation("save worker unavailable"))?;
        self.active = true;
        Ok(())
    }

    pub(super) fn poll(&mut self) -> Result<Option<SaveCompletion>, WorldError> {
        if !self.active {
            return Ok(None);
        }
        match self.receiver.try_recv() {
            Ok(done) => {
                self.active = false;
                Ok(Some(done))
            }
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(WorldError::Mutation("save worker stopped")),
        }
    }

    pub(super) fn finish(&mut self) -> Result<Option<SaveCompletion>, WorldError> {
        if !self.active {
            return Ok(None);
        }
        let done = self
            .receiver
            .recv()
            .map_err(|_| WorldError::Mutation("save worker stopped"))?;
        self.active = false;
        Ok(Some(done))
    }
}

impl Drop for SaveWorker {
    fn drop(&mut self) {
        self.sender.take();
        if self
            .worker
            .take()
            .is_some_and(|worker| worker.join().is_err())
        {
            eprintln!("[bcore] save worker panicked");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_disk_keeps_completion_pending_without_blocking_the_tick_poll() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut worker = SaveWorker::with_writer(move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        })
        .unwrap();
        worker
            .start(SaveJob {
                batch: DirtyBatch::default(),
                acks: BTreeMap::from([(1, 42)]),
            })
            .unwrap();
        started_rx.recv().unwrap();
        for _ in 0..100 {
            assert!(worker.poll().unwrap().is_none());
        }
        release_tx.send(()).unwrap();
        let complete = worker.finish().unwrap().unwrap();
        complete.result.unwrap();
        assert_eq!(complete.job.acks[&1], 42);
        assert!(!worker.active());
    }
}
