use touchbar_logic_sdk::{CallContext, Logic};
use touchbar_model::*;
pub struct ErrLogic(String);
impl Logic for ErrLogic {
    fn describe() -> LogicDescription {
        LogicDescription::default()
    }
    fn init(config: &str, _: StatePatch, _: &CallContext) -> Result<(Self, LogicUpdate), String> {
        if config == "init" {
            Err("fixture init error".into())
        } else {
            Ok((Self(config.into()), LogicUpdate::default()))
        }
    }
    fn handle(&mut self, _: LogicEvent, _: &CallContext) -> Result<LogicUpdate, String> {
        if self.0 == "handle" || self.0.is_empty() {
            Err("fixture handle error".into())
        } else {
            Ok(LogicUpdate::default())
        }
    }
}
touchbar_logic_sdk::export!(ErrLogic);
