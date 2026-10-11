use crate::{FactName, FactValue, ModelError, SystemKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioTarget {
    DefaultSink,
    DefaultSource,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MuteValue {
    On,
    Off,
    Toggle,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bus {
    Session,
    System,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbusValue {
    pub signature: String,
    pub json: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbusCall {
    pub bus: Bus,
    pub destination: String,
    pub path: String,
    pub interface: String,
    pub method: String,
    pub args: DbusValue,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbusMatch {
    pub bus: Bus,
    pub sender: Option<String>,
    pub path: Option<String>,
    pub interface: Option<String>,
    pub member: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbusProperty {
    pub bus: Bus,
    pub destination: String,
    pub path: String,
    pub interface: String,
    pub property: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnSpec {
    pub argv: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub stdin: Option<String>,
    pub max_output: u32,
}
impl SpawnSpec {
    pub fn new(argv: Vec<String>) -> Self {
        Self {
            argv,
            env: BTreeMap::new(),
            stdin: None,
            max_output: 65536,
        }
    }
    pub fn validate(&self) -> Result<(), ModelError> {
        if self.argv.is_empty()
            || self.argv[0].is_empty()
            || self.argv.iter().any(|a| a.contains('\0'))
            || self
                .env
                .iter()
                .any(|(k, v)| k.is_empty() || k.contains(['=', '\0']) || v.contains('\0'))
        {
            Err(ModelError(
                "spawn requires a nonempty argv and valid environment".into(),
            ))
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    AudioSetVolume {
        target: AudioTarget,
        level: f64,
    },
    AudioSetMute {
        target: AudioTarget,
        value: MuteValue,
    },
    MediaPlayPause,
    MediaNext,
    MediaPrevious,
    MediaSeek {
        position_ms: u64,
    },
    DesktopShortcut {
        keys: Vec<String>,
        app: Option<String>,
    },
    DesktopBrightness {
        level: f64,
    },
    DesktopWorkspace {
        id: String,
    },
    DesktopAppLaunch {
        id: String,
    },
    KeyTap {
        key: SystemKey,
    },
    KeyDown {
        key: SystemKey,
    },
    KeyUp {
        key: SystemKey,
    },
    OpenUri {
        uri: String,
    },
    TimerAfter {
        ms: u64,
    },
    SpawnRun(SpawnSpec),
    DbusCall(DbusCall),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lifetime {
    UntilHidden,
    Committed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectDefaults {
    pub lifetime: Lifetime,
    pub timeout_ms: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectPolicy {
    pub lifetime: Lifetime,
    pub timeout_ms: Option<u64>,
    pub coalesce: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectRequest {
    pub tag: String,
    pub effect: Effect,
    pub lifetime: Option<Lifetime>,
    pub coalesce: Option<String>,
    pub timeout_ms: Option<u64>,
}
impl EffectRequest {
    pub fn policy(&self) -> EffectPolicy {
        let defaults = self.effect.defaults();
        EffectPolicy {
            lifetime: self.lifetime.unwrap_or(defaults.lifetime),
            timeout_ms: self.timeout_ms.or(defaults.timeout_ms),
            coalesce: self.coalesce.clone(),
        }
    }
    pub fn validate(&self) -> Result<(), ModelError> {
        if self.tag.is_empty() || self.timeout_ms == Some(0) {
            return Err(ModelError(
                "request requires a tag and a positive timeout".into(),
            ));
        }
        self.effect.validate()
    }
}
impl Effect {
    pub fn defaults(&self) -> EffectDefaults {
        let (lifetime, timeout_ms) = match self {
            Self::AudioSetVolume { .. }
            | Self::AudioSetMute { .. }
            | Self::DesktopShortcut { .. }
            | Self::DesktopBrightness { .. }
            | Self::DesktopWorkspace { .. } => (Lifetime::Committed, Some(1000)),
            Self::MediaPlayPause
            | Self::MediaNext
            | Self::MediaPrevious
            | Self::MediaSeek { .. } => (Lifetime::Committed, Some(2000)),
            Self::DesktopAppLaunch { .. } | Self::OpenUri { .. } => {
                (Lifetime::Committed, Some(5000))
            }
            Self::SpawnRun(_) | Self::DbusCall(_) => (Lifetime::UntilHidden, Some(5000)),
            Self::TimerAfter { .. } => (Lifetime::UntilHidden, None),
            Self::KeyTap { .. } | Self::KeyDown { .. } | Self::KeyUp { .. } => {
                (Lifetime::Committed, None)
            }
        };
        EffectDefaults {
            lifetime,
            timeout_ms,
        }
    }
    pub fn validate(&self) -> Result<(), ModelError> {
        let valid = match self {
            Self::AudioSetVolume { level, .. } => level.is_finite() && (0.0..=1.5).contains(level),
            Self::DesktopBrightness { level } => level.is_finite() && (0.0..=1.0).contains(level),
            Self::DesktopShortcut { keys, .. } => {
                !keys.is_empty()
                    && keys
                        .iter()
                        .all(|key| !key.is_empty() && !key.contains('\0'))
            }
            Self::DesktopWorkspace { id } | Self::DesktopAppLaunch { id } => {
                !id.is_empty() && !id.contains('\0')
            }
            Self::OpenUri { uri } => !uri.is_empty() && !uri.contains('\0'),
            Self::SpawnRun(spec) => return spec.validate(),
            Self::DbusCall(call) => {
                !call.destination.is_empty()
                    && call.path.starts_with('/')
                    && !call.interface.is_empty()
                    && !call.method.is_empty()
            }
            _ => true,
        };
        if valid {
            Ok(())
        } else {
            Err(ModelError("invalid effect parameters".into()))
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectError {
    Unsupported,
    Unavailable,
    Failed(String),
    Timeout,
    Cancelled,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnResult {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EffectResult {
    None,
    Mute(bool),
    Spawn(SpawnResult),
    Dbus(DbusValue),
}
pub type EffectOutcome = Result<EffectResult, EffectError>;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SubscriptionKind {
    AudioWatch { target: AudioTarget },
    MediaWatch,
    DesktopFact { name: FactName },
    TimerInterval { ms: u64 },
    SpawnLines(SpawnSpec),
    DbusSignal(DbusMatch),
    DbusProperty(DbusProperty),
}
impl SubscriptionKind {
    pub fn validate(&self) -> Result<(), ModelError> {
        match self {
            Self::TimerInterval { ms } if *ms < 16 => {
                Err(ModelError("interval must be at least 16 ms".into()))
            }
            Self::SpawnLines(spec) => spec.validate(),
            Self::DesktopFact { name } if name.0.is_empty() => {
                Err(ModelError("fact name is empty".into()))
            }
            _ => Ok(()),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubscriptionSpec {
    pub tag: String,
    pub kind: SubscriptionKind,
    pub background: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioState {
    pub available: bool,
    pub volume: f64,
    pub muted: bool,
    pub name: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaState {
    pub player: Option<String>,
    pub status: PlaybackStatus,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub art_uri: Option<String>,
    pub position_ms: u64,
    pub length_ms: u64,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_next: bool,
    pub can_previous: bool,
    pub can_seek: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SubscriptionData {
    Audio(AudioState),
    Media(MediaState),
    Fact(Option<FactValue>),
    Tick,
    Line(String),
    Dbus(DbusValue),
}
