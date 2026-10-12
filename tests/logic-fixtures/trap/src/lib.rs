use touchbar_logic_sdk::{CallContext, Description, Logic};
use touchbar_model::*;
pub struct Adversary(String);
#[inline(never)]
fn recurse(depth: u64) -> u64 {
    if depth == 0 {
        0
    } else {
        std::hint::black_box(recurse(depth.wrapping_add(1))).wrapping_add(depth)
    }
}
impl Logic for Adversary {
    fn describe() -> LogicDescription {
        Description::new()
            .action("trap", vec![ActionKind::Activated], None)
            .action("stack", vec![ActionKind::Activated], None)
            .build()
    }
    fn init(config: &str, _: StatePatch, _: &CallContext) -> Result<(Self, LogicUpdate), String> {
        Ok((
            Self(if config.is_empty() {
                "trap".into()
            } else {
                config.into()
            }),
            LogicUpdate::default(),
        ))
    }
    fn handle(&mut self, event: LogicEvent, _: &CallContext) -> Result<LogicUpdate, String> {
        if matches!(event, LogicEvent::Action(ref action) if action.action == self.0) {
            if self.0 == "stack" {
                std::hint::black_box(recurse(1));
            } else {
                panic!("fixture trap");
            }
        }
        Ok(LogicUpdate::default())
    }
}
touchbar_logic_sdk::export!(Adversary);
