// Compile the same business logic without importing the component export glue.
#[path = "../../../../plugins/controls/src/logic.rs"]
mod controls;
use touchbar_logic_sdk::{CallContext, Description, Logic};
use touchbar_model::*;
pub struct Adversary {
    action: String,
    controls: Option<controls::Controls>,
}
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
        let mut description = controls::Controls::describe();
        description.actions.extend(
            Description::new()
                .action("trap", vec![ActionKind::Activated], None)
                .action("stack", vec![ActionKind::Activated], None)
                .build()
                .actions,
        );
        description
    }
    fn init(
        config: &str,
        persisted: StatePatch,
        context: &CallContext,
    ) -> Result<(Self, LogicUpdate), String> {
        if matches!(config, "volume" | "microphone" | "brightness") {
            let (controls, update) = controls::Controls::init(config, persisted, context)?;
            return Ok((
                Self {
                    action: "volume-level".into(),
                    controls: Some(controls),
                },
                update,
            ));
        }
        Ok((
            Self {
                action: if config.is_empty() {
                    "trap".into()
                } else {
                    config.into()
                },
                controls: None,
            },
            LogicUpdate::default(),
        ))
    }
    fn handle(&mut self, event: LogicEvent, context: &CallContext) -> Result<LogicUpdate, String> {
        if matches!(event, LogicEvent::Action(ref action) if action.action == self.action) {
            if self.action == "stack" {
                std::hint::black_box(recurse(1));
            } else {
                panic!("fixture trap");
            }
        }
        if let Some(controls) = &mut self.controls {
            return controls.handle(event, context);
        }
        Ok(LogicUpdate::default())
    }
}
touchbar_logic_sdk::export!(Adversary);
