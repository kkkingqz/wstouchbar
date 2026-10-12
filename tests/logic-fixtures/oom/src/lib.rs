use touchbar_logic_sdk::{CallContext, Description, Logic};
use touchbar_model::*;
pub struct Adversary(String);
impl Logic for Adversary {
    fn describe() -> LogicDescription {
        Description::new()
            .action("oom", vec![ActionKind::Activated], None)
            .build()
    }
    fn init(config: &str, _: StatePatch, _: &CallContext) -> Result<(Self, LogicUpdate), String> {
        Ok((
            Self(if config.is_empty() {
                "oom".into()
            } else {
                config.into()
            }),
            LogicUpdate::default(),
        ))
    }
    fn handle(&mut self, event: LogicEvent, _: &CallContext) -> Result<LogicUpdate, String> {
        if matches!(event, LogicEvent::Action(ref action) if action.action == self.0) {
            {
                let mut blocks = Vec::new();
                loop {
                    blocks.push(vec![0u8; 256 * 1024]);
                    std::hint::black_box(&mut blocks);
                }
            };
        }
        Ok(LogicUpdate::default())
    }
}
touchbar_logic_sdk::export!(Adversary);
