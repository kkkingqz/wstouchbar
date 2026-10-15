//! Asynchronous effect execution and shared subscription sources.
pub mod spawn;

pub mod dbus;

mod engine;
mod fake;
pub use engine::{Config, Engine};
pub use fake::{FakeControl, FakeRule};
