use touchbar_logic_sdk::{CallContext, Description, Logic};
use touchbar_model::*;
pub struct Echo;
impl Logic for Echo {
    fn describe() -> LogicDescription {
        touchbar_logic_sdk::log::info!("echo.describe");
        Description::new()
            .field("last", FieldKind::Scalar(ScalarKind::Text), false)
            .build()
    }
    fn init(config: &str, _: StatePatch, _: &CallContext) -> Result<(Self, LogicUpdate), String> {
        Ok((
            Self,
            if config.is_empty() {
                LogicUpdate::default()
            } else {
                serde_json::from_str(config).map_err(|e| e.to_string())?
            },
        ))
    }
    fn handle(&mut self, event: LogicEvent, ctx: &CallContext) -> Result<LogicUpdate, String> {
        let last =
            serde_json::json!({"event": event, "context": {"now_ms": ctx.now_ms}}).to_string();
        Ok(LogicUpdate {
            state: vec![("last".into(), StateValue::Scalar(Scalar::Text(last)))],
            ..Default::default()
        })
    }
}
touchbar_logic_sdk::export!(Echo);
