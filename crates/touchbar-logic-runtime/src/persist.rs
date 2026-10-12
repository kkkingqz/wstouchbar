use crate::LogRecord;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use touchbar_core::{PersistKey, PersistScope};
use touchbar_logic_types::LogicLimits;
use touchbar_model::{StatePatch, StateValue};
#[derive(Debug, thiserror::Error)]
#[error("persistent state: {0}")]
pub struct StoreError(pub String);
pub trait PersistStore {
    fn load(&mut self, key: &PersistKey) -> Result<StatePatch, StoreError>;
    fn write(&mut self, key: &PersistKey, fields: &StatePatch) -> Result<(), StoreError>;
    fn take_logs(&mut self) -> Vec<LogRecord> {
        vec![]
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    fields: BTreeMap<String, StateValue>,
}
pub(super) fn merged(current: StatePatch, patch: &StatePatch) -> BTreeMap<String, StateValue> {
    let mut fields: BTreeMap<_, _> = current.into_iter().collect();
    for (k, v) in patch {
        fields.insert(k.clone(), v.clone());
    }
    fields
}
pub(super) fn encoded(
    fields: &BTreeMap<String, StateValue>,
    limit: usize,
) -> Result<Vec<u8>, StoreError> {
    let bytes = serde_json::to_vec(&Document {
        version: 1,
        fields: fields.clone(),
    })
    .map_err(|e| StoreError(e.to_string()))?;
    if bytes.len() > limit {
        Err(StoreError(format!(
            "size {} exceeds {limit} bytes",
            bytes.len()
        )))
    } else {
        Ok(bytes)
    }
}
fn stable_key(key: &PersistKey) -> Result<String, StoreError> {
    serde_json::to_string(key).map_err(|e| StoreError(e.to_string()))
}
/// Clones share state, so a second Runtime can simulate a session restart.
#[derive(Clone)]
pub struct MemoryStore {
    data: Arc<Mutex<BTreeMap<String, BTreeMap<String, StateValue>>>>,
    limit: usize,
}
impl Default for MemoryStore {
    fn default() -> Self {
        Self::with_limit(LogicLimits::default().max_persist_bytes)
    }
}
impl MemoryStore {
    pub fn with_limit(limit: usize) -> Self {
        Self {
            data: Arc::default(),
            limit,
        }
    }
}
impl PersistStore for MemoryStore {
    fn load(&mut self, key: &PersistKey) -> Result<StatePatch, StoreError> {
        let key = stable_key(key)?;
        Ok(self
            .data
            .lock()
            .map_err(|e| StoreError(e.to_string()))?
            .get(&key)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect())
    }
    fn write(&mut self, key: &PersistKey, patch: &StatePatch) -> Result<(), StoreError> {
        let key = stable_key(key)?;
        let mut data = self.data.lock().map_err(|e| StoreError(e.to_string()))?;
        let fields = merged(
            data.get(&key)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect(),
            patch,
        );
        encoded(&fields, self.limit)?;
        data.insert(key, fields);
        Ok(())
    }
}
/// A sequential writer under XDG_STATE_HOME. Only renaming a synced complete file
/// publishes an update; readers never observe a partial document.
pub struct FsStore {
    state_home: PathBuf,
    limit: usize,
    logs: Vec<LogRecord>,
}
impl FsStore {
    pub fn new(state_home: impl Into<PathBuf>) -> Self {
        Self::with_limit(state_home, LogicLimits::default().max_persist_bytes)
    }
    pub fn with_limit(state_home: impl Into<PathBuf>, limit: usize) -> Self {
        Self {
            state_home: state_home.into(),
            limit,
            logs: vec![],
        }
    }
    pub fn from_env() -> Result<Self, StoreError> {
        let home = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .map(|p| p.join(".local/state"))
            })
            .ok_or_else(|| StoreError("no absolute XDG_STATE_HOME or HOME".into()))?;
        Ok(Self::new(home))
    }
    pub fn path(&self, key: &PersistKey) -> PathBuf {
        let mut p = self
            .state_home
            .join("touchbar")
            .join(key.plugin.as_str())
            .join(key.surface.as_str());
        let set = match &key.scope {
            PersistScope::ProfileSlot { slot, set } => {
                p.push("slots");
                p.push(slot.as_str());
                set
            }
            PersistScope::Layer(set) => {
                p.push("layers");
                set
            }
        };
        p.join(format!("{}.json", set.id()))
    }
    pub fn take_logs(&mut self) -> Vec<LogRecord> {
        std::mem::take(&mut self.logs)
    }
    fn corrupted(&mut self, path: &Path, reason: impl std::fmt::Display) -> StatePatch {
        self.logs.push(LogRecord::warning(format!(
            "ignoring persistent state {}: {reason}",
            path.display()
        )));
        vec![]
    }
}
impl PersistStore for FsStore {
    fn load(&mut self, key: &PersistKey) -> Result<StatePatch, StoreError> {
        let path = self.path(key);
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(io_error(&path, e)),
        };
        let max = u64::try_from(self.limit)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let mut bytes = vec![];
        file.take(max)
            .read_to_end(&mut bytes)
            .map_err(|e| io_error(&path, e))?;
        if bytes.len() > self.limit {
            return Ok(self.corrupted(&path, "size exceeds limit"));
        }
        let document: Document = match serde_json::from_slice(&bytes) {
            Ok(d) => d,
            Err(e) => return Ok(self.corrupted(&path, e)),
        };
        if document.version != 1 {
            return Ok(self.corrupted(&path, format!("unsupported version {}", document.version)));
        }
        Ok(document.fields.into_iter().collect())
    }
    fn write(&mut self, key: &PersistKey, patch: &StatePatch) -> Result<(), StoreError> {
        let fields = merged(self.load(key)?, patch);
        let bytes = encoded(&fields, self.limit)?;
        let path = self.path(key);
        let parent = path
            .parent()
            .ok_or_else(|| StoreError("missing parent".into()))?;
        fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let (temporary, mut file) = loop {
            let temporary = parent.join(format!(
                ".{}.{}.{}.tmp",
                path.file_name().unwrap_or_default().to_string_lossy(),
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
            {
                Ok(file) => break (temporary, file),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(io_error(&temporary, e)),
            }
        };
        let result = (|| {
            file.write_all(&bytes)
                .map_err(|e| io_error(&temporary, e))?;
            file.sync_all().map_err(|e| io_error(&temporary, e))?;
            drop(file);
            fs::rename(&temporary, &path).map_err(|e| io_error(&path, e))?;
            File::open(parent)
                .and_then(|d| d.sync_all())
                .map_err(|e| io_error(parent, e))
        })(); // also sync directory metadata after rename
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
    fn take_logs(&mut self) -> Vec<LogRecord> {
        self.take_logs()
    }
}
fn io_error(path: &Path, error: std::io::Error) -> StoreError {
    StoreError(format!("{}: {error}", path.display()))
}
