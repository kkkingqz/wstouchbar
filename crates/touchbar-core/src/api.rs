use crate::{ProfileSwitch, Resolution};
use serde::{Deserialize, Serialize};
use touchbar_model::*;

/// Stable address of persisted fields, independent of runtime generations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistKey {
    pub plugin: PluginId,
    pub surface: SurfaceName,
    pub scope: PersistScope,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersistScope {
    ProfileSlot(SlotId),
    Layer(SetRef),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Display,
    Native,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstanceStatus {
    Loading,
    Hidden,
    Visible,
    Faulted,
    Disposed,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstanceSnapshot {
    pub id: InstanceId,
    pub surface: SurfaceId,
    pub set: SetRef,
    pub status: InstanceStatus,
    pub state: StatePatch,
    pub in_flight: Option<CallId>,
    pub queue_len: usize,
    pub retry_at: Option<Timestamp>,
    pub fault: Option<String>,
    pub persist_key: PersistKey,
    pub call: Option<CallSnapshot>,
    pub queue: Vec<LogicCall>,
    pub requests: Vec<RequestSnapshot>,
    pub subscriptions: Vec<SubscriptionSnapshot>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum KeySource {
    Surface(SurfaceId),
    Desktop,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopCapabilities {
    pub key: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneItem {
    pub instance: InstanceId,
    pub slot: SlotId,
    pub set: SetSpec,
    pub state: StatePatch,
    pub plane: ScenePlane,
    pub units: UnitSize,
    pub status: InstanceStatus,
    /// Host interaction state, independent of the logic plugin's state patch.
    pub interactions: Vec<WidgetInteraction>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScrubberState {
    pub offset_px: f32,
    pub highlighted: Option<String>,
    pub selected: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WidgetInteraction {
    pub widget: WidgetId,
    pub record_key: Option<String>,
    pub pressed: bool,
    pub value: Option<f64>,
    pub scrubber: ScrubberState,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScrubberItemGeometry {
    pub record_key: String,
    /// Unscrolled content coordinates, relative to the viewport origin.
    pub rect: Rect,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScrubberLayout {
    pub instance: InstanceId,
    pub widget: WidgetId,
    pub viewport: Rect,
    pub content_width: f32,
    pub items: Vec<ScrubberItemGeometry>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub profile: Option<ProfileRef>,
    pub items: Vec<SceneItem>,
    pub presentation: Option<PresentationScene>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PresentationScene {
    pub id: PresentationId,
    pub owner: InstanceId,
    pub panel: PanelSpec,
    pub placement: Placement,
    pub lifecycle: PresentationLifecycle,
    pub anchor: Rect,
    pub region: Rect,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotBounds {
    pub instance: InstanceId,
    pub rect: Rect,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PresentationLayout {
    pub id: PresentationId,
    pub bounds: Rect,
}
pub type Representations = Vec<(InstanceId, String)>;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Input {
    Locale(String),
    SurfaceAdded(SurfaceInfo),
    SurfaceChanged(SurfaceInfo),
    SurfaceRemoved(SurfaceId),
    Touch {
        surface: SurfaceId,
        contact: ContactId,
        phase: TouchPhase,
        x: f32,
        y: f32,
    },
    Key {
        source: KeySource,
        code: WatchKey,
        phase: KeyPhase,
    },
    Mode {
        surface: SurfaceId,
        mode: Mode,
    },
    Revoked {
        surface: SurfaceId,
    },
    Layout {
        surface: SurfaceId,
        generation: Generation,
        hit_map: HitMap,
        representations: Representations,
        scrubbers: Vec<ScrubberLayout>,
        slots: Vec<SlotBounds>,
        presentation: Option<PresentationLayout>,
    },
    Fact {
        name: FactName,
        value: Option<FactValue>,
    },
    Catalog(Catalog),
    DesktopCapabilities(DesktopCapabilities),
    LogicDescribed {
        instance: InstanceId,
        description: LogicDescription,
        persisted: StatePatch,
    },
    LogicUpdate {
        instance: InstanceId,
        call: CallId,
        update: LogicUpdate,
    },
    LogicFault {
        instance: InstanceId,
        call: CallId,
        reason: String,
    },
    LogicError {
        instance: InstanceId,
        call: CallId,
        message: String,
    },
    InstanceFault {
        instance: InstanceId,
        reason: String,
    },
    EffectDone {
        request: RequestId,
        result: EffectOutcome,
    },
    SubscriptionData {
        subscription: SubscriptionId,
        data: SubscriptionData,
    },
    SubscriptionLost {
        subscription: SubscriptionId,
        error: EffectError,
    },
    Timer {
        id: TimerId,
    },
    FrameDone {
        surface: SurfaceId,
        generation: Generation,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Output {
    CallLogic {
        instance: InstanceId,
        call: CallId,
        invocation: LogicCall,
    },
    CreateInstance {
        instance: InstanceId,
        surface: SurfaceId,
        plugin: PluginId,
        set: SetRef,
        persist_key: PersistKey,
    },
    DisposeInstance {
        instance: InstanceId,
    },
    StartEffect {
        request: RequestId,
        effect: Effect,
        policy: EffectPolicy,
    },
    CancelEffect {
        request: RequestId,
    },
    Subscribe {
        subscription: SubscriptionId,
        kind: SubscriptionKind,
    },
    Unsubscribe {
        subscription: SubscriptionId,
    },
    SendKey {
        surface: SurfaceId,
        key: SystemKey,
        phase: KeyPhase,
    },
    DesktopKey {
        surface: SurfaceId,
        key: SystemKey,
        phase: KeyPhase,
    },
    WatchKeys {
        surface: SurfaceId,
        set: Vec<WatchKey>,
    },
    Backlight {
        surface: SurfaceId,
        limit: f32,
    },
    Render {
        surface: SurfaceId,
        generation: Generation,
        scene: Scene,
    },
    Persist {
        instance: InstanceId,
        key: PersistKey,
        fields: StatePatch,
    },
    SetTimer {
        id: TimerId,
        at: Timestamp,
    },
    ClearTimer {
        id: TimerId,
    },
    Log {
        level: LogLevel,
        event: String,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfaceSnapshot {
    pub info: SurfaceInfo,
    pub mode: Mode,
    pub generation: Generation,
    pub layout_generation: Option<Generation>,
    pub scene: Scene,
    pub resolution: Resolution,
    pub profile: ProfileSwitch,
    pub layers: Vec<LayerSnapshot>,
    pub presentation: Option<PresentationScene>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoreSnapshot {
    pub facts: Facts,
    pub surfaces: Vec<SurfaceSnapshot>,
    pub plugins: Vec<PluginId>,
    pub now: Timestamp,
    pub instances: Vec<InstanceSnapshot>,
    pub captures: Vec<CaptureSnapshot>,
    pub requests: Vec<RequestSnapshot>,
    pub subscriptions: Vec<SubscriptionSnapshot>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptureSnapshot {
    pub id: CaptureId,
    pub surface: SurfaceId,
    pub contact: ContactId,
    pub target: HitTarget,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CallSnapshot {
    pub id: CallId,
    pub invocation: LogicCall,
    pub discard_update: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RequestSnapshot {
    pub id: RequestId,
    pub surface: SurfaceId,
    pub tag: String,
    pub effect: Effect,
    pub policy: EffectPolicy,
    pub running: bool,
    pub deadline: Option<Timestamp>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubscriptionSnapshot {
    pub owner: InstanceId,
    pub spec: SubscriptionSpec,
    pub active: Option<SubscriptionId>,
    pub failures: u32,
    pub retry_at: Option<Timestamp>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerSnapshot {
    pub set: SetRef,
    pub latched: bool,
    pub held: Vec<(KeySource, String)>,
}
