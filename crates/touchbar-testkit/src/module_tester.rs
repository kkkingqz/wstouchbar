use crate::{LogicHost, WasmLogic};
use std::path::Path;
use touchbar_core::{Input, PersistKey, PersistScope};
use touchbar_model::*;

/// Direct guest calls; no Core, effects executor or rendering is involved.
pub struct ModuleTester {
    pub update: LogicUpdate,
    pub state: StatePatch,
    pub description: LogicDescription,
    pub input: Option<Input>,
    pub logic: WasmLogic,
    now: Timestamp,
    instance: Option<InstanceId>,
    generation: u64,
    next_call: u64,
    set: SetSpec,
    provider: PluginSpec,
}
pub struct ExpectedState(StateValue);
impl From<Scalar> for ExpectedState {
    fn from(v: Scalar) -> Self {
        Self(StateValue::Scalar(v))
    }
}
impl From<StateValue> for ExpectedState {
    fn from(v: StateValue) -> Self {
        Self(v)
    }
}
impl From<&str> for ExpectedState {
    fn from(v: &str) -> Self {
        Scalar::Text(v.into()).into()
    }
}
impl From<String> for ExpectedState {
    fn from(v: String) -> Self {
        Scalar::Text(v).into()
    }
}
impl From<f64> for ExpectedState {
    fn from(v: f64) -> Self {
        Scalar::Number(v).into()
    }
}
impl From<bool> for ExpectedState {
    fn from(v: bool) -> Self {
        Scalar::Boolean(v).into()
    }
}
impl ModuleTester {
    pub fn load(path: impl AsRef<Path>) -> Self {
        Self::try_load(path).expect("load test component")
    }
    pub fn try_load(path: impl AsRef<Path>) -> Result<Self, String> {
        let mut logic = WasmLogic::new();
        logic.register("module", "test-1", path)?;
        let set = SetSpec::default_for("module/main".parse().unwrap());
        let provider = PluginSpec {
            id: "module".parse().unwrap(),
            revision: "test-1".into(),
            enabled: true,
            has_logic: true,
            requires: vec![],
        };
        let description = logic.probe(&provider.id, &provider.revision)?;
        Ok(Self {
            logic,
            set,
            provider,
            description,
            update: LogicUpdate::default(),
            state: vec![],
            input: None,
            now: Timestamp(0),
            instance: None,
            generation: 0,
            next_call: 0,
        })
    }
    pub fn init(&mut self, config: &str, persisted: StatePatch) -> &mut Self {
        if let Some(i) = self.instance.take() {
            self.logic.dispose(i);
        }
        self.generation += 1;
        let instance = InstanceId {
            number: 1,
            generation: Generation(self.generation),
        };
        self.instance = Some(instance);
        self.state.clear();
        self.update = LogicUpdate::default();
        let key = PersistKey {
            plugin: self.provider.id.clone(),
            surface: "headless:module".parse().unwrap(),
            scope: PersistScope::ProfileSlot {
                slot: format!("init-{}", self.generation).parse().unwrap(),
                set: self.set.reference.clone(),
            },
        };
        if let Err(reason) = self.logic.persist(&key, &persisted) {
            self.input = Some(Input::InstanceFault { instance, reason });
            return self;
        }
        let response = self
            .logic
            .create(self.now, instance, &self.set, Some(&self.provider), &key);
        if let Input::LogicDescribed {
            description,
            persisted,
            ..
        } = response
        {
            self.description = description;
            self.invoke(LogicCall::Init {
                config: config.into(),
                persisted,
            });
        } else {
            self.input = Some(response);
        }
        self
    }
    pub fn handle(&mut self, event: LogicEvent) -> &mut Self {
        assert!(self.instance.is_some(), "init before handle");
        self.invoke(LogicCall::Handle(event));
        self
    }
    pub fn advance_ms(&mut self, ms: u64) -> &mut Self {
        self.now = self.now.after(ms);
        self
    }
    fn invoke(&mut self, invocation: LogicCall) {
        self.next_call += 1;
        self.update = LogicUpdate::default();
        self.input = self
            .logic
            .invoke(
                self.now,
                &self.set.reference,
                self.instance.expect("initialized instance"),
                CallId(self.next_call),
                &invocation,
            )
            .map(|(_, input)| input);
        if let Some(Input::LogicUpdate { update, .. }) = &self.input {
            self.update = update.clone();
            for (name, value) in &update.state {
                if let Some((_, previous)) = self.state.iter_mut().find(|(n, _)| n == name) {
                    *previous = value.clone();
                } else {
                    self.state.push((name.clone(), value.clone()));
                }
            }
        }
    }
    pub fn expect_state(&mut self, name: &str, value: impl Into<ExpectedState>) -> &mut Self {
        assert!(
            self.state.contains(&(name.into(), value.into().0)),
            "expected state {name}, got {:?}; reply {:?}",
            self.state,
            self.input
        );
        self
    }
    pub fn expect_request(&mut self, check: impl Fn(&EffectRequest) -> bool) -> &mut Self {
        assert!(
            self.update.requests.iter().any(check),
            "requests {:?}",
            self.update.requests
        );
        self
    }
    pub fn expect_subscriptions(
        &mut self,
        check: impl Fn(&[SubscriptionSpec]) -> bool,
    ) -> &mut Self {
        assert!(
            check(self.update.subscriptions.as_deref().unwrap_or_default()),
            "subscriptions {:?}",
            self.update.subscriptions
        );
        self
    }
}
pub fn action(name: &str, kind: ActionKind, value: f64) -> LogicEvent {
    LogicEvent::Action(ActionEvent {
        action: name.into(),
        kind,
        value: Some(Scalar::Number(value)),
        record_key: None,
        origin: Origin::Physical,
        cancel_reason: None,
    })
}
pub fn audio_data(tag: &str, volume: f64, muted: bool) -> LogicEvent {
    LogicEvent::SubscriptionData {
        tag: tag.into(),
        data: SubscriptionData::Audio(AudioState {
            available: true,
            volume,
            muted,
            name: "test".into(),
        }),
    }
}
