//! Surface transport boundary; CPU-only headless surfaces never initialize GL.
use std::{cell::RefCell, rc::Rc, sync::Arc};
use touchbar_model::*;
#[derive(Clone, Debug)]
pub struct PresentedFrame {
    pub generation: Generation,
    pub display_list: touchbar_ui::DisplayList,
    pub layout: touchbar_scene::FrameLayout,
    pub pixels: Option<touchbar_gl::RenderedFrame>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presentation {
    Complete,
    Pending,
}
pub trait Surface {
    fn info(&self) -> SurfaceInfo;
    fn needs_pixels(&self) -> bool {
        false
    }
    fn present(&mut self, frame: Arc<PresentedFrame>) -> Result<Presentation, String>;
    fn send_key(&mut self, _key: SystemKey, _phase: KeyPhase) -> Result<(), String> {
        Err("surface key injection unavailable".into())
    }
    fn watch_keys(&mut self, _keys: &[WatchKey]) -> Result<(), String> {
        Ok(())
    }
    fn backlight(&mut self, _limit: f32) -> Result<(), String> {
        Ok(())
    }
    fn close(&mut self) {}
}
#[derive(Clone, Debug)]
pub enum SurfaceEvent {
    Input(touchbar_core::Input),
    Presented {
        surface: SurfaceId,
        generation: Generation,
    },
    Closed(SurfaceId),
}
#[derive(Default, Debug)]
pub struct FramePacer {
    busy: bool,
    last: Option<Timestamp>,
    next: Option<Timestamp>,
}
impl FramePacer {
    pub fn begin(&mut self, now: Timestamp, animation: bool) -> bool {
        if self.busy || animation && self.next.is_none_or(|at| now < at) {
            return false;
        }
        self.busy = true;
        self.last = Some(now);
        self.next = None;
        true
    }
    pub fn finish(&mut self, animate: bool) {
        self.busy = false;
        self.next = if animate {
            self.last.map(|at| at.after(17))
        } else {
            None
        };
    }
    pub fn deadline(&self) -> Option<Timestamp> {
        self.next
    }
    pub fn stop_animation(&mut self) {
        self.next = None;
    }
    pub fn cancel(&mut self) {
        self.busy = false;
        self.next = None;
    }
    pub fn busy(&self) -> bool {
        self.busy
    }
}
#[derive(Clone, Default)]
pub struct HeadlessHandle(Rc<RefCell<HeadlessState>>);
#[derive(Default)]
struct HeadlessState {
    last: Option<Arc<PresentedFrame>>,
    frames: u64,
    keys: Vec<(SystemKey, KeyPhase)>,
}
impl HeadlessHandle {
    pub fn frame(&self) -> Option<Arc<PresentedFrame>> {
        self.0.borrow().last.clone()
    }
    pub fn frames(&self) -> u64 {
        self.0.borrow().frames
    }
    pub fn keys(&self) -> Vec<(SystemKey, KeyPhase)> {
        self.0.borrow().keys.clone()
    }
}
pub struct HeadlessSurface {
    info: SurfaceInfo,
    handle: HeadlessHandle,
}
impl HeadlessSurface {
    pub fn new(info: SurfaceInfo) -> (Self, HeadlessHandle) {
        let handle = HeadlessHandle::default();
        (
            Self {
                info,
                handle: handle.clone(),
            },
            handle,
        )
    }
}
impl Surface for HeadlessSurface {
    fn info(&self) -> SurfaceInfo {
        self.info.clone()
    }
    fn present(&mut self, frame: Arc<PresentedFrame>) -> Result<Presentation, String> {
        let mut state = self.handle.0.borrow_mut();
        state.last = Some(frame);
        state.frames = state.frames.saturating_add(1);
        Ok(Presentation::Complete)
    }
    fn send_key(&mut self, key: SystemKey, phase: KeyPhase) -> Result<(), String> {
        let mut state = self.handle.0.borrow_mut();
        if state.keys.len() >= 1024 {
            state.keys.remove(0);
        }
        state.keys.push((key, phase));
        Ok(())
    }
}
