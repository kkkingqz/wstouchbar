use crate::{
    EffectError, EffectOutcome, EffectRequest, PanelId, RecordValue, Scalar, ScalarKind,
    SubscriptionData, SubscriptionSpec,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ActionKind {
    Pressed,
    LongPressed,
    Activated,
    Released,
    Cancelled,
    ValueChanged,
    Selected,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Origin {
    Physical,
    Synthetic,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CancelReason {
    Hidden,
    Layer,
    Presentation,
    Profile,
    Layout,
    Mode,
    Revoked,
    Fault,
    Hardware,
    SurfaceRemoved,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionEvent {
    pub action: String,
    pub kind: ActionKind,
    pub value: Option<Scalar>,
    pub record_key: Option<String>,
    pub origin: Origin,
    pub cancel_reason: Option<CancelReason>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogicContext {
    pub surface_kind: String,
    pub slot_width: f32,
    pub representation: String,
    pub color_scheme: String,
    pub accent: Option<String>,
    pub locale: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LogicEvent {
    Action(ActionEvent),
    Shown,
    Hidden,
    ContextChanged(LogicContext),
    PresentationClosed(PanelId),
    EffectDone { tag: String, result: EffectOutcome },
    SubscriptionData { tag: String, data: SubscriptionData },
    SubscriptionLost { tag: String, error: EffectError },
}
/// Host calls. Init dispatches to WIT init; Handle dispatches to WIT handle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LogicCall {
    Init {
        config: String,
        persisted: StatePatch,
    },
    Handle(LogicEvent),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum StateValue {
    Scalar(Scalar),
    Records(Vec<RecordValue>),
    Canvas(Vec<CanvasCommand>),
}
pub type StatePatch = Vec<(String, StateValue)>;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FieldKind {
    Scalar(ScalarKind),
    Records(Vec<(String, ScalarKind)>),
    Canvas,
}
impl FieldKind {
    pub fn accepts(&self, value: &StateValue) -> bool {
        match (self, value) {
            (Self::Scalar(kind), StateValue::Scalar(v)) => {
                *kind == v.kind() && v.validate().is_ok()
            }
            (Self::Records(fields), StateValue::Records(records)) => records.iter().all(|record| {
                record.len() == fields.len()
                    && fields.iter().all(|(name, kind)| {
                        record
                            .get(name)
                            .is_some_and(|v| v.kind() == *kind && v.validate().is_ok())
                    })
            }),
            (Self::Canvas, StateValue::Canvas(_)) => true,
            _ => false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateField {
    pub name: String,
    pub kind: FieldKind,
    pub persist: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionDescription {
    pub kinds: Vec<ActionKind>,
    pub value: Option<ScalarKind>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LogicDescription {
    pub actions: Vec<(String, ActionDescription)>,
    pub state: Vec<StateField>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Placement {
    Anchored,
    Region,
    Full,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresentationLifecycle {
    Transient,
    Persistent,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PresentationCommand {
    Open {
        panel: PanelId,
        placement: Placement,
        lifecycle: PresentationLifecycle,
    },
    Close,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LogicUpdate {
    pub state: StatePatch,
    pub requests: Vec<EffectRequest>,
    pub cancel: Vec<String>,
    pub subscriptions: Option<Vec<SubscriptionSpec>>,
    pub presentation: Option<PresentationCommand>,
}

// Canvas data follows the source project's foundation/WIT (d2f9dfd), without renderer dependencies.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum CanvasColor {
    Role(ColorRole),
    Rgba {
        red: f32,
        green: f32,
        blue: f32,
        alpha: f32,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanvasPaint {
    pub color: CanvasColor,
    pub opacity: f32,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanvasLinearGradient {
    pub start: crate::Point,
    pub end: crate::Point,
    pub start_color: CanvasPaint,
    pub end_color: CanvasPaint,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlign {
    Leading,
    Center,
    Trailing,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CanvasCommand {
    FillRect {
        rect: crate::Rect,
        radius: f32,
        paint: CanvasPaint,
    },
    Line {
        start: crate::Point,
        end: crate::Point,
        width: f32,
        paint: CanvasPaint,
    },
    Polyline {
        points: Vec<crate::Point>,
        width: f32,
        paint: CanvasPaint,
    },
    FillCircle {
        center: crate::Point,
        radius: f32,
        paint: CanvasPaint,
    },
    Text {
        rect: crate::Rect,
        text: String,
        size: f32,
        paint: CanvasPaint,
        align: TextAlign,
    },
    FillLinearGradientRect {
        rect: crate::Rect,
        radius: f32,
        gradient: CanvasLinearGradient,
    },
    StrokePath {
        segments: Vec<CanvasPathSegment>,
        width: f32,
        paint: CanvasPaint,
    },
    FillPath {
        segments: Vec<CanvasPathSegment>,
        paint: CanvasPaint,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum CanvasPathSegment {
    MoveTo(crate::Point),
    LineTo(crate::Point),
    QuadraticTo {
        control: crate::Point,
        endpoint: crate::Point,
    },
    CubicTo {
        control_one: crate::Point,
        control_two: crate::Point,
        endpoint: crate::Point,
    },
    Close,
}
