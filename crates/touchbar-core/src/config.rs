use serde::{Deserialize, Serialize};
use touchbar_logic_types::LogicLimits;
use touchbar_model::{Mm, ModelError};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoreConfig {
    pub max_surfaces: usize,
    pub profile_pending_ms: u64,
    pub backoff_initial_ms: u64,
    pub backoff_max_ms: u64,
    pub effect_slots: usize,
    pub spawn_slots: usize,
    pub session_spawn_slots: usize,
    pub persist_delay_ms: u64,
    pub tap_ms: u64,
    pub tap_slop: Mm,
    pub pan_threshold: Mm,
    pub swipe_distance: Mm,
    #[serde(with = "touchbar_model::wire_float::f32")]
    pub swipe_speed_mm_s: f32,
    pub fallback_distance: Mm,
    pub logic_soft_ms: u64,
    pub logic_hard_ms: u64,
    pub max_update_bytes: usize,
    pub max_requests_per_update: usize,
    pub max_subscriptions: usize,
    pub max_text_bytes: usize,
    pub max_records: usize,
    pub max_persist_bytes: usize,
}
impl Default for CoreConfig {
    fn default() -> Self {
        let logic = LogicLimits::default();
        Self {
            max_surfaces: 8,
            profile_pending_ms: 1500,
            backoff_initial_ms: 1000,
            backoff_max_ms: 60_000,
            effect_slots: 8,
            spawn_slots: 4,
            session_spawn_slots: 64,
            persist_delay_ms: 1000,
            tap_ms: 350,
            tap_slop: Mm(1.3),
            pan_threshold: Mm(1.0),
            swipe_distance: Mm(4.0),
            swipe_speed_mm_s: 40.0,
            fallback_distance: Mm(3.0),
            logic_soft_ms: logic.logic_soft_ms,
            logic_hard_ms: logic.logic_hard_ms,
            max_update_bytes: logic.max_update_bytes,
            max_requests_per_update: logic.max_requests_per_update,
            max_subscriptions: logic.max_subscriptions,
            max_text_bytes: logic.max_text_bytes,
            max_records: logic.max_records,
            max_persist_bytes: logic.max_persist_bytes,
        }
    }
}
impl CoreConfig {
    pub fn validate(&self) -> Result<(), ModelError> {
        if self.max_surfaces == 0
            || self.max_surfaces > 8
            || self.profile_pending_ms == 0
            || self.backoff_initial_ms == 0
            || self.backoff_max_ms < self.backoff_initial_ms
            || self.effect_slots == 0
            || self.spawn_slots == 0
            || self.session_spawn_slots == 0
            || self.persist_delay_ms == 0
            || self.tap_ms == 0
            || self.logic_soft_ms == 0
            || self.logic_hard_ms < self.logic_soft_ms
            || [
                self.max_update_bytes,
                self.max_requests_per_update,
                self.max_subscriptions,
                self.max_text_bytes,
                self.max_records,
                self.max_persist_bytes,
            ]
            .contains(&0)
            || [
                self.tap_slop.0,
                self.pan_threshold.0,
                self.swipe_distance.0,
                self.swipe_speed_mm_s,
                self.fallback_distance.0,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
        {
            Err(ModelError(
                "invalid core limits or gesture thresholds".into(),
            ))
        } else {
            Ok(())
        }
    }
}
