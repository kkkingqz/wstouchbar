use crate::CoreConfig;
use serde::{Deserialize, Serialize};
use touchbar_model::{Point, Timestamp};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Winner {
    LongPress,
    Pan,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum GestureEvent {
    Tap,
    LongPressed,
    PanBegan,
    PanChanged,
    PanEnded,
    Swipe,
}
/// One arena per accepted contact. Capture routing owns the arena's identity.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct GestureArena {
    pub origin: Point,
    pub last: Point,
    began: Timestamp,
    max_distance: f64,
    density: f32,
    hold_ms: Option<u64>,
    pan: bool,
    winner: Option<Winner>,
}
impl GestureArena {
    pub fn new(
        origin: Point,
        began: Timestamp,
        density: f32,
        hold_ms: Option<u64>,
        pan: bool,
    ) -> Self {
        Self {
            origin,
            last: origin,
            began,
            density,
            hold_ms,
            pan,
            winner: None,
            max_distance: 0.0,
        }
    }
    pub fn motion(&mut self, point: Point, config: &CoreConfig) -> Vec<GestureEvent> {
        self.last = point;
        let dx = f64::from(point.x) - f64::from(self.origin.x);
        let dy = f64::from(point.y) - f64::from(self.origin.y);
        self.max_distance = self.max_distance.max(dx.hypot(dy));
        if self.winner == Some(Winner::Pan) {
            return vec![GestureEvent::PanChanged];
        }
        if self.winner.is_none()
            && self.pan
            && dx.abs() >= dy.abs()
            && dx.abs() >= threshold(config.pan_threshold.0, self.density)
        {
            self.winner = Some(Winner::Pan);
            return vec![GestureEvent::PanBegan, GestureEvent::PanChanged];
        }
        vec![]
    }
    pub fn advance(&mut self, now: Timestamp, config: &CoreConfig) -> Vec<GestureEvent> {
        if self.winner.is_none()
            && self.hold_deadline().is_some_and(|at| at <= now)
            && self.max_distance <= threshold(config.tap_slop.0, self.density)
        {
            self.winner = Some(Winner::LongPress);
            vec![GestureEvent::LongPressed]
        } else {
            vec![]
        }
    }
    pub fn finish(
        &mut self,
        point: Point,
        now: Timestamp,
        config: &CoreConfig,
    ) -> Vec<GestureEvent> {
        let dx = f64::from(point.x) - f64::from(self.origin.x);
        let dy = f64::from(point.y) - f64::from(self.origin.y);
        self.max_distance = self.max_distance.max(dx.hypot(dy));
        let elapsed = now.elapsed_since(self.began);
        match self.winner {
            Some(Winner::Pan) => {
                let mut events = vec![GestureEvent::PanEnded];
                if elapsed > 0
                    && dx.abs() >= dy.abs()
                    && dx.abs() >= threshold(config.swipe_distance.0, self.density)
                    && dx.abs() * 1000.0 / elapsed as f64
                        >= f64::from(config.swipe_speed_mm_s) * f64::from(self.density)
                {
                    events.push(GestureEvent::Swipe);
                }
                events
            }
            None if elapsed <= config.tap_ms
                && self.max_distance <= threshold(config.tap_slop.0, self.density) =>
            {
                vec![GestureEvent::Tap]
            }
            None | Some(Winner::LongPress) => vec![],
        }
    }
    pub fn hold_deadline(&self) -> Option<Timestamp> {
        if self.winner.is_none() {
            self.hold_ms.map(|ms| self.began.after(ms))
        } else {
            None
        }
    }
    pub fn held(&self) -> bool {
        self.winner == Some(Winner::LongPress)
    }
}
fn threshold(mm: f32, density: f32) -> f64 {
    let px = mm * density;
    if px.is_finite() && px > 0.0 {
        f64::from(px)
    } else {
        f64::from(mm) * f64::from(density)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn p(x: f32) -> Point {
        Point { x, y: 0.0 }
    }
    #[test]
    fn finite_extreme_coordinates_keep_arena_serializable() {
        let mut a = GestureArena::new(p(f32::MAX), Timestamp(0), 5.0, None, false);
        a.motion(p(-f32::MAX), &CoreConfig::default());
        let encoded = serde_json::to_string(&a).unwrap();
        let mut restored: GestureArena = serde_json::from_str(&encoded).unwrap();
        assert!(
            restored
                .finish(p(0.0), Timestamp(10), &CoreConfig::default())
                .is_empty()
        );
    }
    #[test]
    fn thresholds_scale_with_density_and_swipe_requires_distance_and_speed() {
        let config = CoreConfig::default();
        for density in [3.0, 6.0, 12.0] {
            let mut a = GestureArena::new(p(0.0), Timestamp(0), density, None, true);
            assert!(a.motion(p(0.9 * density), &config).is_empty());
            assert_eq!(
                a.motion(p(density), &config),
                vec![GestureEvent::PanBegan, GestureEvent::PanChanged]
            );
            assert_eq!(
                a.finish(p(4.0 * density), Timestamp(100), &config),
                vec![GestureEvent::PanEnded, GestureEvent::Swipe]
            );
            let mut slow = GestureArena::new(p(0.0), Timestamp(0), density, None, true);
            slow.motion(p(density), &config);
            assert_eq!(
                slow.finish(p(4.0 * density), Timestamp(101), &config),
                vec![GestureEvent::PanEnded]
            );
        }
    }
    #[test]
    fn moving_away_and_back_never_resurrects_tap_or_hold() {
        let config = CoreConfig::default();
        let mut a = GestureArena::new(p(0.0), Timestamp(0), 5.0, Some(400), false);
        a.motion(p(7.0), &config);
        a.motion(p(0.0), &config);
        assert!(a.advance(Timestamp(400), &config).is_empty());
        assert!(a.finish(p(0.0), Timestamp(300), &config).is_empty());
    }
    #[test]
    fn tap_duration_and_long_press_are_exclusive() {
        let config = CoreConfig::default();
        let mut tap = GestureArena::new(p(0.0), Timestamp(0), 5.0, None, false);
        assert_eq!(
            tap.finish(p(6.5), Timestamp(350), &config),
            vec![GestureEvent::Tap]
        );
        let mut late = GestureArena::new(p(0.0), Timestamp(0), 5.0, None, false);
        assert!(late.finish(p(0.0), Timestamp(351), &config).is_empty());
        let mut held = GestureArena::new(p(0.0), Timestamp(0), 5.0, Some(400), true);
        assert_eq!(
            held.advance(Timestamp(400), &config),
            vec![GestureEvent::LongPressed]
        );
        assert!(held.motion(p(100.0), &config).is_empty());
        assert!(held.finish(p(0.0), Timestamp(500), &config).is_empty());
    }
}
