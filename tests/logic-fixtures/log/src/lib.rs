use touchbar_logic_sdk::{CallContext, Logic, log};
use touchbar_model::*;
pub struct Logging(usize);
fn emit(length: usize) {
    let text = "ж".repeat(length);
    for i in 0..1000 {
        log::info!("{i}: {text}");
    }
}
impl Logic for Logging {
    fn describe() -> LogicDescription {
        LogicDescription::default()
    }
    fn init(config: &str, _: StatePatch, _: &CallContext) -> Result<(Self, LogicUpdate), String> {
        let length = config.parse().unwrap_or(1024);
        emit(length);
        Ok((Self(length), LogicUpdate::default()))
    }
    fn handle(&mut self, _: LogicEvent, _: &CallContext) -> Result<LogicUpdate, String> {
        emit(self.0);
        Ok(LogicUpdate::default())
    }
}
touchbar_logic_sdk::export!(Logging);
