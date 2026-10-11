use std::collections::{BTreeMap, BTreeSet};
use touchbar_model::*;
type Handler = Box<dyn FnMut(&Effect) -> (u64, EffectOutcome)>;
pub struct FakeEffects {
    pub delay_ms: u64,
    pub result: EffectOutcome,
    pub started: Vec<(RequestId, Effect)>,
    pub cancelled: Vec<RequestId>,
    pub(crate) active: BTreeSet<RequestId>,
    pub(crate) subscriptions: BTreeMap<SubscriptionId, SubscriptionKind>,
    handler: Option<Handler>,
}
impl Default for FakeEffects {
    fn default() -> Self {
        Self {
            delay_ms: 0,
            result: Ok(EffectResult::None),
            started: vec![],
            cancelled: vec![],
            active: BTreeSet::new(),
            subscriptions: BTreeMap::new(),
            handler: None,
        }
    }
}
impl FakeEffects {
    pub fn with_handler(handler: impl FnMut(&Effect) -> (u64, EffectOutcome) + 'static) -> Self {
        Self {
            handler: Some(Box::new(handler)),
            ..Self::default()
        }
    }
    pub(crate) fn start(&mut self, request: RequestId, effect: Effect) -> (u64, EffectOutcome) {
        self.active.insert(request);
        self.started.push((request, effect.clone()));
        if let Some(handler) = &mut self.handler {
            handler(&effect)
        } else {
            (
                if let Effect::TimerAfter { ms } = effect {
                    ms
                } else {
                    self.delay_ms
                },
                self.result.clone(),
            )
        }
    }
    pub(crate) fn cancel(&mut self, request: RequestId) {
        self.active.remove(&request);
        self.cancelled.push(request);
    }
}
