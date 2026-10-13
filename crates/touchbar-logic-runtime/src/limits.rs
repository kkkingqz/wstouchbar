use touchbar_logic_types::LogicLimits;
use wasmtime::{ResourceLimiter, StoreLimits, StoreLimitsBuilder};
/// Bounds linear memory bytes and table elements separately, across the store.
pub(super) struct MemoryLimiter {
    inner: StoreLimits,
    maximum: usize,
    used: usize,
    pending: usize,
    table_maximum: usize,
    table_used: usize,
    table_pending: usize,
    pub denied: Option<&'static str>,
}
impl MemoryLimiter {
    pub fn new(limits: &LogicLimits) -> Self {
        Self {
            inner: StoreLimitsBuilder::new()
                .memory_size(limits.max_memory_bytes)
                .table_elements(limits.max_table_elements)
                .build(),
            maximum: limits.max_memory_bytes,
            used: 0,
            pending: 0,
            table_maximum: limits.max_table_elements,
            table_used: 0,
            table_pending: 0,
            denied: None,
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
            self.denied = Some("memory");
        }
        Ok(allowed)
    }
    fn memory_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        self.used = self.used.saturating_sub(self.pending);
        self.pending = 0;
        self.denied = Some("memory");
        self.inner.memory_grow_failed(error)
    }
    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        self.table_pending = 0;
        let delta = desired.saturating_sub(current);
        let allowed = self.inner.table_growing(current, desired, maximum)?
            && self.table_used.saturating_add(delta) <= self.table_maximum;
        if allowed {
            self.table_used = self.table_used.saturating_add(delta);
            self.table_pending = delta;
        } else {
            self.denied = Some("table");
        }
        Ok(allowed)
    }
    fn table_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        self.table_used = self.table_used.saturating_sub(self.table_pending);
        self.table_pending = 0;
        self.denied = Some("table");
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
pub(super) fn fault_reason(error: &wasmtime::Error, denied: Option<&str>) -> String {
    if let Some(resource) = denied {
        format!("{resource}: {error:#}")
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
    fn tables_share_budget_and_failed_growth_refunds_it() {
        let mut l = MemoryLimiter::new(&LogicLimits {
            max_table_elements: 128,
            ..Default::default()
        });
        assert!(l.table_growing(0, 64, None).unwrap());
        assert!(l.table_growing(0, 64, None).unwrap());
        assert!(!l.table_growing(64, 65, None).unwrap());
        let mut l = MemoryLimiter::new(&LogicLimits {
            max_table_elements: 128,
            ..Default::default()
        });
        assert!(l.table_growing(0, 64, None).unwrap());
        l.table_grow_failed(wasmtime::format_err!("allocation failure"))
            .unwrap();
        assert!(l.table_growing(0, 128, None).unwrap());
        assert!(!l.table_growing(0, 1, None).unwrap());
    }
    #[test]
    fn total_memory_and_failed_growth_are_accounted() {
        let mut l = MemoryLimiter::new(&LogicLimits {
            max_memory_bytes: 128 * 1024,
            ..Default::default()
        });
        assert!(l.memory_growing(0, 64 * 1024, None).unwrap());
        assert!(l.memory_growing(0, 64 * 1024, None).unwrap());
        assert!(!l.memory_growing(0, 64 * 1024, None).unwrap());
        assert_eq!(l.denied, Some("memory"));
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
