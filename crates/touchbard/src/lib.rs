//! Privileged Touch Bar daemon: pure state machines and Linux drivers.

pub mod config;

pub mod machine;

pub use machine::{Command, Daemon, DisplayInfo, Event, Outgoing, Session};

pub mod device;

pub mod worker;

pub mod drm;

pub mod probe;
