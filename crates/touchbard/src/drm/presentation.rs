use std::io;
use touchbar_protocol::damage::Damage;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub index: u32,
    pub frame: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Presented {
    pub presented: Frame,
    pub released: Option<Frame>,
}
/// One pending commit, plus a separately held scanout. Reset by replacing the entire surface.
pub struct Presentation {
    count: u32,
    current: Option<Frame>,
    pending: Option<Frame>,
    last: u64,
    full: bool,
}
impl Presentation {
    pub fn new(count: u32) -> io::Result<Self> {
        if !(2..=3).contains(&count) {
            return Err(io::Error::other("invalid swapchain count"));
        }
        Ok(Self {
            count,
            current: None,
            pending: None,
            last: 0,
            full: true,
        })
    }
    pub fn begin(&mut self, frame: Frame, damage: Damage) -> io::Result<Damage> {
        if self.pending.is_some()
            || frame.index >= self.count
            || frame.frame <= self.last
            || self.current.is_some_and(|f| f.index == frame.index)
        {
            return Err(io::Error::other("busy buffer or invalid frame order"));
        }
        self.pending = Some(frame);
        self.last = frame.frame;
        Ok(if self.full { Damage::Full } else { damage })
    }
    pub fn complete(&mut self) -> io::Result<Presented> {
        let presented = self
            .pending
            .take()
            .ok_or_else(|| io::Error::other("no pending commit"))?;
        let released = self.current.replace(presented);
        self.full = false;
        Ok(Presented {
            presented,
            released,
        })
    }
    pub fn failed(&mut self) {
        self.pending = None;
        self.full = true;
    }
}
