pub mod model;
mod native;
pub use native::{NativeAudio, NativeOptions};
// Channel boundary for the native PipeWire worker.
use std::{future::Future, pin::Pin};
use touchbar_model::*;
pub type AudioFuture = Pin<Box<dyn Future<Output = EffectOutcome> + Send + 'static>>;
#[derive(Clone, Debug)]
pub enum AudioCommand {
    SetVolume {
        target: AudioTarget,
        level: f64,
    },
    SetMute {
        target: AudioTarget,
        value: MuteValue,
    },
}
pub trait AudioBackend: Send + Sync {
    fn command(&self, command: AudioCommand) -> AudioFuture;
    fn watch(&self, target: AudioTarget) -> Result<AudioWatch, EffectError>;
}
pub struct AudioWatch {
    pub events: async_channel::Receiver<Result<AudioState, EffectError>>,
    cancel: Option<Box<dyn FnOnce() + Send>>,
}
impl AudioWatch {
    pub fn new(
        events: async_channel::Receiver<Result<AudioState, EffectError>>,
        cancel: impl FnOnce() + Send + 'static,
    ) -> Self {
        Self {
            events,
            cancel: Some(Box::new(cancel)),
        }
    }
}
impl Drop for AudioWatch {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }
}
pub struct UnavailableAudio;
impl AudioBackend for UnavailableAudio {
    fn command(&self, _: AudioCommand) -> AudioFuture {
        Box::pin(std::future::ready(Err(EffectError::Unavailable)))
    }
    fn watch(&self, _: AudioTarget) -> Result<AudioWatch, EffectError> {
        let (tx, rx) = async_channel::bounded(1);
        tx.try_send(Ok(AudioState {
            available: false,
            volume: 0.0,
            muted: false,
            name: String::new(),
        }))
        .map_err(|_| EffectError::Unavailable)?;
        Ok(AudioWatch::new(rx, move || drop(tx)))
    }
}
