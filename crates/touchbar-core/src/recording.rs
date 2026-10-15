use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use touchbar_model::Timestamp;

pub const DUMP_VERSION: u32 = 4;
pub const CHECKPOINT_INTERVAL: usize = 2000;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceEntry {
    pub at: Timestamp,
    pub input: Input,
    pub outputs: Vec<Output>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayDump {
    pub format_version: u32,
    pub checkpoint: CoreState,
    pub inputs: Vec<TraceEntry>,
}
#[derive(Debug)]
pub struct ReplayResult {
    pub core: Core,
    pub trace: Vec<TraceEntry>,
}
#[derive(Debug)]
pub enum ReplayError {
    Json(serde_json::Error),
    Version(u64),
    Restore(RestoreError),
    Divergence { index: usize },
    Limit,
}
impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(e) => write!(f, "invalid replay JSON: {e}"),
            Self::Version(v) => {
                write!(f, "unsupported dump version {v}; supported: {DUMP_VERSION}")
            }
            Self::Restore(e) => e.fmt(f),
            Self::Divergence { index } => write!(f, "replay outputs differ at input {index}"),
            Self::Limit => write!(f, "replay dump exceeds 4000 inputs"),
        }
    }
}
impl std::error::Error for ReplayError {}
impl ReplayDump {
    pub fn to_json(&self) -> Result<String, ReplayError> {
        serde_json::to_string_pretty(self).map_err(ReplayError::Json)
    }
    pub fn from_json(json: &str) -> Result<Self, ReplayError> {
        // Inspect versions before decoding a payload whose shape may have changed.
        let value: serde_json::Value = serde_json::from_str(json).map_err(ReplayError::Json)?;
        if let Some(version) = value.get("format_version").and_then(|v| v.as_u64())
            && version != u64::from(DUMP_VERSION)
        {
            return Err(ReplayError::Version(version));
        }
        if let Some(version) = value
            .get("checkpoint")
            .and_then(|v| v.get("format_version"))
            .and_then(|v| v.as_u64())
            && version != u64::from(CORE_STATE_VERSION)
        {
            let found = u32::try_from(version).unwrap_or(u32::MAX);
            return Err(ReplayError::Restore(RestoreError::Version {
                found,
                supported: CORE_STATE_VERSION,
            }));
        }
        // Deserialize directly from the stream: a Value would erase the special
        // JSON decoding of numeric BTreeMap keys in the complete core state.
        serde_json::from_str(json).map_err(ReplayError::Json)
    }
    pub fn replay(&self) -> Result<ReplayResult, ReplayError> {
        if self.format_version != DUMP_VERSION {
            return Err(ReplayError::Version(u64::from(self.format_version)));
        }
        if self.inputs.len() > 2 * CHECKPOINT_INTERVAL {
            return Err(ReplayError::Limit);
        }
        let mut core = Core::restore(self.checkpoint.clone()).map_err(ReplayError::Restore)?;
        let mut trace = vec![];
        for (index, entry) in self.inputs.iter().enumerate() {
            let outputs = core.handle(entry.at, entry.input.clone());
            if outputs != entry.outputs {
                return Err(ReplayError::Divergence { index });
            }
            trace.push(TraceEntry {
                at: entry.at,
                input: entry.input.clone(),
                outputs,
            });
        }
        Ok(ReplayResult { core, trace })
    }
}
#[derive(Clone, Debug)]
struct Checkpoint {
    position: usize,
    state: CoreState,
}
#[derive(Clone, Debug)]
pub struct Recorder {
    position: usize,
    checkpoints: VecDeque<Checkpoint>,
    entries: VecDeque<TraceEntry>,
}
impl Recorder {
    pub fn new(core: &Core) -> Self {
        Self {
            position: 0,
            checkpoints: VecDeque::from([Checkpoint {
                position: 0,
                state: core.checkpoint(),
            }]),
            entries: VecDeque::new(),
        }
    }
    pub fn handle(&mut self, core: &mut Core, at: Timestamp, input: Input) -> Vec<Output> {
        let outputs = core.handle(at, input.clone());
        self.entries.push_back(TraceEntry {
            at,
            input,
            outputs: outputs.clone(),
        });
        self.position += 1;
        if self.position.is_multiple_of(CHECKPOINT_INTERVAL) {
            self.checkpoints.push_back(Checkpoint {
                position: self.position,
                state: core.checkpoint(),
            });
            if self.checkpoints.len() > 2 {
                let previous = self.checkpoints.pop_front().expect("third checkpoint");
                let next = self.checkpoints.front().expect("retained checkpoint");
                for _ in previous.position..next.position {
                    self.entries.pop_front();
                }
            }
        }
        outputs
    }
    pub fn checkpoint_count(&self) -> usize {
        self.checkpoints.len()
    }
    pub fn dump(&self) -> ReplayDump {
        ReplayDump {
            format_version: DUMP_VERSION,
            checkpoint: self
                .checkpoints
                .front()
                .expect("initial checkpoint")
                .state
                .clone(),
            inputs: self.entries.iter().cloned().collect(),
        }
    }
}
