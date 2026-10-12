use touchbar_logic_sdk::{CallContext, Description, Logic, Update};
use touchbar_model::*;
pub struct Persist(f64);
impl Logic for Persist {
    fn describe() -> LogicDescription {
        Description::new()
            .action(
                "change",
                vec![ActionKind::Activated, ActionKind::ValueChanged],
                Some(ScalarKind::Number),
            )
            .field("value", FieldKind::Scalar(ScalarKind::Number), true)
            .field("label", FieldKind::Scalar(ScalarKind::Text), true)
            .build()
    }
    fn init(
        _: &str,
        persisted: StatePatch,
        _: &CallContext,
    ) -> Result<(Self, LogicUpdate), String> {
        let value = persisted
            .iter()
            .find_map(|(key, value)| match (key.as_str(), value) {
                ("value", StateValue::Scalar(Scalar::Number(value))) => Some(*value),
                _ => None,
            })
            .unwrap_or(0.0);
        let mut update = Update::new()
            .set("value", value)
            .set("label", "initial")
            .build();
        for (key, value) in persisted {
            if key == "label" {
                update.state.retain(|(k, _)| k != &key);
                update.state.push((key, value));
            }
        }
        Ok((Self(value), update))
    }
    fn handle(&mut self, event: LogicEvent, _: &CallContext) -> Result<LogicUpdate, String> {
        if let LogicEvent::Action(action) = event {
            self.0 = if let Some(Scalar::Number(value)) = action.value {
                value
            } else {
                self.0 + 1.0
            };
            return Ok(Update::new().set("value", self.0).build());
        }
        Ok(LogicUpdate::default())
    }
}
touchbar_logic_sdk::export!(Persist);
