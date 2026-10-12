use touchbar_logic_types::LogicLimits;
use wasmtime::{ResourceLimiter, StoreLimits, StoreLimitsBuilder};
/// StoreLimits bounds each memory; this wrapper also bounds their aggregate.
pub(super) struct MemoryLimiter {
    inner: StoreLimits,
    maximum: usize,
    used: usize,
    pending: usize,
    pub denied: bool,
}
impl MemoryLimiter {
    pub fn new(limits: &LogicLimits) -> Self {
        Self {
            inner: StoreLimitsBuilder::new()
                .memory_size(limits.max_memory_bytes)
                .build(),
            maximum: limits.max_memory_bytes,
            used: 0,
            pending: 0,
            denied: false,
        }
    }
}
impl ResourceLimiter for MemoryLimiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        self.pending = 0;
        let delta = desired.saturating_sub(current);
        let allowed = self.inner.memory_growing(current, desired, maximum)?
            && self.used.saturating_add(delta) <= self.maximum;
        if allowed {
            self.used = self.used.saturating_add(delta);
            self.pending = delta;
        } else {
            self.denied = true;
        }
        Ok(allowed)
    }
    fn memory_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        self.used = self.used.saturating_sub(self.pending);
        self.pending = 0;
        self.denied = true;
        self.inner.memory_grow_failed(error)
    }
    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        self.inner.table_growing(current, desired, maximum)
    }
    fn table_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        self.inner.table_grow_failed(error)
    }
    fn instances(&self) -> usize {
        self.inner.instances()
    }
    fn tables(&self) -> usize {
        self.inner.tables()
    }
    fn memories(&self) -> usize {
        self.inner.memories()
    }
}
pub(super) fn fault_reason(error: &wasmtime::Error, memory_denied: bool) -> String {
    if memory_denied {
        format!("memory: {error:#}")
    } else {
        match error.downcast_ref::<wasmtime::Trap>() {
            Some(wasmtime::Trap::Interrupt) => "timeout".into(),
            Some(wasmtime::Trap::StackOverflow) => format!("stack: {error:#}"),
            _ => format!("trap: {error:#}"),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn total_memory_and_failed_growth_are_accounted() {
        let mut l = MemoryLimiter::new(&LogicLimits {
            max_memory_bytes: 128 * 1024,
            ..Default::default()
        });
        assert!(l.memory_growing(0, 64 * 1024, None).unwrap());
        assert!(l.memory_growing(0, 64 * 1024, None).unwrap());
        assert!(!l.memory_growing(0, 64 * 1024, None).unwrap());
        assert!(l.denied);
        let mut l = MemoryLimiter::new(&LogicLimits {
            max_memory_bytes: 64 * 1024,
            ..Default::default()
        });
        assert!(l.memory_growing(0, 64 * 1024, None).unwrap());
        l.memory_grow_failed(wasmtime::format_err!("allocation failure"))
            .unwrap();
        assert!(l.memory_growing(0, 64 * 1024, None).unwrap());
    }
}
