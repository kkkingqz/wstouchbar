#![cfg(target_arch = "wasm32")]

use touchbar_logic_sdk::{CallContext, Description, Logic, Update};
use touchbar_model::*;

struct Counter(f64);
impl Logic for Counter {
    fn describe() -> LogicDescription {
        Description::new()
            .field("count", FieldKind::Scalar(ScalarKind::Number), false)
            .build()
    }
    fn init(_: &str, _: StatePatch, ctx: &CallContext) -> Result<(Self, LogicUpdate), String> {
        touchbar_logic_sdk::log::info!("counter init at {}", ctx.now_ms);
        Ok((Self(0.0), Update::new().set("count", 0.0).build()))
    }
    fn handle(&mut self, _: LogicEvent, _: &CallContext) -> Result<LogicUpdate, String> {
        self.0 += 1.0;
        Ok(Update::new().set("count", self.0).build())
    }
}
touchbar_logic_sdk::export!(Counter);
