//! One blocking operation in flight and four queued. Idle uses channel notifications only.
use async_channel::{Receiver, Sender, TrySendError};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
struct Job<Q> {
    generation: u64,
    value: Q,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Completion<R> {
    pub generation: u64,
    pub value: R,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitError {
    Full,
    Stale,
    Closed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkerClosed;
pub struct Worker<Q, R> {
    request: Sender<Job<Q>>,
    purge: Receiver<Job<Q>>,
    reply: Receiver<Completion<R>>,
    generation: Arc<AtomicU64>,
    closed: Arc<AtomicBool>,
}
impl<Q: Send + 'static, R: Send + 'static> Worker<Q, R> {
    pub fn new(mut execute: impl FnMut(Q) -> R + Send + 'static) -> io::Result<Self> {
        let (request, receive) = async_channel::bounded::<Job<Q>>(4);
        let purge = receive.clone();
        let (send, reply) = async_channel::bounded(5);
        let generation = Arc::new(AtomicU64::new(0));
        let epoch = generation.clone();
        let closed = Arc::new(AtomicBool::new(false));
        let stopping = closed.clone();
        std::thread::Builder::new()
            .name("touchbar-device".into())
            .spawn(move || {
                while let Ok(job) = receive.recv_blocking() {
                    if stopping.load(Ordering::Acquire) {
                        break;
                    }
                    if job.generation != epoch.load(Ordering::Acquire) {
                        continue;
                    }
                    let value = execute(job.value);
                    if stopping.load(Ordering::Acquire) {
                        break;
                    }
                    if job.generation != epoch.load(Ordering::Acquire) {
                        continue;
                    }
                    if send
                        .send_blocking(Completion {
                            generation: job.generation,
                            value,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })?;
        Ok(Self {
            request,
            purge,
            reply,
            generation,
            closed,
        })
    }
    pub fn try_submit(&mut self, generation: u64, value: Q) -> Result<(), SubmitError> {
        if self.closed.load(Ordering::Acquire) || self.request.is_closed() {
            return Err(SubmitError::Closed);
        }
        let previous = self.generation.load(Ordering::Acquire);
        if generation < previous {
            return Err(SubmitError::Stale);
        }
        if generation > previous {
            self.generation.store(generation, Ordering::Release);
            while self.purge.try_recv().is_ok() {}
            while self.reply.try_recv().is_ok() {}
        }
        self.request
            .try_send(Job { generation, value })
            .map_err(|error| match error {
                TrySendError::Full(_) => SubmitError::Full,
                TrySendError::Closed(_) => SubmitError::Closed,
            })
    }
    pub async fn next(&self) -> Result<Completion<R>, WorkerClosed> {
        loop {
            let reply = self.reply.recv().await.map_err(|_| WorkerClosed)?;
            if reply.generation == self.generation.load(Ordering::Acquire) {
                return Ok(reply);
            }
        }
    }
}
impl<Q, R> Drop for Worker<Q, R> {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        self.request.close();
        self.reply.close();
        // Never join here: a kernel operation may still be blocked. The policy's safety deadline
        // must remain effective. The worker drops all queued tasks once that operation returns.
    }
}
