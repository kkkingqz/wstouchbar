use std::collections::BTreeMap;
use touchbar_core::{Input, PersistKey};
use touchbar_logic_runtime::{MemoryStore, PersistStore};
use touchbar_model::*;
#[derive(Clone, Debug)]
pub enum LogicReaction {
    Update(LogicUpdate),
    Delayed { ms: u64, update: LogicUpdate },
    Error(String),
    Fault(String),
    Hang,
}
type Handler = Box<dyn FnMut(InstanceId, &LogicCall) -> LogicReaction>;
struct Registration {
    description: Option<LogicDescription>,
    handler: Handler,
}
#[derive(Default)]
pub struct FakeLogic {
    handlers: BTreeMap<SetRef, Registration>,
    pub calls: Vec<(InstanceId, CallId, LogicCall)>,
    pub(crate) store: MemoryStore,
}
impl FakeLogic {
    pub fn register(
        &mut self,
        set: &str,
        description: Option<LogicDescription>,
        handler: impl FnMut(InstanceId, &LogicCall) -> LogicReaction + 'static,
    ) {
        self.handlers.insert(
            set.parse().expect("logic set ref"),
            Registration {
                description,
                handler: Box::new(handler),
            },
        );
    }
    pub(crate) fn description(&self, set: &SetSpec) -> LogicDescription {
        self.handlers
            .get(&set.reference)
            .and_then(|h| h.description.clone())
            .unwrap_or_else(|| infer_description(set))
    }
    pub(crate) fn call(
        &mut self,
        set: &SetRef,
        instance: InstanceId,
        call: CallId,
        invocation: &LogicCall,
    ) -> LogicReaction {
        self.calls.push((instance, call, invocation.clone()));
        if let Some(handler) = self.handlers.get_mut(set) {
            (handler.handler)(instance, invocation)
        } else {
            LogicReaction::Update(LogicUpdate {
                state: match invocation {
                    LogicCall::Init { persisted, .. } => persisted.clone(),
                    _ => vec![],
                },
                ..LogicUpdate::default()
            })
        }
    }
}
fn infer_description(set: &SetSpec) -> LogicDescription {
    let mut description = LogicDescription::default();
    let mut widgets = vec![];
    for widget in &set.widgets {
        widget.visit(&mut widgets);
    }
    for panel in &set.panels {
        for widget in &panel.widgets {
            widget.visit(&mut widgets);
        }
    }
    for widget in widgets {
        for (kind, name) in &widget.handlers {
            if let Some((_, action)) = description.actions.iter_mut().find(|(n, _)| n == name) {
                if !action.kinds.contains(kind) {
                    action.kinds.push(*kind);
                }
            } else {
                description.actions.push((
                    name.clone(),
                    ActionDescription {
                        kinds: vec![*kind],
                        value: if *kind == ActionKind::ValueChanged {
                            Some(ScalarKind::Number)
                        } else {
                            None
                        },
                    },
                ));
            }
        }
        for (binding, name) in &widget.state_bindings {
            if name.starts_with("record.") || description.state.iter().any(|f| &f.name == name) {
                continue;
            }
            let kind = match binding {
                StateBinding::Text | StateBinding::Symbol => FieldKind::Scalar(ScalarKind::Text),
                StateBinding::Value => FieldKind::Scalar(ScalarKind::Number),
                StateBinding::Enabled | StateBinding::Visible | StateBinding::Selected => {
                    FieldKind::Scalar(ScalarKind::Boolean)
                }
                StateBinding::Canvas => FieldKind::Canvas,
                StateBinding::Records => FieldKind::Records(vec![
                    ("key".into(), ScalarKind::Text),
                    ("text".into(), ScalarKind::Text),
                ]),
            };
            description.state.push(StateField {
                name: name.clone(),
                kind,
                persist: set.persist.contains(name),
            });
        }
    }
    description
}

/// A synchronous host. Replies are scheduled by Harness at virtual now + delay.
pub trait LogicHost {
    fn create(
        &mut self,
        now: Timestamp,
        instance: InstanceId,
        set: &SetSpec,
        provider: Option<&PluginSpec>,
        key: &PersistKey,
    ) -> Input;
    fn invoke(
        &mut self,
        now: Timestamp,
        set: &SetRef,
        instance: InstanceId,
        call: CallId,
        invocation: &LogicCall,
    ) -> Option<(u64, Input)>;
    fn dispose(&mut self, instance: InstanceId);
    fn persist(&mut self, key: &PersistKey, fields: &StatePatch) -> Result<(), String>;
}
impl LogicHost for FakeLogic {
    fn create(
        &mut self,
        _now: Timestamp,
        instance: InstanceId,
        set: &SetSpec,
        _provider: Option<&PluginSpec>,
        key: &PersistKey,
    ) -> Input {
        match self.store.load(key) {
            Ok(persisted) => Input::LogicDescribed {
                instance,
                description: self.description(set),
                persisted,
            },
            Err(e) => Input::InstanceFault {
                instance,
                reason: e.to_string(),
            },
        }
    }
    fn invoke(
        &mut self,
        _now: Timestamp,
        set: &SetRef,
        instance: InstanceId,
        call: CallId,
        invocation: &LogicCall,
    ) -> Option<(u64, Input)> {
        let reply = match self.call(set, instance, call, invocation) {
            LogicReaction::Update(update) => (
                0,
                Input::LogicUpdate {
                    instance,
                    call,
                    update,
                },
            ),
            LogicReaction::Delayed { ms, update } => (
                ms,
                Input::LogicUpdate {
                    instance,
                    call,
                    update,
                },
            ),
            LogicReaction::Error(message) => (
                0,
                Input::LogicError {
                    instance,
                    call,
                    message,
                },
            ),
            LogicReaction::Fault(reason) => (
                0,
                Input::LogicFault {
                    instance,
                    call,
                    reason,
                },
            ),
            LogicReaction::Hang => return None,
        };
        Some(reply)
    }
    fn dispose(&mut self, _instance: InstanceId) {}
    fn persist(&mut self, key: &PersistKey, fields: &StatePatch) -> Result<(), String> {
        self.store.write(key, fields).map_err(|e| e.to_string())
    }
}
