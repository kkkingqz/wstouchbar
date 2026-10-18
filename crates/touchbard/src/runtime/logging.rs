//! Journald captures stderr; the configured tracing filter also applies after reload.
use std::io;
use tracing_subscriber::{EnvFilter, Registry, prelude::*, reload};
pub type Filter = reload::Handle<EnvFilter, Registry>;
pub fn init(filter: &str) -> io::Result<Filter> {
    let filter = EnvFilter::try_new(filter).map_err(io::Error::other)?;
    let (layer, handle) = reload::Layer::new(filter);
    let journal = std::env::var_os("JOURNAL_STREAM").and_then(|_| tracing_journald::layer().ok());
    let stderr = journal.is_none().then(|| {
        tracing_subscriber::fmt::layer()
            .with_writer(io::stderr)
            .with_ansi(false)
    });
    tracing_subscriber::registry()
        .with(layer)
        .with(journal)
        .with(stderr)
        .try_init()
        .map_err(io::Error::other)?;
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use touchbar_model::Timestamp;
    #[test]
    fn repeated_errors_have_one_pending_minute_summary_without_idle_ticks() {
        let mut errors = Repeats::default();
        assert_eq!(errors.record(Timestamp(0), "drm", "gone").len(), 1);
        assert!(errors.deadline().is_none());
        for i in 1..100 {
            assert!(errors.record(Timestamp(i), "drm", "gone").is_empty());
        }
        assert_eq!(errors.deadline(), Some(Timestamp(60_000)));
        assert!(errors.take_due(Timestamp(59_999)).is_empty());
        let reports = errors.take_due(Timestamp(60_000));
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].suppressed, 99);
        assert!(errors.deadline().is_none());
        assert_eq!(
            errors.record(Timestamp(60_001), "drm", "different").len(),
            1
        );
    }
}

use std::collections::BTreeMap;
use touchbar_model::Timestamp;
struct Repeat {
    message: String,
    last: Timestamp,
    count: u64,
}
pub(crate) struct Report {
    source: String,
    message: String,
    pub suppressed: u64,
}
impl Report {
    pub fn emit(self) {
        tracing::warn!(source=%self.source,error=%self.message,suppressed=self.suppressed,"daemon diagnostic");
    }
}
#[derive(Default)]
pub(crate) struct Repeats {
    entries: BTreeMap<String, Repeat>,
}
impl Repeats {
    pub fn record(&mut self, now: Timestamp, source: &str, message: &str) -> Vec<Report> {
        let message = message.chars().take(1024).collect::<String>();
        let mut reports = Vec::new();
        if let Some(entry) = self.entries.get_mut(source) {
            if entry.message == message {
                entry.count = entry.count.saturating_add(1);
                return self.take_due(now);
            }
            if entry.count > 0 {
                reports.push(Report {
                    source: source.into(),
                    message: entry.message.clone(),
                    suppressed: entry.count,
                });
            }
        }
        if self.entries.len() == 32 && !self.entries.contains_key(source) {
            self.entries.pop_first();
        }
        self.entries.insert(
            source.into(),
            Repeat {
                message: message.clone(),
                last: now,
                count: 0,
            },
        );
        reports.push(Report {
            source: source.into(),
            message,
            suppressed: 0,
        });
        reports
    }
    pub fn deadline(&self) -> Option<Timestamp> {
        self.entries
            .values()
            .filter(|e| e.count > 0)
            .map(|e| e.last.after(60_000))
            .min()
    }
    pub fn take_due(&mut self, now: Timestamp) -> Vec<Report> {
        let mut reports = Vec::new();
        for (source, e) in &mut self.entries {
            if e.count > 0 && e.last.after(60_000) <= now {
                reports.push(Report {
                    source: source.clone(),
                    message: e.message.clone(),
                    suppressed: e.count,
                });
                e.count = 0;
                e.last = now;
            }
        }
        reports
    }
}
