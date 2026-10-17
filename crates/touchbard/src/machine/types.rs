use super::mode;
use crate::config::Config;
use serde::Serialize;
use touchbar_bindings::KeyInput;
use touchbar_model::{KeyPhase, SystemKey, TouchPhase};
use touchbar_protocol::{DaemonMessage, DamageRect, Mode, SessionMessage, SwapchainHeader};
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Session {
    pub uid: u32,
    pub id: String,
    pub user_display: Option<String>,
    pub active: bool,
    pub state: String,
    pub class: String,
    pub kind: String,
    pub locked: bool,
}
impl Session {
    pub fn eligible(&self, uid: u32) -> bool {
        self.uid == uid
            && self.active
            && self.state == "active"
            && self.class == "user"
            && matches!(self.kind.as_str(), "wayland" | "x11")
            && self.user_display.as_deref() == Some(self.id.as_str())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayInfo {
    pub header: SwapchainHeader,
    pub caps: u32,
    pub width_mm100: u32,
    pub height_mm100: u32,
}
impl DisplayInfo {
    pub fn valid(self) -> bool {
        self.header.validate().is_ok() && self.width_mm100 > 0 && self.height_mm100 > 0
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Start {
        preparing_sleep: bool,
    },
    Session(Option<Session>),
    Hello {
        connection: u64,
        uid: u32,
        version: u16,
    },
    Disconnected {
        connection: u64,
    },
    ProtocolError {
        connection: u64,
    },
    Sent {
        connection: u64,
    },
    Message {
        connection: u64,
        message: SessionMessage,
    },
    Device(Option<u64>),
    DisplayReady {
        generation: u64,
        info: DisplayInfo,
    },
    NativeSafe {
        generation: u64,
    },
    TransitionFailed {
        generation: u64,
        message: String,
    },
    Driver {
        device: u64,
        mode: Option<u8>,
    },
    SetMode(Mode),
    PrepareForSleep(bool),
    InhibitorFailed {
        generation: u64,
        message: String,
    },
    InhibitorAcquired {
        generation: u64,
    },
    Shutdown,
    PhysicalKey(KeyInput),
    VirtualActivity,
    Activity,
    Lid(bool),
    DriverTimeouts {
        dim_seconds: Option<u64>,
        idle_seconds: Option<u64>,
    },
    Autodim(bool),
    Touch {
        tracking: u32,
        phase: TouchPhase,
        x: i32,
        y: i32,
    },
    FramePresented {
        generation: u64,
        connection: u64,
        index: u32,
        frame: u64,
        commit_us: u32,
    },
    FrameReleased {
        generation: u64,
        connection: u64,
        index: u32,
        frame: u64,
    },
    Config(Config),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outgoing {
    Message(DaemonMessage),
    Swapchain {
        generation: u64,
        header: SwapchainHeader,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Device(mode::Command),
    Flush {
        connection: u64,
    },
    Close {
        connection: u64,
    },
    Uinput {
        key: SystemKey,
        phase: KeyPhase,
    },
    Present {
        generation: u64,
        connection: u64,
        index: u32,
        frame: u64,
        damage: Vec<DamageRect>,
    },
    AcquireInhibitor {
        generation: u64,
    },
    ReleaseInhibitor {
        generation: u64,
    },
    WriteRecovery(bool),
    WriteDriverMode(u8),
    SetBacklight(u8),
    OwnAutodim,
    RestoreAutodim,
    Exit,
}
