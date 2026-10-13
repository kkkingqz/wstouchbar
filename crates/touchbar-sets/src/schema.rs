use serde::{Deserialize, Serialize};
use touchbar_model::{Length, SystemKey, UnitSize};

fn zero() -> Length {
    Length::Unit(0.0)
}
fn default_true() -> bool {
    true
}
fn preferred() -> Length {
    Length::Unit(160.0)
}
fn maximum() -> Length {
    Length::Unit(8192.0)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetFile {
    pub version: u32,
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub logic_config: String,
    #[serde(default)]
    pub persist: Vec<String>,
    pub units: Option<Units>,
    pub layout: LayoutNode,
    #[serde(default)]
    pub panels: Vec<PanelDefinition>,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Units {
    pub u: UnitSize,
}

/// Panel widgets belong to the owner's logic instance (D48/D76).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelDefinition {
    pub id: String,
    pub layout: LayoutNode,
    pub principal_item: Option<String>,
    #[serde(default = "zero")]
    pub minimum_width: Length,
    #[serde(default = "preferred")]
    pub preferred_width: Length,
    #[serde(default = "maximum")]
    pub maximum_width: Length,
    #[serde(default = "default_true")]
    pub dismiss_on_selection: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyDefinition {
    pub key: SystemKey,
    pub id: Option<String>,
    pub width: Option<Length>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutKind {
    Row,
    Column,
    Stack,
    Button,
    Text,
    Icon,
    Image,
    Slider,
    Meter,
    Progress,
    Scrubber,
    Repeat,
    Canvas,
    Responsive,
    ShaderEffect,
    FixedSpace,
    FlexibleSpace,
    Key,
    KeyRow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColorRole {
    Background,
    Control,
    ControlPressed,
    Track,
    Foreground,
    Muted,
    Accent,
    Destructive,
    OnAccent,
    OnDestructive,
}

/// The allowed property names are fixed; validation also checks which node
/// kinds can use children, repetition and event bindings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutNode {
    pub kind: LayoutKind,
    pub id: Option<String>,
    #[serde(default)]
    pub children: Vec<LayoutNode>,
    #[serde(default = "zero")]
    pub spacing: Length,
    #[serde(default = "zero")]
    pub padding: Length,
    pub width: Option<Length>,
    pub height: Option<Length>,
    pub minimum: Option<Length>,
    pub maximum: Option<Length>,
    pub grow: Option<f32>,
    pub label: Option<String>,
    pub label_from: Option<String>,
    pub text: Option<String>,
    pub text_from: Option<String>,
    pub symbol: Option<String>,
    pub symbol_from: Option<String>,
    pub asset: Option<String>,
    pub asset_from: Option<String>,
    pub color: Option<ColorRole>,
    pub color_from: Option<String>,
    pub background: Option<ColorRole>,
    pub background_from: Option<String>,
    #[serde(default)]
    pub plain: bool,
    pub pressed_background: Option<ColorRole>,
    pub pressed_background_from: Option<String>,
    pub corner_radius: Option<Length>,
    pub font_size: Option<Length>,
    pub value: Option<f64>,
    pub value_from: Option<String>,
    pub enabled_from: Option<String>,
    pub visible_from: Option<String>,
    pub opacity: Option<f32>,
    pub disabled_opacity: Option<f32>,
    pub on_pressed: Option<String>,
    pub on_activate: Option<String>,
    pub on_long_pressed: Option<String>,
    pub on_value_changed: Option<String>,
    pub on_selected: Option<String>,
    pub on_released: Option<String>,
    pub on_cancelled: Option<String>,
    pub repeat_from: Option<String>,
    pub repeat_key: Option<String>,
    pub template: Option<Box<LayoutNode>>,
    pub canvas_from: Option<String>,
    #[serde(default)]
    pub variants: Vec<ResponsiveDefinition>,
    pub effect: Option<EffectDefinition>,
    pub motion: Option<MotionDefinition>,
    pub selected_from: Option<String>,
    pub pressed_from: Option<String>,
    pub hold_ms: Option<u64>,
    pub expanded_panel: Option<String>,
    pub press_and_hold_panel: Option<String>,
    pub key: Option<SystemKey>,
    #[serde(default)]
    pub keys: Vec<KeyDefinition>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponsiveDefinition {
    pub representation: String,
    pub minimum_width: Length,
    pub layout: Box<LayoutNode>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScalarBinding {
    Number(f64),
    State(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectDefinition {
    pub id: u64,
    pub source: String,
    #[serde(default)]
    pub parameters: Vec<ScalarBinding>,
    pub period_ms: Option<u32>,
    pub opacity: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransformDefinition {
    pub x: Option<ScalarBinding>,
    pub y: Option<ScalarBinding>,
    pub scale: Option<ScalarBinding>,
    pub opacity: Option<ScalarBinding>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionDefinition {
    pub id: u64,
    pub duration_ms: u32,
    pub easing: String,
    pub playback: String,
    pub from: TransformDefinition,
    pub to: TransformDefinition,
}
