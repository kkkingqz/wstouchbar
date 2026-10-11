use serde::{Deserialize, Serialize};

/// Closed semantic keys; mapping to evdev belongs to touchbard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SystemKey {
    Esc,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    BrightnessDown,
    BrightnessUp,
    KeyboardIlluminationDown,
    KeyboardIlluminationUp,
    PreviousSong,
    PlayPause,
    NextSong,
    Mute,
    VolumeDown,
    VolumeUp,
    MicrophoneMute,
    Search,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WatchKey {
    Fn,
    Esc,
    LeftShift,
    RightShift,
    LeftControl,
    RightControl,
    LeftAlt,
    RightAlt,
    LeftMeta,
    RightMeta,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyPhase {
    Pressed,
    Released,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TouchPhase {
    Down,
    Move,
    Up,
    Cancel,
}
