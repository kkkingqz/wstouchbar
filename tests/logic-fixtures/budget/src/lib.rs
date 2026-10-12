use touchbar_logic_sdk::{CallContext, Logic};
use touchbar_model::*;
pub struct Budget;
fn oversized() -> LogicUpdate {
    LogicUpdate {
        state: vec![(
            "large".into(),
            StateValue::Scalar(Scalar::Text("x".repeat(300_000))),
        )],
        ..Default::default()
    }
}
impl Logic for Budget {
    fn describe() -> LogicDescription {
        LogicDescription::default()
    }
    fn init(_: &str, _: StatePatch, _: &CallContext) -> Result<(Self, LogicUpdate), String> {
        Ok((Self, oversized()))
    }
    fn handle(&mut self, _: LogicEvent, _: &CallContext) -> Result<LogicUpdate, String> {
        Ok(oversized())
    }
}
touchbar_logic_sdk::export!(Budget);
