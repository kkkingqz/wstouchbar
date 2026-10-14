use crate::*;
use touchbar_core::*;
use touchbar_model::*;
#[derive(Clone, Debug)]
pub struct Target {
    pub surface: SurfaceId,
    pub widget: String,
}
pub fn at(widget: &str) -> Target {
    Target {
        surface: SurfaceId(1),
        widget: widget.into(),
    }
}
type EffectCheck = Box<dyn Fn(&Effect) -> bool>;
enum Step {
    Input(Input),
    After(u64),
    Tap(Target),
    Down(u32, Target),
    Up(u32),
    Expect(EffectCheck),
    Drag(Target, f64, f64, u64, u32),
    Subscription(SurfaceId, String, String, SubscriptionData),
}
pub struct Scenario<H: LogicHost = FakeLogic> {
    config: CoreConfig,
    logic: H,
    effects: FakeEffects,
    steps: Vec<Step>,
}
pub fn scenario() -> Scenario {
    Scenario {
        config: CoreConfig::default(),
        logic: FakeLogic::default(),
        effects: FakeEffects::default(),
        steps: vec![],
    }
}
impl<H: LogicHost> Scenario<H> {
    pub fn config(mut self, config: CoreConfig) -> Self {
        self.config = config;
        self
    }
    pub fn logic<N: LogicHost>(self, logic: N) -> Scenario<N> {
        Scenario {
            config: self.config,
            logic,
            effects: self.effects,
            steps: self.steps,
        }
    }
    pub fn effects(mut self, effects: FakeEffects) -> Self {
        self.effects = effects;
        self
    }
    pub fn catalog(mut self, catalog: Catalog) -> Self {
        self.steps.push(Step::Input(Input::Catalog(catalog)));
        self
    }
    pub fn input(mut self, input: Input) -> Self {
        self.steps.push(Step::Input(input));
        self
    }
    pub fn fact(self, name: &str, value: &str) -> Self {
        self.input(Input::Fact {
            name: name.into(),
            value: Some(FactValue::Text(value.into())),
        })
    }
    pub fn tap(mut self, widget: &str) -> Self {
        self.steps.push(Step::Tap(at(widget)));
        self
    }
    pub fn touch_down(mut self, contact: u32, target: Target) -> Self {
        self.steps.push(Step::Down(contact, target));
        self
    }
    pub fn touch_up(mut self, contact: u32) -> Self {
        self.steps.push(Step::Up(contact));
        self
    }
    pub fn after_ms(mut self, ms: u64) -> Self {
        self.steps.push(Step::After(ms));
        self
    }
    pub fn expect_effect(mut self, check: impl Fn(&Effect) -> bool + 'static) -> Self {
        self.steps.push(Step::Expect(Box::new(check)));
        self
    }
    pub fn drag(mut self, widget: &str, from: f64, to: f64, duration: u64, steps: u32) -> Self {
        self.steps
            .push(Step::Drag(at(widget), from, to, duration, steps));
        self
    }
    pub fn subscription_data(self, set: &str, tag: &str, data: SubscriptionData) -> Self {
        self.subscription_data_on(SurfaceId(1), set, tag, data)
    }
    pub fn subscription_data_on(
        mut self,
        surface: SurfaceId,
        set: &str,
        tag: &str,
        data: SubscriptionData,
    ) -> Self {
        self.steps
            .push(Step::Subscription(surface, set.into(), tag.into(), data));
        self
    }
    pub fn run(self) -> Harness<H> {
        let mut harness = Harness::with_logic(self.config, self.logic);
        harness.effects = self.effects;
        harness.send(Input::SurfaceAdded(headless(1)));
        for step in self.steps {
            match step {
                Step::Input(input) => {
                    harness.send(input);
                }
                Step::After(ms) => harness.advance_ms(ms),
                Step::Tap(target) => harness.tap(target.surface, &target.widget),
                Step::Down(contact, target) => {
                    let point = harness.hit(target.surface, &target.widget);
                    harness.touch(target.surface, contact, TouchPhase::Down, point);
                }
                Step::Up(contact) => harness.touch_up(SurfaceId(1), contact),
                Step::Drag(target, from, to, duration, steps) => {
                    harness.drag(target.surface, &target.widget, from, to, duration, steps)
                }
                Step::Subscription(surface, set, tag, data) => {
                    harness.subscription_data(surface, &set, &tag, data)
                }
                Step::Expect(check) => assert!(
                    harness
                        .effects
                        .started
                        .iter()
                        .any(|(_, effect)| check(effect)),
                    "expected effect missing; got {:?}",
                    harness.effects.started
                ),
            }
        }
        harness
    }
}
