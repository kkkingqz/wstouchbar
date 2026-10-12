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
mod persist;
pub use persist::*;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use touchbar_core::{Input, PersistKey};
use touchbar_logic_types::LogicLimits;
use touchbar_model::*;
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Cache, CacheConfig, Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

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
    limits: StoreLimits,
    logs: Vec<LogRecord>,
}
impl bindings::LogicImports for HostState {
    fn log(&mut self, level: bindings::LogLevel, message: String) {
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
struct Module {
    pre: bindings::LogicPre<HostState>,
    description: Option<LogicDescription>,
}
struct Instance {
    store: Store<HostState>,
    api: bindings::Logic,
}
struct EpochTicker {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl EpochTicker {
    fn new(engine: Engine) -> Result<Self, RuntimeError> {
        let stop = Arc::new(AtomicBool::new(false));
        let running = stop.clone();
        let thread = thread::Builder::new()
            .name("touchbar-wasm-epoch".into())
            .spawn(move || {
                while !running.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(1));
                    engine.increment_epoch();
                }
            })
            .map_err(|e| RuntimeError(e.to_string()))?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
pub struct Runtime {
    engine: Engine,
    linker: Linker<HostState>,
    config: RuntimeConfig,
    modules: BTreeMap<(PluginId, String), Result<Module, LoadError>>,
    instances: BTreeMap<InstanceId, Instance>,
    persist: Box<dyn PersistStore>,
    logs: Vec<LogRecord>,
    _ticker: EpochTicker,
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
            persist,
            logs: vec![],
            _ticker: ticker,
        })
    }
    pub fn load_module(
        &mut self,
        plugin: PluginId,
        revision: &str,
        bytes: &[u8],
    ) -> Result<(), LoadError> {
        let key = (plugin, revision.to_owned());
        if let Some(module) = self.modules.get(&key) {
            return module.as_ref().map(|_| ()).map_err(Clone::clone);
        }
        let module = (|| {
            let component =
                Component::new(&self.engine, bytes).map_err(|e| LoadError(format!("{e:#}")))?;
            let ty = component.component_type();
            let exports: Vec<_> = ty
                .exports(&self.engine)
                .map(|(name, _)| name.to_owned())
                .collect();
            if !exports
                .iter()
                .any(|name| name == "wstouchbar:logic/api@1.0.0")
            {
                return Err(LoadError(format!(
                    "expected export wstouchbar:logic/api@1.0.0; found {exports:?}"
                )));
            }
            for (name, import) in ty.imports(&self.engine) {
                let allowed = matches!(
                    (name, import.ty),
                    (
                        "log",
                        wasmtime::component::types::ComponentItem::ComponentFunc(_)
                    ) | (
                        "log-level",
                        wasmtime::component::types::ComponentItem::Type(_)
                    )
                );
                if !allowed {
                    return Err(LoadError(format!("forbidden import {name}")));
                }
            }
            let pre = self
                .linker
                .instantiate_pre(&component)
                .and_then(bindings::LogicPre::new)
                .map_err(|e| LoadError(format!("v1 interface types: {e:#}")))?;
            Ok(Module {
                pre,
                description: None,
            })
        })();
        let result = module.as_ref().map(|_| ()).map_err(Clone::clone);
        self.modules.insert(key, module);
        result
    }
    fn instantiate(&self, pre: &bindings::LogicPre<HostState>) -> Result<Instance, String> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.config.limits.max_memory_bytes)
            .build();
        let mut store = Store::new(
            &self.engine,
            HostState {
                limits,
                logs: vec![],
            },
        );
        store.limiter(|host| &mut host.limits);
        store.set_epoch_deadline(self.config.limits.logic_hard_ms.max(1));
        let api = pre.instantiate(&mut store).map_err(|e| format!("{e:#}"))?;
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
                let description = probe
                    .api
                    .wstouchbar_logic_api()
                    .call_describe(&mut probe.store)
                    .map(translation::description_from)
                    .map_err(|e| format!("describe: {e:#}"))?;
                self.logs.append(&mut probe.store.data_mut().logs);
                if let Some(Ok(module)) =
                    self.modules.get_mut(&(plugin.clone(), revision.to_owned()))
                {
                    module.description = Some(description.clone());
                }
                description
            };
            let persisted = self.persist.read(key).map_err(|e| e.to_string())?;
            let live = self.instantiate(&pre)?;
            self.instances.insert(instance, live);
            Ok::<_, String>((description, persisted))
        })();
        match outcome {
            Ok((description, persisted)) => Input::LogicDescribed {
                instance,
                description,
                persisted,
            },
            Err(reason) => Input::InstanceFault { instance, reason },
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
        let Some(live) = self.instances.get_mut(&instance) else {
            return CallResponse {
                input: None,
                logs: vec![LogRecord::warning(format!(
                    "ignored call to unknown instance {instance:?}"
                ))],
                duration: start.elapsed(),
            };
        };
        live.store.data_mut().logs.clear();
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
        let mut logs = std::mem::take(&mut live.store.data_mut().logs);
        let input = match result {
            Ok(Ok(wire)) => {
                let update = translation::update_from(wire);
                logs.extend(update.warnings.into_iter().map(LogRecord::warning));
                Input::LogicUpdate {
                    instance,
                    call,
                    update: update.value,
                }
            }
            Ok(Err(message)) => Input::LogicError {
                instance,
                call,
                message,
            },
            Err(error) => {
                self.instances.remove(&instance);
                Input::LogicFault {
                    instance,
                    call,
                    reason: format!("{error:#}"),
                }
            }
        };
        CallResponse {
            input: Some(input),
            logs,
            duration: start.elapsed(),
        }
    }
    pub fn dispose(&mut self, instance: InstanceId) {
        if self.instances.remove(&instance).is_none() {
            self.logs.push(LogRecord::warning(format!(
                "ignored dispose of unknown instance {instance:?}"
            )));
        }
    }
    pub fn persist(&mut self, key: &PersistKey, fields: &StatePatch) -> Result<(), StoreError> {
        self.persist.write(key, fields)
    }
    pub fn take_logs(&mut self) -> Vec<LogRecord> {
        std::mem::take(&mut self.logs)
    }
}
