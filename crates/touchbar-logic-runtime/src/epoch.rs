use crate::RuntimeError;
use std::{
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};
use wasmtime::Engine;

#[derive(Default)]
struct State {
    active: usize,
    stop: bool,
}
type Shared = Arc<(Mutex<State>, Condvar)>;
pub(crate) struct EpochTicker {
    shared: Shared,
    thread: Option<JoinHandle<()>>,
    #[cfg(test)]
    ticks: Arc<std::sync::atomic::AtomicUsize>,
}
pub(crate) struct ActiveCall {
    shared: Shared,
}
impl EpochTicker {
    pub(crate) fn new(engine: Engine) -> Result<Self, RuntimeError> {
        let shared: Shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let running = shared.clone();
        #[cfg(test)]
        let ticks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        #[cfg(test)]
        let thread_ticks = ticks.clone();
        let thread = thread::Builder::new()
            .name("touchbar-wasm-epoch".into())
            .spawn(move || {
                let (mutex, wake) = &*running;
                let mut state = mutex.lock().unwrap();
                loop {
                    while !state.stop && state.active == 0 {
                        state = wake.wait(state).unwrap();
                    }
                    if state.stop {
                        break;
                    }
                    let (next, timeout) =
                        wake.wait_timeout(state, Duration::from_millis(1)).unwrap();
                    state = next;
                    if timeout.timed_out() && !state.stop && state.active > 0 {
                        // Keep the mutex until the tick finishes: dropping the last guard
                        // guarantees that no subsequent idle tick can race with it.
                        engine.increment_epoch();
                        #[cfg(test)]
                        thread_ticks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            })
            .map_err(|e| RuntimeError(e.to_string()))?;
        Ok(Self {
            shared,
            thread: Some(thread),
            #[cfg(test)]
            ticks,
        })
    }
    pub(crate) fn enter(&self) -> ActiveCall {
        let (mutex, wake) = &*self.shared;
        let mut state = mutex.lock().unwrap();
        state.active += 1;
        wake.notify_one();
        ActiveCall {
            shared: self.shared.clone(),
        }
    }
}
impl Drop for ActiveCall {
    fn drop(&mut self) {
        let (mutex, wake) = &*self.shared;
        mutex.lock().unwrap().active -= 1;
        wake.notify_one();
    }
}
impl Drop for EpochTicker {
    fn drop(&mut self) {
        let (mutex, wake) = &*self.shared;
        mutex.lock().unwrap().stop = true;
        wake.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::atomic::Ordering, time::Instant};
    #[test]
    fn epoch_does_not_grow_without_guest_calls() {
        let ticker = EpochTicker::new(Engine::default()).unwrap();
        thread::sleep(Duration::from_millis(30));
        assert_eq!(ticker.ticks.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn epoch_stops_after_last_active_call_and_restarts() {
        let ticker = EpochTicker::new(Engine::default()).unwrap();
        for _ in 0..3 {
            let guard = ticker.enter();
            let before = ticker.ticks.load(Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_secs(2);
            while ticker.ticks.load(Ordering::SeqCst) == before {
                assert!(Instant::now() < deadline, "active ticker did not wake");
                thread::sleep(Duration::from_millis(1));
            }
            let nested = ticker.enter();
            drop(guard);
            // A remaining active guard still owns interruption.
            let before = ticker.ticks.load(Ordering::SeqCst);
            while ticker.ticks.load(Ordering::SeqCst) == before {
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(1));
            }
            drop(nested);
            let idle = ticker.ticks.load(Ordering::SeqCst);
            thread::sleep(Duration::from_millis(20));
            assert_eq!(ticker.ticks.load(Ordering::SeqCst), idle);
        }
    }
}
