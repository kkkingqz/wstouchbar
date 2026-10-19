//! Surface adapter; GL belongs to the Session's shared renderer.
use super::Handle;
use crate::surface::{Presentation, PresentedFrame, Surface};
use std::sync::Arc;
use touchbar_model::*;
pub struct TouchbarSurface {
    handle: Handle,
    info: SurfaceInfo,
    index: Option<u32>,
}
impl TouchbarSurface {
    pub fn new(handle: Handle) -> Result<Self, String> {
        let info = handle.info().ok_or("Touch Bar has no Welcome")?;
        Ok(Self {
            handle,
            info,
            index: None,
        })
    }
}
impl Surface for TouchbarSurface {
    fn info(&self) -> SurfaceInfo {
        self.handle.info().unwrap_or_else(|| self.info.clone())
    }
    fn uses_shared_target(&self) -> bool {
        true
    }
    fn prepare_shared_target(&mut self) -> Result<Option<super::TargetSlot>, String> {
        let slot = self.handle.prepare_frame().map_err(|e| e.to_string())?;
        self.index = slot.as_ref().map(|s| s.index);
        Ok(slot)
    }
    fn target_imported(&mut self, path: touchbar_gl::TargetPath) {
        self.handle.target_path(path);
    }
    fn present(&mut self, frame: Arc<PresentedFrame>) -> Result<Presentation, String> {
        let Some(index) = self.index.take() else {
            return Ok(Presentation::Pending);
        };
        let rendered = frame
            .target
            .as_ref()
            .ok_or("shared target was not rendered")?;
        let damage = rendered
            .damage
            .iter()
            .map(|r| {
                Ok(touchbar_protocol::DamageRect {
                    x: u16::try_from(r.x).map_err(|e| e.to_string())?,
                    y: u16::try_from(r.y).map_err(|e| e.to_string())?,
                    width: u16::try_from(r.width).map_err(|e| e.to_string())?,
                    height: u16::try_from(r.height).map_err(|e| e.to_string())?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.handle
            .submit(frame.generation, index, &damage)
            .map_err(|e| e.to_string())?;
        Ok(Presentation::Pending)
    }
    fn frame_unchanged(&mut self) -> Result<(), String> {
        let index = self
            .index
            .take()
            .ok_or("unchanged frame was not prepared")?;
        self.handle
            .finish_unchanged(index)
            .map_err(|e| e.to_string())
    }
    fn watch_keys(&mut self, keys: &[WatchKey]) -> Result<(), String> {
        self.handle.watch_keys(keys).map_err(|e| e.to_string())
    }
    fn send_key(&mut self, key: SystemKey, phase: KeyPhase) -> Result<(), String> {
        self.handle.send_key(key, phase).map_err(|e| e.to_string())
    }
    fn backlight(&mut self, limit: f32) -> Result<(), String> {
        self.handle.backlight(limit).map_err(|e| e.to_string())
    }
    fn close(&mut self) {
        self.handle.close()
    }
}
