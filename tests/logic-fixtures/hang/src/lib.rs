use touchbar_logic_sdk::{CallContext, Description, Logic};
use touchbar_model::*;
pub struct Adversary(String);
impl Logic for Adversary {
    fn describe() -> LogicDescription {
        Description::new()
            .action("hang", vec![ActionKind::Activated], None)
            .build()
    }
    fn init(config: &str, _: StatePatch, _: &CallContext) -> Result<(Self, LogicUpdate), String> {
        Ok((
            Self(if config.is_empty() {
                "hang".into()
            } else {
                config.into()
            }),
            LogicUpdate::default(),
        ))
    }
    fn handle(&mut self, event: LogicEvent, _: &CallContext) -> Result<LogicUpdate, String> {
        if matches!(event, LogicEvent::Action(ref action) if action.action == self.0) {
            loop {
                std::hint::spin_loop();
            }
        }
        Ok(LogicUpdate::default())
    }
}
touchbar_logic_sdk::export!(Adversary);
