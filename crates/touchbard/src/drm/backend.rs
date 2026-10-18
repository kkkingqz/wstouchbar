//! The sole owner of DRM and blocking USB operations. Callers close their event descriptor
//! before Switch/Release/Open, and tag requests with the mode machine's generation.
use super::{CommitMode, Completion, Exports, Frame, Submission, Surface, events::PageFlip};
use crate::{
    device::{SwitchPlan, SystemOps},
    worker::Worker,
};
use std::{io, path::PathBuf};
use touchbar_protocol::DamageRect;
#[derive(Clone)]
pub enum Operation {
    Switch(SwitchPlan),
    CheckMaster(PathBuf),
    Open {
        card: PathBuf,
        dmi_model: Option<String>,
    },
    Present {
        frame: Frame,
        damage: Vec<DamageRect>,
        mode: CommitMode,
    },
    PageFlip(PageFlip),
    PaintProbe {
        index: u32,
        patch: Option<DamageRect>,
    },
    Release,
}
pub enum Reply {
    Done,
    Opened(Exports),
    Submitted(Submission),
    Completed(Option<Completion>),
}
pub type DeviceWorker = Worker<Operation, io::Result<Reply>>;
// Declaration order is significant: release DRM before dropping the exclusive-device lock.
struct Backend<G> {
    surface: Option<Surface>,
    _guard: G,
}
impl<G> Backend<G> {
    fn execute(&mut self, operation: Operation) -> io::Result<Reply> {
        match operation {
            Operation::Switch(plan) => {
                drop(self.surface.take());
                plan.execute(&mut SystemOps { release: || Ok(()) })?;
                Ok(Reply::Done)
            }
            Operation::CheckMaster(card) => {
                Surface::check_master(&card)?;
                Ok(Reply::Done)
            }
            Operation::Open { card, dmi_model } => {
                // Fresh DRM file and allocations prevent old client writes or page-flip events
                // from becoming part of a new connection's swapchain.
                drop(self.surface.take());
                let opened = Surface::open(&card, dmi_model.as_deref())?;
                let exports = opened.exports()?;
                self.surface = Some(opened);
                Ok(Reply::Opened(exports))
            }
            Operation::Release => {
                drop(self.surface.take());
                Ok(Reply::Done)
            }
            operation => {
                let s = self.surface.as_mut().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotConnected, "DRM output is not open")
                })?;
                match operation {
                    Operation::Present {
                        frame,
                        damage,
                        mode,
                    } => Ok(Reply::Submitted(s.submit(frame, &damage, mode)?)),
                    Operation::PageFlip(event) => Ok(Reply::Completed(s.page_flip(event)?)),
                    Operation::PaintProbe { index, patch } => {
                        s.paint_probe(index, patch)?;
                        Ok(Reply::Done)
                    }
                    _ => unreachable!("non-surface operations handled above"),
                }
            }
        }
    }
}
/// `guard` (normally ExclusiveDevice) outlives DRM cleanup even if its caller times out.
pub fn device_worker(guard: impl Send + 'static) -> io::Result<DeviceWorker> {
    let mut backend = Backend {
        surface: None,
        _guard: guard,
    };
    Worker::new(move |operation| backend.execute(operation))
}

/// Embed DRM in the daemon's single worker alongside persistence/sysfs operations.
pub(crate) fn executor(
    guard: impl Send + 'static,
) -> impl FnMut(Operation) -> io::Result<Reply> + Send {
    let mut backend = Backend {
        surface: None,
        _guard: guard,
    };
    move |operation| backend.execute(operation)
}
