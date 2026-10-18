//! Atomic appletbdrm output. All potentially blocking operations belong to the device worker.
pub mod dimensions;
pub mod events;
mod presentation;
pub use presentation::{Frame, Presentation, Presented};
mod surface;
pub use surface::{CommitMode, Completion, Exports, Submission, Surface, monotonic_us};
mod backend;
pub use backend::{DeviceWorker, Operation, Reply, device_worker};

pub(crate) use backend::executor;
