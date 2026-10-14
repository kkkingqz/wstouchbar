use touchbar_logic_sdk::{CallContext, GuestInstance, Logic, Update};
use touchbar_model::*;

struct Counter(u64);
impl Logic for Counter {
    fn describe() -> LogicDescription {
        LogicDescription::default()
    }
    fn init(config: &str, _: StatePatch, ctx: &CallContext) -> Result<(Self, LogicUpdate), String> {
        if config == "err" {
            return Err("bad config".into());
        }
        Ok((
            Self(ctx.now_ms),
            Update::new().set("count", ctx.now_ms as f64).build(),
        ))
    }
    fn handle(&mut self, _: LogicEvent, ctx: &CallContext) -> Result<LogicUpdate, String> {
        if ctx.now_ms == 0 {
            return Err("bad event".into());
        }
        self.0 += 1;
        Ok(Update::new().set("count", self.0 as f64).build())
    }
}
touchbar_logic_sdk::export!(Counter);

#[test]
fn guest_retains_one_logic_value_per_instance_and_passes_caller_time() {
    let mut a = GuestInstance::<Counter>::default();
    let mut b = GuestInstance::<Counter>::default();
    assert!(
        a.handle(LogicEvent::Shown, &CallContext { now_ms: 1 })
            .is_err()
    );
    a.init("", vec![], &CallContext { now_ms: 41 }).unwrap();
    b.init("", vec![], &CallContext { now_ms: 100 }).unwrap();
    let result = a
        .handle(LogicEvent::Shown, &CallContext { now_ms: 50 })
        .unwrap();
    assert_eq!(result.state[0].1, StateValue::Scalar(Scalar::Number(42.0)));
    let result = b
        .handle(LogicEvent::Shown, &CallContext { now_ms: 101 })
        .unwrap();
    assert_eq!(result.state[0].1, StateValue::Scalar(Scalar::Number(101.0)));
}

#[test]
fn guest_returns_logic_errors_and_failed_init_leaves_no_old_value() {
    let mut guest = GuestInstance::<Counter>::default();
    guest.init("", vec![], &CallContext { now_ms: 1 }).unwrap();
    assert_eq!(
        guest
            .handle(LogicEvent::Shown, &CallContext { now_ms: 0 })
            .unwrap_err(),
        "bad event"
    );
    assert_eq!(
        guest
            .init("err", vec![], &CallContext { now_ms: 1 })
            .unwrap_err(),
        "bad config"
    );
    assert!(
        guest
            .handle(LogicEvent::Shown, &CallContext { now_ms: 1 })
            .is_err()
    );
}

#[cfg(debug_assertions)]
#[test]
fn guest_checks_budget_even_when_logic_returns_an_update_without_builders() {
    struct Oversized;
    impl Logic for Oversized {
        fn describe() -> LogicDescription {
            LogicDescription::default()
        }
        fn init(_: &str, _: StatePatch, _: &CallContext) -> Result<(Self, LogicUpdate), String> {
            Ok((Self, LogicUpdate::default()))
        }
        fn handle(&mut self, _: LogicEvent, _: &CallContext) -> Result<LogicUpdate, String> {
            Ok(LogicUpdate {
                state: vec![(
                    "large".into(),
                    StateValue::Scalar(Scalar::Text("x".repeat(300_000))),
                )],
                ..Default::default()
            })
        }
    }
    let mut guest = GuestInstance::<Oversized>::default();
    guest.init("", vec![], &CallContext::default()).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        guest.handle(LogicEvent::Shown, &CallContext::default())
    }));
    assert!(result.is_err());
}
