//! One blocking operation in flight and four queued; idle waits on notifications only.
use async_channel::Receiver;
use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Condvar, Mutex},
};
struct Job<Q> {
    generation: u64,
    committed: bool,
    value: Q,
}
struct Queue<Q, R> {
    jobs: VecDeque<Job<Q>>,
    replies: VecDeque<(bool, Completion<R>)>,
    generation: u64,
    closed: bool,
}
struct Shared<Q, R> {
    queue: Mutex<Queue<Q, R>>,
    ready: Condvar,
    space: Condvar,
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
    shared: Arc<Shared<Q, R>>,
    wake: Receiver<()>,
}
impl<Q: Send + 'static, R: Send + 'static> Worker<Q, R> {
    pub fn new(mut execute: impl FnMut(Q) -> R + Send + 'static) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue {
                jobs: VecDeque::<Job<Q>>::new(),
                replies: VecDeque::new(),
                generation: 0,
                closed: false,
            }),
            ready: Condvar::new(),
            space: Condvar::new(),
        });
        let work = shared.clone();
        let (send, wake) = async_channel::bounded(1);
        std::thread::Builder::new()
            .name("touchbar-device".into())
            .spawn(move || {
                loop {
                    let job = {
                        let mut q = work.queue.lock().unwrap();
                        while q.jobs.is_empty() && !q.closed {
                            q = work.ready.wait(q).unwrap();
                        }
                        if q.closed {
                            break;
                        }
                        q.jobs.pop_front().unwrap()
                    };
                    let value = execute(job.value);
                    let mut q = work.queue.lock().unwrap();
                    while q.replies.len() == 5
                        && !q.closed
                        && (job.committed || job.generation == q.generation)
                    {
                        q = work.space.wait(q).unwrap();
                    }
                    if q.closed {
                        break;
                    }
                    if !job.committed && job.generation != q.generation {
                        continue;
                    }
                    q.replies.push_back((
                        job.committed,
                        Completion {
                            generation: job.generation,
                            value,
                        },
                    ));
                    let _ = send.try_send(());
                }
            })?;
        Ok(Self { shared, wake })
    }
    /// Mode work: a newer generation atomically cancels queued mode jobs and results.
    pub fn try_submit(&mut self, generation: u64, value: Q) -> Result<(), SubmitError> {
        self.submit(generation, false, value)
    }
    /// System work survives mode changes, sharing the same bounded FIFO and worker.
    pub fn try_submit_committed(&mut self, serial: u64, value: Q) -> Result<(), SubmitError> {
        self.submit(serial, true, value)
    }
    fn advance(q: &mut Queue<Q, R>, generation: u64) -> Result<(), SubmitError> {
        if generation < q.generation {
            return Err(SubmitError::Stale);
        }
        q.generation = generation;
        q.jobs
            .retain(|job| job.committed || job.generation == generation);
        // Drop stale exported descriptors before any new-generation operation can start.
        // Requests, replies and epoch share one lock; committed replies retain FIFO order.
        q.replies
            .retain(|(committed, reply)| *committed || reply.generation == generation);
        Ok(())
    }
    pub fn advance_generation(&mut self, generation: u64) -> Result<(), SubmitError> {
        let mut q = self.shared.queue.lock().unwrap();
        if q.closed || self.wake.is_closed() {
            return Err(SubmitError::Closed);
        }
        Self::advance(&mut q, generation)?;
        self.shared.space.notify_all();
        Ok(())
    }
    fn submit(&mut self, generation: u64, committed: bool, value: Q) -> Result<(), SubmitError> {
        let mut q = self.shared.queue.lock().unwrap();
        if q.closed || self.wake.is_closed() {
            return Err(SubmitError::Closed);
        }
        if !committed {
            Self::advance(&mut q, generation)?;
            self.shared.space.notify_all();
        }
        if q.jobs.len() == 4 {
            return Err(SubmitError::Full);
        }
        q.jobs.push_back(Job {
            generation,
            committed,
            value,
        });
        self.shared.ready.notify_one();
        Ok(())
    }
    fn pop(&self) -> Result<Option<Completion<R>>, WorkerClosed> {
        let mut q = self.shared.queue.lock().unwrap();
        if q.closed {
            return Err(WorkerClosed);
        }
        while let Some((committed, reply)) = q.replies.pop_front() {
            self.shared.space.notify_one();
            if committed || reply.generation == q.generation {
                return Ok(Some(reply));
            }
        }
        Ok(None)
    }
    pub async fn next(&mut self) -> Result<Completion<R>, WorkerClosed> {
        loop {
            if let Some(reply) = self.pop()? {
                return Ok(reply);
            }
            self.wake.recv().await.map_err(|_| WorkerClosed)?;
        }
    }
}
impl<Q, R> Drop for Worker<Q, R> {
    fn drop(&mut self) {
        let mut q = self.shared.queue.lock().unwrap();
        q.closed = true;
        q.jobs.clear();
        q.replies.clear();
        self.shared.ready.notify_all();
        self.shared.space.notify_all();
        self.wake.close();
        // Never join: the policy's safety deadline must survive a blocked kernel operation.
    }
}
