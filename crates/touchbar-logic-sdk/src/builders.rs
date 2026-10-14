use touchbar_logic_types::{LogicLimits, check_budget};
use touchbar_model::*;

#[derive(Default)]
pub struct Description(LogicDescription);
impl Description {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn action(
        mut self,
        name: impl Into<String>,
        kinds: Vec<ActionKind>,
        value: Option<ScalarKind>,
    ) -> Self {
        self.0
            .actions
            .push((name.into(), ActionDescription { kinds, value }));
        self
    }
    pub fn field(mut self, name: impl Into<String>, kind: FieldKind, persist: bool) -> Self {
        self.0.state.push(StateField {
            name: name.into(),
            kind,
            persist,
        });
        self
    }
    pub fn build(self) -> LogicDescription {
        self.0
    }
}

#[derive(Default)]
pub struct Update(LogicUpdate);
impl Update {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(mut self, name: impl Into<String>, value: impl IntoValue) -> Self {
        self.0.state.push((name.into(), value.into_value()));
        self
    }
    pub fn request(mut self, request: impl Into<EffectRequest>) -> Self {
        self.0.requests.push(request.into());
        self
    }
    pub fn cancel(mut self, tag: impl Into<String>) -> Self {
        self.0.cancel.push(tag.into());
        self
    }
    pub fn subscriptions(mut self, subscriptions: Vec<SubscriptionSpec>) -> Self {
        self.0.subscriptions = Some(subscriptions);
        self
    }
    pub fn presentation(mut self, command: PresentationCommand) -> Self {
        self.0.presentation = Some(command);
        self
    }
    pub fn build(self) -> LogicUpdate {
        debug_assert!(
            check_budget(&self.0, &LogicLimits::default()).is_ok(),
            "logic update exceeds the default wire budget"
        );
        self.0
    }
}

pub struct Request(EffectRequest);
impl Request {
    pub fn new(tag: impl Into<String>, effect: Effect) -> Self {
        Self(EffectRequest {
            tag: tag.into(),
            effect,
            lifetime: None,
            coalesce: None,
            timeout_ms: None,
        })
    }
    pub fn coalesce(mut self, key: impl Into<String>) -> Self {
        self.0.coalesce = Some(key.into());
        self
    }
    pub fn committed(mut self) -> Self {
        self.0.lifetime = Some(Lifetime::Committed);
        self
    }
    pub fn timeout(mut self, ms: u64) -> Self {
        self.0.timeout_ms = Some(ms);
        self
    }
    pub fn build(self) -> EffectRequest {
        self.0
    }
}
impl From<Request> for EffectRequest {
    fn from(request: Request) -> Self {
        request.0
    }
}

/// Values accepted by Update::set without introducing SDK-specific state types.
pub trait IntoValue {
    fn into_value(self) -> StateValue;
}
impl IntoValue for StateValue {
    fn into_value(self) -> StateValue {
        self
    }
}
impl IntoValue for Scalar {
    fn into_value(self) -> StateValue {
        StateValue::Scalar(self)
    }
}
impl IntoValue for f64 {
    fn into_value(self) -> StateValue {
        Scalar::Number(self).into_value()
    }
}
impl IntoValue for bool {
    fn into_value(self) -> StateValue {
        Scalar::Boolean(self).into_value()
    }
}
impl IntoValue for &str {
    fn into_value(self) -> StateValue {
        Scalar::Text(self.into()).into_value()
    }
}
impl IntoValue for String {
    fn into_value(self) -> StateValue {
        Scalar::Text(self).into_value()
    }
}
