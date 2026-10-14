use touchbar_core::CoreConfig;
use touchbar_logic_types::LogicLimits;

#[test]
fn core_uses_shared_logic_defaults_without_changing_its_serialized_config() {
    let core = CoreConfig::default();
    let limits = LogicLimits::default();
    assert_eq!(core.logic_soft_ms, limits.logic_soft_ms);
    assert_eq!(core.logic_hard_ms, limits.logic_hard_ms);
    assert_eq!(core.max_update_bytes, limits.max_update_bytes);
    assert_eq!(core.max_requests_per_update, limits.max_requests_per_update);
    assert_eq!(core.max_subscriptions, limits.max_subscriptions);
    assert_eq!(core.max_text_bytes, limits.max_text_bytes);
    assert_eq!(core.max_records, limits.max_records);
    assert_eq!(core.max_persist_bytes, limits.max_persist_bytes);
    let json = serde_json::to_value(&core).unwrap();
    assert_eq!(json.as_object().unwrap().len(), 22);
    assert_eq!(serde_json::from_value::<CoreConfig>(json).unwrap(), core);
}
