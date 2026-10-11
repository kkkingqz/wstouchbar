use crate::{ProfileSwitch, Resolution};
use serde::{Deserialize, Serialize};
use touchbar_model::*;

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
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub profile: Option<ProfileRef>,
    pub items: Vec<SceneItem>,
}
pub type Representations = Vec<(InstanceId, String)>;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Input {
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
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoreSnapshot {
    pub facts: Facts,
    pub surfaces: Vec<SurfaceSnapshot>,
    pub plugins: Vec<PluginId>,
    pub now: Timestamp,
    pub instances: Vec<InstanceSnapshot>,
}
