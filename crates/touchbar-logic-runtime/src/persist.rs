use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use touchbar_core::PersistKey;
use touchbar_model::StatePatch;
#[derive(Debug, thiserror::Error)]
#[error("persistent state: {0}")]
pub struct StoreError(pub String);
pub trait PersistStore {
    fn read(&mut self, key: &PersistKey) -> Result<StatePatch, StoreError>;
    fn write(&mut self, key: &PersistKey, fields: &StatePatch) -> Result<(), StoreError>;
}
/// Clones share state, so a second Runtime can simulate a session restart.
#[derive(Clone, Default)]
pub struct MemoryStore(Arc<Mutex<BTreeMap<String, StatePatch>>>);
impl PersistStore for MemoryStore {
    fn read(&mut self, key: &PersistKey) -> Result<StatePatch, StoreError> {
        let key = serde_json::to_string(key).map_err(|e| StoreError(e.to_string()))?;
        Ok(self
            .0
            .lock()
            .map_err(|e| StoreError(e.to_string()))?
            .get(&key)
            .cloned()
            .unwrap_or_default())
    }
    fn write(&mut self, key: &PersistKey, fields: &StatePatch) -> Result<(), StoreError> {
        let key = serde_json::to_string(key).map_err(|e| StoreError(e.to_string()))?;
        let mut all = self.0.lock().map_err(|e| StoreError(e.to_string()))?;
        let state = all.entry(key).or_default();
        for (name, value) in fields {
            if let Some((_, old)) = state.iter_mut().find(|(n, _)| n == name) {
                *old = value.clone();
            } else {
                state.push((name.clone(), value.clone()));
            }
        }
        Ok(())
    }
}
