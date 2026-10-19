use crate::{Session, diagnostics::Record};
use serde::Serialize;
use touchbar_control::{Command, Reply};
use touchbar_model::{InstanceId, Timestamp};
#[derive(Clone, Debug, Serialize)]
pub struct Timing {
    pub samples: usize,
    pub p50: Option<f64>,
    pub p99: Option<f64>,
}
impl Timing {
    pub(crate) fn from(values: &std::collections::VecDeque<f64>) -> Self {
        let mut sorted = values.iter().copied().collect::<Vec<_>>();
        sorted.sort_by(f64::total_cmp);
        let percentile = |p: usize| {
            sorted
                .get((sorted.len() * p).div_ceil(100).saturating_sub(1))
                .copied()
        };
        Self {
            samples: sorted.len(),
            p50: percentile(50),
            p99: percentile(99),
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct StatusMetrics {
    pub frame_ms: Timing,
    pub wasm_ms: Timing,
}
#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub touchbar: Option<crate::touchbard::Status>,
    pub core: touchbar_core::CoreSnapshot,
    pub load_issues: Vec<touchbar_loader::LoadIssue>,
    pub last_calls: Vec<(InstanceId, Timestamp)>,
    pub metrics: StatusMetrics,
    pub logs: Vec<Record>,
}
impl Session {
    pub fn status(&self) -> Status {
        Status {
            touchbar: self.touchbard_status.clone(),
            core: self.snapshot(),
            load_issues: self.load_issues().into(),
            last_calls: self.last_calls.iter().map(|(i, t)| (*i, *t)).collect(),
            metrics: StatusMetrics {
                frame_ms: Timing::from(&self.metrics().frame_ms),
                wasm_ms: Timing::from(&self.metrics().wasm_ms),
            },
            logs: self.logs().iter().cloned().collect(),
        }
    }
    /// Reload acknowledges scheduling; admission and diagnostics appear in status.
    pub fn control(
        &mut self,
        command: Command,
        fake: Option<&touchbar_desktop::FakeBackend>,
    ) -> (Reply, bool) {
        let result: Result<Reply, String> = match command {
            Command::Status => serde_json::to_value(self.status())
                .map(Reply::Status)
                .map_err(|e| e.to_string()),
            Command::Dump => self.save_dump().map(Reply::Dump).map_err(|e| e.to_string()),
            Command::Reload => return (Reply::Ack, true),
            command => fake
                .ok_or_else(|| "sim commands require --desktop=fake".to_owned())
                .and_then(|fake| {
                    let (name, value) = match command {
                        Command::SimFact { name, value } => (name, value),
                        Command::SimTheme { theme } => (
                            "appearance.scheme".into(),
                            Some(touchbar_model::FactValue::Text(theme.as_str().into())),
                        ),
                        _ => unreachable!("non-sim handled above"),
                    };
                    fake.set_fact(name, value)
                        .map(|()| Reply::Ack)
                        .map_err(|e| e.to_string())
                }),
        };
        (result.unwrap_or_else(Reply::Error), false)
    }
}
