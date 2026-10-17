//! Closed semantic key maps and authoritative physical-device state.
use std::{collections::BTreeSet, io};
use touchbar_bindings::KeyInput;
use touchbar_model::{KeyPhase, SystemKey, WatchKey};
pub const SYSTEM_KEYS: &[(SystemKey, u16)] = &[
    (SystemKey::Esc, 1),
    (SystemKey::F1, 59),
    (SystemKey::F2, 60),
    (SystemKey::F3, 61),
    (SystemKey::F4, 62),
    (SystemKey::F5, 63),
    (SystemKey::F6, 64),
    (SystemKey::F7, 65),
    (SystemKey::F8, 66),
    (SystemKey::F9, 67),
    (SystemKey::F10, 68),
    (SystemKey::F11, 87),
    (SystemKey::F12, 88),
    (SystemKey::BrightnessDown, 224),
    (SystemKey::BrightnessUp, 225),
    (SystemKey::KeyboardIlluminationDown, 229),
    (SystemKey::KeyboardIlluminationUp, 230),
    (SystemKey::PreviousSong, 165),
    (SystemKey::PlayPause, 164),
    (SystemKey::NextSong, 163),
    (SystemKey::Mute, 113),
    (SystemKey::VolumeDown, 114),
    (SystemKey::VolumeUp, 115),
    (SystemKey::MicrophoneMute, 248),
    (SystemKey::Search, 217),
];
pub fn system_code(key: SystemKey) -> u16 {
    match key {
        SystemKey::Esc => 1,
        SystemKey::F1 => 59,
        SystemKey::F2 => 60,
        SystemKey::F3 => 61,
        SystemKey::F4 => 62,
        SystemKey::F5 => 63,
        SystemKey::F6 => 64,
        SystemKey::F7 => 65,
        SystemKey::F8 => 66,
        SystemKey::F9 => 67,
        SystemKey::F10 => 68,
        SystemKey::F11 => 87,
        SystemKey::F12 => 88,
        SystemKey::BrightnessDown => 224,
        SystemKey::BrightnessUp => 225,
        SystemKey::KeyboardIlluminationDown => 229,
        SystemKey::KeyboardIlluminationUp => 230,
        SystemKey::PreviousSong => 165,
        SystemKey::PlayPause => 164,
        SystemKey::NextSong => 163,
        SystemKey::Mute => 113,
        SystemKey::VolumeDown => 114,
        SystemKey::VolumeUp => 115,
        SystemKey::MicrophoneMute => 248,
        SystemKey::Search => 217,
    }
}
pub fn system_key(code: u16) -> Option<SystemKey> {
    SYSTEM_KEYS
        .iter()
        .find(|(_, c)| *c == code)
        .map(|(k, _)| *k)
}
pub const WATCH_KEYS: &[(WatchKey, u16)] = &[
    (WatchKey::Fn, 464),
    (WatchKey::Esc, 1),
    (WatchKey::LeftShift, 42),
    (WatchKey::RightShift, 54),
    (WatchKey::LeftControl, 29),
    (WatchKey::RightControl, 97),
    (WatchKey::LeftAlt, 56),
    (WatchKey::RightAlt, 100),
    (WatchKey::LeftMeta, 125),
    (WatchKey::RightMeta, 126),
    (WatchKey::F1, 59),
    (WatchKey::F2, 60),
    (WatchKey::F3, 61),
    (WatchKey::F4, 62),
    (WatchKey::F5, 63),
    (WatchKey::F6, 64),
    (WatchKey::F7, 65),
    (WatchKey::F8, 66),
    (WatchKey::F9, 67),
    (WatchKey::F10, 68),
    (WatchKey::F11, 87),
    (WatchKey::F12, 88),
];
pub fn watch_code(key: WatchKey) -> u16 {
    match key {
        WatchKey::Fn => 464,
        WatchKey::Esc => 1,
        WatchKey::LeftShift => 42,
        WatchKey::RightShift => 54,
        WatchKey::LeftControl => 29,
        WatchKey::RightControl => 97,
        WatchKey::LeftAlt => 56,
        WatchKey::RightAlt => 100,
        WatchKey::LeftMeta => 125,
        WatchKey::RightMeta => 126,
        WatchKey::F1 => 59,
        WatchKey::F2 => 60,
        WatchKey::F3 => 61,
        WatchKey::F4 => 62,
        WatchKey::F5 => 63,
        WatchKey::F6 => 64,
        WatchKey::F7 => 65,
        WatchKey::F8 => 66,
        WatchKey::F9 => 67,
        WatchKey::F10 => 68,
        WatchKey::F11 => 87,
        WatchKey::F12 => 88,
    }
}
pub fn watch_key(code: u16) -> Option<WatchKey> {
    WATCH_KEYS.iter().find(|(_, c)| *c == code).map(|(k, _)| *k)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBitmap(pub(crate) [u8; 96]);
impl Default for KeyBitmap {
    fn default() -> Self {
        Self([0; 96])
    }
}
impl KeyBitmap {
    pub fn set(&mut self, code: u16, pressed: bool) -> io::Result<()> {
        let i = usize::from(code);
        let byte = self.0.get_mut(i / 8).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "key code outside bitmap")
        })?;
        let mask = 1 << (i % 8);
        if pressed {
            *byte |= mask;
        } else {
            *byte &= !mask;
        }
        Ok(())
    }
    pub fn contains(&self, code: u16) -> bool {
        let i = usize::from(code);
        self.0.get(i / 8).is_some_and(|b| b & (1 << (i % 8)) != 0)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    Physical,
    Virtual,
    Own,
}
#[derive(Clone, Debug)]
pub struct DeviceSnapshot {
    pub kind: InputKind,
    pub keys: KeyBitmap,
    pub activity: bool,
    pub lid: Option<bool>,
}
#[derive(Default, Debug)]
pub struct Changes {
    pub keys: Vec<KeyInput>,
    pub activity: bool,
    pub lid: Option<bool>,
}
pub struct KeyState {
    wanted: BTreeSet<WatchKey>,
    reported: BTreeSet<WatchKey>,
    lid: Option<bool>,
}
impl KeyState {
    pub fn new(wanted: BTreeSet<WatchKey>) -> Self {
        let mut s = Self {
            wanted,
            reported: BTreeSet::new(),
            lid: None,
        };
        s.wanted.insert(WatchKey::Fn);
        s
    }
    pub fn set_wanted(&mut self, mut wanted: BTreeSet<WatchKey>) {
        wanted.insert(WatchKey::Fn);
        self.wanted = wanted;
    }
    pub fn refresh(&mut self, nodes: &[DeviceSnapshot]) -> Changes {
        let physical: Vec<_> = nodes
            .iter()
            .filter(|n| n.kind == InputKind::Physical)
            .collect();
        let pressed: BTreeSet<_> = self
            .wanted
            .iter()
            .copied()
            .filter(|k| physical.iter().any(|n| n.keys.contains(watch_code(*k))))
            .collect();
        let mut changes = Changes {
            activity: nodes.iter().any(|n| n.kind != InputKind::Own && n.activity),
            ..Changes::default()
        };
        for code in self.reported.difference(&pressed) {
            changes.keys.push(KeyInput {
                code: *code,
                phase: KeyPhase::Released,
            });
        }
        for code in pressed.difference(&self.reported) {
            changes.keys.push(KeyInput {
                code: *code,
                phase: KeyPhase::Pressed,
            });
        }
        self.reported = pressed;
        // Retain the last measured lid state if the sensor disappears. A missing sensor
        // is not evidence that a closed lid opened. Multiple physical sensors close if any closes.
        let sensors: Vec<_> = physical.iter().filter_map(|n| n.lid).collect();
        if !sensors.is_empty() {
            let lid = sensors.iter().any(|b| *b);
            if self.lid != Some(lid) {
                self.lid = Some(lid);
                changes.lid = Some(lid);
            }
        }
        changes
    }
}
mod reader;
pub use reader::{Batch, Capabilities, KeyQueries, KeyReader};
mod output;
pub use output::{SystemKeys, UINPUT_NAME};
mod linux;
mod mode;
pub use linux::{LinuxKeyQueries, Uinput, classify_input};
pub use mode::ModeWriter;
mod collection;
pub use collection::{InputBatch, InputFault, KeyInputs};
