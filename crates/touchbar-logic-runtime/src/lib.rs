//! Synchronous, isolated execution of versioned logic components.
#[allow(unsafe_code)]
pub mod bindings {
    wasmtime::component::bindgen!({path:"../../wit",world:"logic"});
}
pub mod translation {
    touchbar_logic_types::define_wit_translations!(
        crate::bindings::exports::wstouchbar::logic::api
    );
}
mod limits;
use limits::{MemoryLimiter, fault_reason};
mod persist;
pub use persist::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration, Instant},
};
use touchbar_core::{Input, PersistKey};
use touchbar_logic_types::LogicLimits;
use touchbar_model::*;
use wasmtime::component::{HasSelf, Linker};
use wasmtime::{Cache, CacheConfig, Config, Engine, Store};

#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    pub limits: LogicLimits,
    pub cache_dir: Option<PathBuf>,
}
impl Default for RuntimeConfig {
    fn default() -> Self {
        let cache_root = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .map(|p| p.join(".cache"))
            });
        Self {
            limits: LogicLimits::default(),
            cache_dir: cache_root.map(|p| p.join("touchbar/wasm")),
        }
    }
}
#[derive(Debug, thiserror::Error)]
#[error("logic runtime: {0}")]
pub struct RuntimeError(pub String);
#[derive(Clone, Debug, thiserror::Error)]
#[error("logic module: {0}")]
pub struct LoadError(pub String);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogRecord {
    pub level: LogLevel,
    pub message: String,
}
impl LogRecord {
    fn warning(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Warn,
            message: message.into(),
        }
    }
}
pub struct CallResponse {
    pub input: Option<Input>,
    pub logs: Vec<LogRecord>,
    pub duration: Duration,
}
struct HostState {
    limiter: MemoryLimiter,
    limits: LogicLimits,
    dropped: u64,
    logs: Vec<LogRecord>,
}
impl bindings::LogicImports for HostState {
    fn log(&mut self, level: bindings::LogLevel, message: String) {
        if self.logs.len() >= self.limits.max_log_records {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        let mut message = message;
        let mut length = message.len().min(self.limits.max_log_message_bytes);
        while !message.is_char_boundary(length) {
            length -= 1;
        }
        message.truncate(length);
        self.logs.push(LogRecord {
            level: match level {
                bindings::LogLevel::Trace => LogLevel::Trace,
                bindings::LogLevel::Debug => LogLevel::Debug,
                bindings::LogLevel::Info => LogLevel::Info,
                bindings::LogLevel::Warn => LogLevel::Warn,
                bindings::LogLevel::Error => LogLevel::Error,
            },
            message,
        });
    }
}
impl HostState {
    fn reset_call(&mut self) {
        self.logs.clear();
        self.dropped = 0;
        self.limiter.denied = None;
    }
    fn take_logs(&mut self) -> Vec<LogRecord> {
        let mut logs = std::mem::take(&mut self.logs);
        if self.dropped > 0 {
            logs.push(LogRecord::warning(format!(
                "{} guest log records dropped",
                self.dropped
            )));
            self.dropped = 0;
        }
        logs
    }
}
struct Module {
    pre: bindings::LogicPre<HostState>,
    description: Option<LogicDescription>,
}
struct Instance {
    store: Store<HostState>,
    api: bindings::Logic,
}
mod epoch;
use epoch::EpochTicker;
mod compiler;
pub use compiler::{Compiler, PreparedModule};
pub struct Runtime {
    engine: Engine,
    linker: Linker<HostState>,
    config: RuntimeConfig,
    modules: BTreeMap<(PluginId, String), Result<Module, LoadError>>,
    instances: BTreeMap<InstanceId, Instance>,
    pending_fault_disposes: BTreeSet<InstanceId>,
    persist: Box<dyn PersistStore>,
    logs: Vec<LogRecord>,
    ticker: EpochTicker,
}
impl Runtime {
    pub fn new(
        config: RuntimeConfig,
        persist: Box<dyn PersistStore>,
    ) -> Result<Self, RuntimeError> {
        let mut c = Config::new();
        c.wasm_component_model(true)
            .epoch_interruption(true)
            .cranelift_nan_canonicalization(true)
            .relaxed_simd_deterministic(true)
            .max_wasm_stack(config.limits.max_wasm_stack_bytes)
            .memory_reservation(32 * 1024 * 1024);
        if let Some(directory) = &config.cache_dir {
            let mut cache_config = CacheConfig::new();
            cache_config.with_directory(directory);
            c.cache(Some(
                Cache::new(cache_config).map_err(|e| RuntimeError(format!("cache: {e:#}")))?,
            ));
        }
        // The workspace deliberately omits Wasmtime's threads feature.
        let engine = Engine::new(&c).map_err(|e| RuntimeError(e.to_string()))?;
        let mut linker = Linker::new(&engine);
        bindings::Logic::add_to_linker::<_, HasSelf<_>>(&mut linker, |data| data)
            .map_err(|e| RuntimeError(e.to_string()))?;
        let ticker = EpochTicker::new(engine.clone())?;
        Ok(Self {
            engine,
            linker,
            config,
            modules: BTreeMap::new(),
            instances: BTreeMap::new(),
            pending_fault_disposes: BTreeSet::new(),
            persist,
            logs: vec![],
            ticker,
        })
    }
    pub fn compiler(&self) -> Compiler {
        Compiler {
            engine: self.engine.clone(),
            linker: self.linker.clone(),
        }
    }
    /// Install a worker's result without compiling or running the guest.
    /// Successful and failed revision records are both immutable until eviction.
    pub fn install(
        &mut self,
        plugin: PluginId,
        revision: &str,
        prepared: Result<PreparedModule, LoadError>,
    ) -> Result<(), LoadError> {
        let key = (plugin, revision.to_owned());
        if let Some(module) = self.modules.get(&key) {
            return module.as_ref().map(|_| ()).map_err(Clone::clone);
        }
        let module = prepared.and_then(|prepared| {
            if !Engine::same(&self.engine, &prepared.engine) {
                return Err(LoadError(
                    "prepared module belongs to another engine".into(),
                ));
            }
            Ok(Module {
                pre: prepared.pre,
                description: None,
            })
        });
        let result = module.as_ref().map(|_| ()).map_err(Clone::clone);
        self.modules.insert(key, module);
        result
    }
    /// Synchronous convenience. Session loaders use compiler().prepare in a worker.
    pub fn load_module(
        &mut self,
        plugin: PluginId,
        revision: &str,
        bytes: &[u8],
    ) -> Result<(), LoadError> {
        if let Some(module) = self.modules.get(&(plugin.clone(), revision.to_owned())) {
            return module.as_ref().map(|_| ()).map_err(Clone::clone);
        }
        let prepared = self.compiler().prepare(bytes);
        self.install(plugin, revision, prepared)
    }
    /// Evict revisions absent from the accepted catalog, including failed loads.
    /// Existing Store instances keep their compiled code independently.
    pub fn retain_modules(&mut self, mut keep: impl FnMut(&PluginId, &str) -> bool) {
        self.modules
            .retain(|(plugin, revision), _| keep(plugin, revision));
    }
    fn instantiate(&self, pre: &bindings::LogicPre<HostState>) -> Result<Instance, String> {
        let limiter = MemoryLimiter::new(&self.config.limits);
        let mut store = Store::new(
            &self.engine,
            HostState {
                limiter,
                limits: self.config.limits.clone(),
                dropped: 0,
                logs: vec![],
            },
        );
        store.limiter(|host| &mut host.limiter);
        store.set_epoch_deadline(self.config.limits.logic_hard_ms.max(1));
        let api = pre
            .instantiate(&mut store)
            .map_err(|e| fault_reason(&e, store.data().limiter.denied))?;
        Ok(Instance { store, api })
    }
    pub fn create(
        &mut self,
        _now: Timestamp,
        instance: InstanceId,
        plugin: &PluginId,
        revision: &str,
        _set: &SetRef,
        key: &PersistKey,
    ) -> Input {
        if self.instances.contains_key(&instance) {
            return Input::InstanceFault {
                instance,
                reason: "instance already exists".into(),
            };
        }
        let _active = self.ticker.enter();
        let outcome = (|| {
            let module = self
                .modules
                .get(&(plugin.clone(), revision.to_owned()))
                .ok_or_else(|| format!("module {plugin}@{revision} is not loaded"))?
                .as_ref()
                .map_err(ToString::to_string)?;
            let pre = module.pre.clone();
            let cached = module.description.clone();
            let description = if let Some(description) = cached {
                description
            } else {
                // Describe uses a separate instance: all live instances start with identical guest state.
                let mut probe = self.instantiate(&pre)?;
                probe
                    .store
                    .set_epoch_deadline(self.config.limits.logic_hard_ms.max(1));
                let start = Instant::now();
                let result = probe
                    .api
                    .wstouchbar_logic_api()
                    .call_describe(&mut probe.store)
                    .map(translation::description_from);
                self.logs.extend(probe.store.data_mut().take_logs());
                if start.elapsed() > Duration::from_millis(self.config.limits.logic_soft_ms) {
                    self.logs
                        .push(LogRecord::warning("describe exceeded soft deadline"));
                }
                let description = result.map_err(|e| {
                    format!(
                        "describe: {}",
                        fault_reason(&e, probe.store.data().limiter.denied)
                    )
                })?;
                if let Some(Ok(module)) =
                    self.modules.get_mut(&(plugin.clone(), revision.to_owned()))
                {
                    module.description = Some(description.clone());
                }
                description
            };
            let persisted = self.persist.load(key);
            self.logs.extend(self.persist.take_logs());
            let persisted = persisted.map_err(|e| e.to_string())?;
            let live = self.instantiate(&pre)?;
            self.instances.insert(instance, live);
            self.pending_fault_disposes.remove(&instance);
            Ok::<_, String>((description, persisted))
        })();
        match outcome {
            Ok((description, persisted)) => Input::LogicDescribed {
                instance,
                description,
                persisted,
            },
            Err(reason) => {
                // Core still acknowledges a failed creation with DisposeInstance.
                self.pending_fault_disposes.insert(instance);
                Input::InstanceFault { instance, reason }
            }
        }
    }
    pub fn call(
        &mut self,
        now: Timestamp,
        instance: InstanceId,
        call: CallId,
        invocation: &LogicCall,
    ) -> CallResponse {
        let start = Instant::now();
        if self.instances.contains_key(&instance)
            && let Err(error) =
                touchbar_logic_types::check_call_budget(invocation, &self.config.limits)
        {
            let message = format!("host input budget: {error}");
            return CallResponse {
                input: Some(Input::LogicError {
                    instance,
                    call,
                    message: message.clone(),
                }),
                logs: vec![LogRecord {
                    level: LogLevel::Error,
                    message,
                }],
                duration: start.elapsed(),
            };
        }
        let Some(live) = self.instances.get_mut(&instance) else {
            return CallResponse {
                input: None,
                logs: vec![LogRecord::warning(format!(
                    "ignored call to unknown instance {instance:?}"
                ))],
                duration: start.elapsed(),
            };
        };
        let _active = self.ticker.enter();
        live.store.data_mut().reset_call();
        live.store
            .set_epoch_deadline(self.config.limits.logic_hard_ms.max(1));
        let ctx = bindings::exports::wstouchbar::logic::api::CallContext { now_ms: now.0 };
        let result = match invocation {
            LogicCall::Init { config, persisted } => live.api.wstouchbar_logic_api().call_init(
                &mut live.store,
                config,
                &translation::state_to(persisted.clone()),
                ctx,
            ),
            LogicCall::Handle(event) => live.api.wstouchbar_logic_api().call_handle(
                &mut live.store,
                &translation::event_to(event.clone()),
                ctx,
            ),
        };
        let mut logs = live.store.data_mut().take_logs();
        let allocation_denied = live.store.data().limiter.denied;
        let input = match result {
            Ok(Ok(wire)) => {
                let update = translation::update_from(wire);
                logs.extend(update.warnings.into_iter().map(LogRecord::warning));
                match touchbar_logic_types::check_budget(&update.value, &self.config.limits) {
                    Ok(()) => Input::LogicUpdate {
                        instance,
                        call,
                        update: update.value,
                    },
                    Err(error) => {
                        self.instances.remove(&instance);
                        self.pending_fault_disposes.insert(instance);
                        Input::LogicFault {
                            instance,
                            call,
                            reason: format!("budget: {error}"),
                        }
                    }
                }
            }
            Ok(Err(message)) => Input::LogicError {
                instance,
                call,
                message,
            },
            Err(error) => {
                self.instances.remove(&instance);
                self.pending_fault_disposes.insert(instance);
                Input::LogicFault {
                    instance,
                    call,
                    reason: fault_reason(&error, allocation_denied),
                }
            }
        };
        let duration = start.elapsed();
        if duration > Duration::from_millis(self.config.limits.logic_soft_ms) {
            logs.push(LogRecord::warning(format!(
                "logic exceeded soft deadline: {duration:?}"
            )));
        }
        CallResponse {
            input: Some(input),
            logs,
            duration,
        }
    }
    pub fn dispose(&mut self, instance: InstanceId) {
        if self.instances.remove(&instance).is_none()
            && !self.pending_fault_disposes.remove(&instance)
        {
            self.logs.push(LogRecord::warning(format!(
                "ignored dispose of unknown instance {instance:?}"
            )));
        }
    }
    pub fn persist(&mut self, key: &PersistKey, fields: &StatePatch) -> Result<(), StoreError> {
        let result = (|| {
            let current = self.persist.load(key)?;
            let merged = persist::merged(current, fields);
            persist::encoded(&merged, self.config.limits.max_persist_bytes)?;
            self.persist.write(key, fields)
        })();
        self.logs.extend(self.persist.take_logs());
        result
    }
    pub fn take_logs(&mut self) -> Vec<LogRecord> {
        std::mem::take(&mut self.logs)
    }
}
