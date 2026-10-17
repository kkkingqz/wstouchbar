use crate::invalid;
use serde::{Deserialize, Serialize};
use std::{io, num::NonZeroU32};
use touchbar_model::{KeyPhase, SystemKey, TouchPhase, WatchKey};
pub const MESSAGE_SIZE: usize = 32;
pub const VERSION: u16 = 2;
pub const MESSAGE_MAGIC: &[u8; 8] = b"TBARMSG2";
pub const MAX_FRAME_DAMAGE: usize = 8;
pub const MAX_DIMENSION: u32 = 16384;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Native,
    Display,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RevokedReason {
    NotActive,
    SessionInactive,
    Replaced,
    Version,
    Shutdown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModeReason {
    Owner,
    Recovery,
    Locked,
    Sleep,
    Error,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}
impl DamageRect {
    pub fn fits(self, width: u32, height: u32) -> bool {
        self.width > 0
            && self.height > 0
            && u32::from(self.x) + u32::from(self.width) <= width
            && u32::from(self.y) + u32::from(self.height) <= height
    }
    fn packed(self) -> u64 {
        u64::from(self.x)
            | u64::from(self.y) << 16
            | u64::from(self.width) << 32
            | u64::from(self.height) << 48
    }
    fn decode(bits: u64) -> io::Result<Self> {
        let b = bits.to_le_bytes();
        let rect = Self {
            x: u16::from_le_bytes([b[0], b[1]]),
            y: u16::from_le_bytes([b[2], b[3]]),
            width: u16::from_le_bytes([b[4], b[5]]),
            height: u16::from_le_bytes([b[6], b[7]]),
        };
        if rect.width == 0 || rect.height == 0 {
            return Err(invalid("empty damage"));
        }
        Ok(rect)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionMessage {
    Hello {
        version: u16,
    },
    FrameReady {
        index: u32,
        frame: u64,
    },
    FrameDamage(DamageRect),
    Key {
        key: SystemKey,
        phase: KeyPhase,
    },
    WatchKey {
        key: WatchKey,
        enabled: bool,
    },
    /// Fixed point upper limit: 0 = off, 65535 = unrestricted.
    Backlight {
        limit: u16,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DaemonMessage {
    Welcome {
        caps: u32,
        width: u32,
        height: u32,
        width_mm100: u32,
        height_mm100: u32,
    },
    Revoked(RevokedReason),
    ModeChanged {
        mode: Mode,
        reason: ModeReason,
    },
    Touch {
        phase: TouchPhase,
        contact: NonZeroU32,
        x: i32,
        y: i32,
    },
    KeyEvent {
        key: WatchKey,
        phase: KeyPhase,
    },
    BufferReleased {
        index: u32,
        frame: u64,
    },
    FramePresented {
        index: u32,
        frame: u64,
        commit_us: u32,
    },
}
struct Fields {
    kind: u16,
    value: u32,
    payload: u64,
    x: i32,
    y: i32,
}
impl Fields {
    fn new(kind: u16, value: u32, payload: u64, x: i32, y: i32) -> Self {
        Self {
            kind,
            value,
            payload,
            x,
            y,
        }
    }
    fn encode(self) -> [u8; 32] {
        let mut bytes = [0; 32];
        bytes[..8].copy_from_slice(MESSAGE_MAGIC);
        bytes[8..10].copy_from_slice(&VERSION.to_le_bytes());
        bytes[10..12].copy_from_slice(&self.kind.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.value.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.payload.to_le_bytes());
        bytes[24..28].copy_from_slice(&self.x.to_le_bytes());
        bytes[28..32].copy_from_slice(&self.y.to_le_bytes());
        bytes
    }
    fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() != MESSAGE_SIZE
            || &bytes[..8] != MESSAGE_MAGIC
            || bytes[8..10] != VERSION.to_le_bytes()
        {
            return Err(invalid("invalid v2 message length, magic or version"));
        }
        Ok(Self {
            kind: u16::from_le_bytes(bytes[10..12].try_into().expect("checked length")),
            value: u32::from_le_bytes(bytes[12..16].try_into().expect("checked length")),
            payload: u64::from_le_bytes(bytes[16..24].try_into().expect("checked length")),
            x: i32::from_le_bytes(bytes[24..28].try_into().expect("checked length")),
            y: i32::from_le_bytes(bytes[28..32].try_into().expect("checked length")),
        })
    }
}
fn phase(value: i32) -> io::Result<KeyPhase> {
    match value {
        0 => Ok(KeyPhase::Released),
        1 => Ok(KeyPhase::Pressed),
        _ => Err(invalid("invalid key phase")),
    }
}
fn phase_code(phase: KeyPhase) -> i32 {
    match phase {
        KeyPhase::Released => 0,
        KeyPhase::Pressed => 1,
    }
}
macro_rules! key_codec {
    ($encode:ident,$decode:ident,$ty:ident,[$($variant:ident=$code:expr),* $(,)?]) => {
        fn $encode(key:$ty)->u32 {match key {$($ty::$variant=>$code),*}}
        fn $decode(code:u32)->io::Result<$ty> {match code {$($code=>Ok($ty::$variant)),*,_=>Err(invalid("unknown key"))}}
    };
}
key_codec!(
    system_code,
    system_key,
    SystemKey,
    [
        BrightnessDown = 1,
        BrightnessUp = 2,
        KeyboardIlluminationDown = 3,
        KeyboardIlluminationUp = 4,
        PreviousSong = 5,
        PlayPause = 6,
        NextSong = 7,
        Mute = 8,
        VolumeDown = 9,
        VolumeUp = 10,
        MicrophoneMute = 11,
        Search = 12,
        Esc = 20,
        F1 = 21,
        F2 = 22,
        F3 = 23,
        F4 = 24,
        F5 = 25,
        F6 = 26,
        F7 = 27,
        F8 = 28,
        F9 = 29,
        F10 = 30,
        F11 = 31,
        F12 = 32
    ]
);
key_codec!(
    watch_code,
    watch_key,
    WatchKey,
    [
        Fn = 1,
        Esc = 2,
        LeftShift = 3,
        RightShift = 4,
        LeftControl = 5,
        RightControl = 6,
        LeftAlt = 7,
        RightAlt = 8,
        LeftMeta = 9,
        RightMeta = 10,
        F1 = 21,
        F2 = 22,
        F3 = 23,
        F4 = 24,
        F5 = 25,
        F6 = 26,
        F7 = 27,
        F8 = 28,
        F9 = 29,
        F10 = 30,
        F11 = 31,
        F12 = 32
    ]
);
impl SessionMessage {
    pub fn encode(self) -> io::Result<[u8; 32]> {
        let fields = match self {
            Self::Hello { version } => Fields::new(10, u32::from(version), 0, 0, 0),
            Self::FrameReady { index, frame } => Fields::new(1, index, frame, 0, 0),
            Self::FrameDamage(rect) => Fields::new(9, 0, rect.packed(), 0, 0),
            Self::Key { key, phase } => Fields::new(8, system_code(key), 0, phase_code(phase), 0),
            Self::WatchKey { key, enabled } => {
                Fields::new(16, watch_code(key), 0, i32::from(enabled), 0)
            }
            Self::Backlight { limit } => Fields::new(17, u32::from(limit), 0, 0, 0),
        };
        let bytes = fields.encode();
        Self::decode(&bytes)?;
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        let f = Fields::decode(bytes)?;
        if f.y != 0 {
            return Err(invalid("nonzero reserved y"));
        }
        match f.kind {
            10 if f.payload == 0 && f.x == 0 => Ok(Self::Hello {
                version: u16::try_from(f.value).map_err(|_| invalid("invalid hello version"))?,
            }),
            1 if f.value < 3 && f.x == 0 => Ok(Self::FrameReady {
                index: f.value,
                frame: f.payload,
            }),
            9 if f.value == 0 && f.x == 0 => Ok(Self::FrameDamage(DamageRect::decode(f.payload)?)),
            8 if f.payload == 0 => Ok(Self::Key {
                key: system_key(f.value)?,
                phase: phase(f.x)?,
            }),
            16 if f.payload == 0 && (0..=1).contains(&f.x) => Ok(Self::WatchKey {
                key: watch_key(f.value)?,
                enabled: f.x == 1,
            }),
            17 if f.payload == 0 && f.x == 0 => Ok(Self::Backlight {
                limit: u16::try_from(f.value).map_err(|_| invalid("invalid backlight limit"))?,
            }),
            _ => Err(invalid("invalid session kind or fields")),
        }
    }
}
impl DaemonMessage {
    pub fn encode(self) -> io::Result<[u8; 32]> {
        let fields = match self {
            Self::Welcome {
                caps,
                width,
                height,
                width_mm100,
                height_mm100,
            } => Fields::new(
                11,
                caps,
                u64::from(width_mm100) | u64::from(height_mm100) << 32,
                i32::try_from(width).map_err(|_| invalid("invalid width"))?,
                i32::try_from(height).map_err(|_| invalid("invalid height"))?,
            ),
            Self::Revoked(reason) => Fields::new(
                12,
                match reason {
                    RevokedReason::NotActive => 1,
                    RevokedReason::SessionInactive => 2,
                    RevokedReason::Replaced => 3,
                    RevokedReason::Version => 4,
                    RevokedReason::Shutdown => 5,
                },
                0,
                0,
                0,
            ),
            Self::ModeChanged { mode, reason } => Fields::new(
                13,
                u32::from(mode == Mode::Display),
                match reason {
                    ModeReason::Owner => 0,
                    ModeReason::Recovery => 1,
                    ModeReason::Locked => 2,
                    ModeReason::Sleep => 3,
                    ModeReason::Error => 4,
                },
                0,
                0,
            ),
            Self::Touch {
                phase,
                contact,
                x,
                y,
            } => Fields::new(
                14,
                match phase {
                    TouchPhase::Down => 0,
                    TouchPhase::Move => 1,
                    TouchPhase::Up => 2,
                    TouchPhase::Cancel => 3,
                },
                u64::from(contact.get()),
                x,
                y,
            ),
            Self::KeyEvent { key, phase } => {
                Fields::new(15, watch_code(key), 0, phase_code(phase), 0)
            }
            Self::BufferReleased { index, frame } => Fields::new(2, index, frame, 0, 0),
            Self::FramePresented {
                index,
                frame,
                commit_us,
            } => Fields::new(
                18,
                index,
                frame,
                i32::try_from(commit_us)
                    .map_err(|_| invalid("commit duration exceeds wire limit"))?,
                0,
            ),
        };
        let bytes = fields.encode();
        Self::decode(&bytes)?;
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        let f = Fields::decode(bytes)?;
        match f.kind {
            11 if (1..=16384).contains(&f.x) && (1..=16384).contains(&f.y) => {
                let b = f.payload.to_le_bytes();
                let width_mm100 = u32::from_le_bytes(b[..4].try_into().expect("four bytes"));
                let height_mm100 = u32::from_le_bytes(b[4..].try_into().expect("four bytes"));
                if width_mm100 == 0 || height_mm100 == 0 {
                    return Err(invalid("zero physical dimensions"));
                }
                Ok(Self::Welcome {
                    caps: f.value,
                    width: u32::try_from(f.x).expect("positive"),
                    height: u32::try_from(f.y).expect("positive"),
                    width_mm100,
                    height_mm100,
                })
            }
            12 if f.payload == 0 && f.x == 0 && f.y == 0 => Ok(Self::Revoked(match f.value {
                1 => RevokedReason::NotActive,
                2 => RevokedReason::SessionInactive,
                3 => RevokedReason::Replaced,
                4 => RevokedReason::Version,
                5 => RevokedReason::Shutdown,
                _ => return Err(invalid("unknown revoke reason")),
            })),
            13 if f.value <= 1 && f.x == 0 && f.y == 0 => Ok(Self::ModeChanged {
                mode: if f.value == 1 {
                    Mode::Display
                } else {
                    Mode::Native
                },
                reason: match f.payload {
                    0 => ModeReason::Owner,
                    1 => ModeReason::Recovery,
                    2 => ModeReason::Locked,
                    3 => ModeReason::Sleep,
                    4 => ModeReason::Error,
                    _ => return Err(invalid("unknown mode reason")),
                },
            }),
            14 => Ok(Self::Touch {
                phase: match f.value {
                    0 => TouchPhase::Down,
                    1 => TouchPhase::Move,
                    2 => TouchPhase::Up,
                    3 => TouchPhase::Cancel,
                    _ => return Err(invalid("invalid touch phase")),
                },
                contact: NonZeroU32::new(
                    u32::try_from(f.payload).map_err(|_| invalid("contact overflow"))?,
                )
                .ok_or_else(|| invalid("zero contact"))?,
                x: f.x,
                y: f.y,
            }),
            15 if f.payload == 0 && f.y == 0 => Ok(Self::KeyEvent {
                key: watch_key(f.value)?,
                phase: phase(f.x)?,
            }),
            2 if f.value < 3 && f.x == 0 && f.y == 0 => Ok(Self::BufferReleased {
                index: f.value,
                frame: f.payload,
            }),
            18 if f.value < 3 && f.x >= 0 && f.y == 0 => Ok(Self::FramePresented {
                index: f.value,
                frame: f.payload,
                commit_us: u32::try_from(f.x).expect("nonnegative"),
            }),
            _ => Err(invalid("invalid daemon kind or fields")),
        }
    }
}
