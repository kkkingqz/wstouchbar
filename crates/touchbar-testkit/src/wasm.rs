use crate::*;
use std::{collections::BTreeSet, path::Path, time::Duration};
use touchbar_core::{Input, PersistKey};
use touchbar_logic_runtime::{
    LogRecord, MemoryStore, PersistStore, Runtime, RuntimeConfig, RuntimeError,
};
use touchbar_model::*;

pub struct WasmLogic {
    pub fallback: FakeLogic,
    pub calls: Vec<(InstanceId, CallId, LogicCall)>,
    pub logs: Vec<LogRecord>,
    pub durations: Vec<(InstanceId, CallId, Duration)>,
    runtime: Runtime,
    store: MemoryStore,
    registered: BTreeSet<PluginId>,
    instances: BTreeSet<InstanceId>,
}
impl Default for WasmLogic {
    fn default() -> Self {
        Self::new()
    }
}
impl WasmLogic {
    pub fn new() -> Self {
        let mut config = RuntimeConfig::default();
        let root = std::env::var_os("CARGO_TARGET_DIR")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"));
        config.cache_dir = Some(root.join("testkit-wasm-cache"));
        Self::with_config(config).expect("test logic runtime")
    }
    pub fn with_config(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        let store = MemoryStore::with_limit(config.limits.max_persist_bytes);
        let runtime = Runtime::new(config, Box::new(store.clone()))?;
        let mut fallback = FakeLogic::default();
        fallback.store = store.clone();
        Ok(Self {
            runtime,
            store,
            fallback,
            calls: vec![],
            logs: vec![],
            durations: vec![],
            registered: BTreeSet::new(),
            instances: BTreeSet::new(),
        })
    }
    pub fn register(
        &mut self,
        plugin: &str,
        revision: &str,
        path: impl AsRef<Path>,
    ) -> Result<(), String> {
        let bytes = std::fs::read(path.as_ref())
            .map_err(|e| format!("{}: {e}", path.as_ref().display()))?;
        self.register_bytes(plugin, revision, &bytes)
    }
    pub fn register_bytes(
        &mut self,
        plugin: &str,
        revision: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        let plugin: PluginId = plugin.parse().map_err(|e: ModelError| e.to_string())?;
        // A failed registered module stays a wasm failure, never an inferred FakeLogic success.
        self.registered.insert(plugin.clone());
        self.runtime
            .load_module(plugin, revision, bytes)
            .map_err(|e| e.to_string())
    }
    pub fn persisted(&mut self, key: &PersistKey) -> StatePatch {
        self.store.load(key).expect("memory persistence")
    }
    pub(crate) fn probe(
        &mut self,
        plugin: &PluginId,
        revision: &str,
    ) -> Result<LogicDescription, String> {
        let instance = InstanceId {
            number: u64::MAX,
            generation: Generation(0),
        };
        let set: SetRef = format!("{plugin}/probe").parse().expect("probe set");
        let key = PersistKey {
            plugin: plugin.clone(),
            surface: "headless:probe".parse().unwrap(),
            scope: touchbar_core::PersistScope::Layer(set.clone()),
        };
        let response = self
            .runtime
            .create(Timestamp(0), instance, plugin, revision, &set, &key);
        self.logs.extend(self.runtime.take_logs());
        let description = match response {
            Input::LogicDescribed { description, .. } => Ok(description),
            Input::InstanceFault { reason, .. } => Err(reason),
            _ => unreachable!("runtime create response"),
        };
        self.runtime.dispose(instance);
        self.logs.extend(self.runtime.take_logs());
        description
    }
}
impl LogicHost for WasmLogic {
    fn create(
        &mut self,
        now: Timestamp,
        instance: InstanceId,
        set: &SetSpec,
        provider: Option<&PluginSpec>,
        key: &PersistKey,
    ) -> Input {
        if let Some(provider) = provider
            && self.registered.contains(&provider.id)
        {
            self.instances.insert(instance);
            let input = self.runtime.create(
                now,
                instance,
                &provider.id,
                &provider.revision,
                &set.reference,
                key,
            );
            self.logs.extend(self.runtime.take_logs());
            input
        } else {
            self.instances.remove(&instance);
            self.fallback.create(now, instance, set, provider, key)
        }
    }
    fn invoke(
        &mut self,
        now: Timestamp,
        set: &SetRef,
        instance: InstanceId,
        call: CallId,
        invocation: &LogicCall,
    ) -> Option<(u64, Input)> {
        if self.instances.contains(&instance) {
            self.calls.push((instance, call, invocation.clone()));
            let response = self.runtime.call(now, instance, call, invocation);
            self.logs.extend(response.logs);
            self.durations.push((instance, call, response.duration));
            response.input.map(|input| (0, input))
        } else {
            self.fallback.invoke(now, set, instance, call, invocation)
        }
    }
    fn dispose(&mut self, instance: InstanceId) {
        // Keep only the routing tombstone: Runtime frees the Store immediately.
        // A late call must reach Runtime's no-op instead of FakeLogic.
        if self.instances.contains(&instance) {
            self.runtime.dispose(instance);
            self.logs.extend(self.runtime.take_logs());
        } else {
            self.fallback.dispose(instance);
        }
    }
    fn persist(&mut self, key: &PersistKey, fields: &StatePatch) -> Result<(), String> {
        let result = self.runtime.persist(key, fields).map_err(|e| e.to_string());
        self.logs.extend(self.runtime.take_logs());
        result
    }
}
