//! Backlight deadlines and the temporary native Fn driver override.
use crate::config::BacklightLevels;
use touchbar_model::Timestamp;
#[derive(Clone, Debug)]
pub struct Power {
    levels: BacklightLevels,
    last: Timestamp,
    dim: Option<u64>,
    idle: Option<u64>,
    lid: bool,
    limit: u16,
}
impl Power {
    pub fn new(levels: BacklightLevels, now: Timestamp) -> Self {
        Self {
            levels,
            last: now,
            dim: None,
            idle: None,
            lid: false,
            limit: u16::MAX,
        }
    }
    pub fn levels(&mut self, levels: BacklightLevels) {
        self.levels = levels;
    }
    pub fn set_timeouts(&mut self, dim: Option<u64>, idle: Option<u64>) {
        self.dim = dim;
        self.idle = idle;
    }
    pub fn activity(&mut self, now: Timestamp) {
        self.last = self.last.max(now);
    }
    pub fn lid(&mut self, closed: bool) {
        self.lid = closed;
    }
    pub fn limit(&mut self, value: u16) {
        self.limit = value;
    }
    pub fn level(&self, now: Timestamp) -> u8 {
        if self.lid {
            return 0;
        }
        let elapsed = now.elapsed_since(self.last);
        let level = if self.dim.is_some_and(|dim| elapsed >= dim) {
            if self
                .dim
                .zip(self.idle)
                .is_some_and(|(dim, idle)| elapsed >= dim.saturating_add(idle))
            {
                0
            } else {
                self.levels.dimmed
            }
        } else {
            self.levels.active
        };
        let cap = 2 * u32::from(self.limit) / u32::from(u16::MAX);
        level.min(u8::try_from(cap).expect("cap <= active u8"))
    }
    pub fn next_deadline(&self, now: Timestamp) -> Option<Timestamp> {
        if self.lid || self.level(now) == 0 {
            return None;
        }
        let dim = self.dim.map(|ms| self.last.after(ms));
        let off = self
            .dim
            .zip(self.idle)
            .map(|(dim, idle)| self.last.after(dim.saturating_add(idle)));
        [dim, off]
            .into_iter()
            .flatten()
            .filter(|at| *at > now)
            .min()
    }
}
#[derive(Clone, Debug, Default)]
pub struct Bridge {
    saved: Option<u8>,
}
impl Bridge {
    pub(super) fn active(&self) -> bool {
        self.saved.is_some()
    }
    pub fn reconcile(&mut self, native: bool, fn_down: bool, mode: Option<u8>) -> Option<u8> {
        if !native || !fn_down {
            return self.saved.take();
        }
        if self.saved.is_none()
            && let Some(mode @ 1..=2) = mode
        {
            self.saved = Some(mode);
            return Some(3 - mode);
        }
        None
    }
}
