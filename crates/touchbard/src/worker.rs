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
struct Queue<Q> {
    jobs: VecDeque<Job<Q>>,
    generation: u64,
    closed: bool,
}
struct Shared<Q> {
    queue: Mutex<Queue<Q>>,
    ready: Condvar,
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
    shared: Arc<Shared<Q>>,
    reply: Receiver<(bool, Completion<R>)>,
}
impl<Q: Send + 'static, R: Send + 'static> Worker<Q, R> {
    pub fn new(mut execute: impl FnMut(Q) -> R + Send + 'static) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue {
                jobs: VecDeque::new(),
                generation: 0,
                closed: false,
            }),
            ready: Condvar::new(),
        });
        let work = shared.clone();
        let (send, reply) = async_channel::bounded(5);
        std::thread::Builder::new()
            .name("touchbar-device".into())
            .spawn(move || {
                loop {
                    let job = {
                        let mut queue = work.queue.lock().unwrap();
                        while queue.jobs.is_empty() && !queue.closed {
                            queue = work.ready.wait(queue).unwrap();
                        }
                        if queue.closed {
                            break;
                        }
                        queue.jobs.pop_front().unwrap()
                    };
                    let value = execute(job.value);
                    {
                        let queue = work.queue.lock().unwrap();
                        if queue.closed {
                            break;
                        }
                        if !job.committed && job.generation != queue.generation {
                            continue;
                        }
                    }
                    if send
                        .send_blocking((
                            job.committed,
                            Completion {
                                generation: job.generation,
                                value,
                            },
                        ))
                        .is_err()
                    {
                        break;
                    }
                }
            })?;
        Ok(Self { shared, reply })
    }
    /// Mode work: a newer generation atomically cancels only queued mode jobs.
    pub fn try_submit(&mut self, generation: u64, value: Q) -> Result<(), SubmitError> {
        self.submit(generation, false, value)
    }
    /// System work: survives mode changes, sharing the same bounded FIFO and worker.
    /// `serial` identifies its completion independently of the mode generation.
    pub fn try_submit_committed(&mut self, serial: u64, value: Q) -> Result<(), SubmitError> {
        self.submit(serial, true, value)
    }
    pub fn advance_generation(&mut self, generation: u64) -> Result<(), SubmitError> {
        let mut queue = self.shared.queue.lock().unwrap();
        if queue.closed || self.reply.is_closed() {
            return Err(SubmitError::Closed);
        }
        if generation < queue.generation {
            return Err(SubmitError::Stale);
        }
        queue.generation = generation;
        queue
            .jobs
            .retain(|job| job.committed || job.generation == generation);
        Ok(())
    }
    fn submit(&mut self, generation: u64, committed: bool, value: Q) -> Result<(), SubmitError> {
        let mut queue = self.shared.queue.lock().unwrap();
        if queue.closed || self.reply.is_closed() {
            return Err(SubmitError::Closed);
        }
        if !committed {
            if generation < queue.generation {
                return Err(SubmitError::Stale);
            }
            if generation > queue.generation {
                queue.generation = generation;
                queue.jobs.retain(|job| job.committed);
            }
        }
        if queue.jobs.len() == 4 {
            return Err(SubmitError::Full);
        }
        queue.jobs.push_back(Job {
            generation,
            committed,
            value,
        });
        self.shared.ready.notify_one();
        Ok(())
    }
    pub async fn next(&self) -> Result<Completion<R>, WorkerClosed> {
        loop {
            let (committed, reply) = self.reply.recv().await.map_err(|_| WorkerClosed)?;
            if committed || reply.generation == self.shared.queue.lock().unwrap().generation {
                return Ok(reply);
            }
        }
    }
}
impl<Q, R> Drop for Worker<Q, R> {
    fn drop(&mut self) {
        let mut queue = self.shared.queue.lock().unwrap();
        queue.closed = true;
        queue.jobs.clear();
        self.shared.ready.notify_one();
        self.reply.close();
        // Never join: the policy's safety deadline must survive a blocked kernel operation.
    }
}
