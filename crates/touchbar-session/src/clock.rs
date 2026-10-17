use std::{cell::Cell, rc::Rc, time::Instant};
use touchbar_model::Timestamp;
pub trait Clock {
    fn now(&self) -> Timestamp;
}
#[derive(Clone)]
pub struct MonotonicClock(Instant);
impl Default for MonotonicClock {
    fn default() -> Self {
        Self(Instant::now())
    }
}
impl Clock for MonotonicClock {
    fn now(&self) -> Timestamp {
        Timestamp(u64::try_from(self.0.elapsed().as_millis()).unwrap_or(u64::MAX))
    }
}
#[derive(Clone, Default)]
pub struct ManualClock(Rc<Cell<Timestamp>>);
impl ManualClock {
    pub fn set(&self, now: Timestamp) -> Result<(), String> {
        if now < self.0.get() {
            return Err("clock cannot move backwards".into());
        }
        self.0.set(now);
        Ok(())
    }
    pub fn advance(&self, ms: u64) {
        self.0.set(self.0.get().after(ms));
    }
}
impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        self.0.get()
    }
}
