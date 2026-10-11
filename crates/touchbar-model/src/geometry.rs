use crate::{InstanceId, WidgetId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
impl Rect {
    pub fn contains(self, p: Point) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.x + self.width && p.y < self.y + self.height
    }
    pub fn vertical_distance(self, p: Point) -> Option<f32> {
        if p.x < self.x || p.x >= self.x + self.width {
            return None;
        }
        Some((self.y - p.y).max(p.y - self.y - self.height).max(0.0))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ScenePlane {
    Profile,
    Layer(u64),
    Presentation,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HitTarget {
    pub instance: InstanceId,
    pub widget: WidgetId,
    pub plane: ScenePlane,
    pub rect: Rect,
    pub record_key: Option<String>,
}
pub type HitMap = Vec<HitTarget>;
