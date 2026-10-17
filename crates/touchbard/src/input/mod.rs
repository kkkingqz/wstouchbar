//! Bounded evdev protocol-B decoding, separate from the Linux device boundary.
mod touch;
pub use touch::{
    Axis, AxisTransform, Geometry, InputEvent, MAX_SLOTS, RawSlot, Sample, Step, TouchState,
};
mod reader;
pub use reader::{Batch, EVENT_SIZE, SnapshotSource, TouchReader, decode_events};
mod linux;
pub use linux::{LinuxQueries, TouchInput};
