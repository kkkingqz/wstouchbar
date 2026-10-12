//! The API skeleton is exported now; controls behavior and sets are ported in L11.
use touchbar_logic_sdk::{CallContext, Logic};
use touchbar_model::*;
pub struct Controls;
impl Logic for Controls {
    fn describe() -> LogicDescription {
        LogicDescription::default()
    }
    fn init(_: &str, _: StatePatch, _: &CallContext) -> Result<(Self, LogicUpdate), String> {
        Ok((Self, LogicUpdate::default()))
    }
    fn handle(&mut self, _: LogicEvent, _: &CallContext) -> Result<LogicUpdate, String> {
        Ok(LogicUpdate::default())
    }
}
touchbar_logic_sdk::export!(Controls);
