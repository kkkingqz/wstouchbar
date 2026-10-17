use super::{Geometry, InputEvent, RawSlot, Sample, Step, TouchState};
use async_io::Async;
use std::{
    io,
    os::fd::{AsFd, BorrowedFd, OwnedFd},
};
/// Linux input_event: two native-word time fields followed by type, code, value.
/// Time fields are deliberately ignored; the shell timestamps delivered inputs.
pub const EVENT_SIZE: usize = 2 * std::mem::size_of::<usize>() + 8;
const BATCH_EVENTS: usize = 64;
pub fn decode_events(bytes: &[u8]) -> io::Result<Vec<InputEvent>> {
    if !bytes.len().is_multiple_of(EVENT_SIZE) || bytes.len() > EVENT_SIZE * BATCH_EVENTS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "partial or oversized evdev batch",
        ));
    }
    Ok(bytes
        .as_chunks::<EVENT_SIZE>()
        .0
        .iter()
        .map(|b| {
            let p = EVENT_SIZE - 8;
            InputEvent {
                kind: u16::from_ne_bytes(b[p..p + 2].try_into().expect("checked event")),
                code: u16::from_ne_bytes(b[p + 2..p + 4].try_into().expect("checked event")),
                value: i32::from_ne_bytes(b[p + 4..p + 8].try_into().expect("checked event")),
            }
        })
        .collect())
}
pub trait SnapshotSource {
    fn snapshot(&self, fd: BorrowedFd<'_>, count: usize) -> io::Result<(Vec<RawSlot>, usize)>;
}
#[derive(Debug)]
pub struct Batch {
    pub events: Vec<Sample>,
    pub fault: Option<io::Error>,
}
pub struct TouchReader<Q: SnapshotSource> {
    file: Async<OwnedFd>,
    source: Q,
    state: TouchState,
    count: usize,
    initial: Vec<Sample>,
    closed: bool,
}
impl<Q: SnapshotSource> TouchReader<Q> {
    /// Low-level reader; the production `TouchInput::open` validates and grabs the device.
    pub fn from_fd(fd: OwnedFd, geometry: Geometry, count: usize, source: Q) -> io::Result<Self> {
        let mut state = TouchState::new(geometry, count, 0)?;
        let (slots, current) = source.snapshot(fd.as_fd(), count)?;
        let initial = state.restore(&slots, current)?;
        Ok(Self {
            file: Async::new(fd)?,
            source,
            state,
            count,
            initial,
            closed: false,
        })
    }
    fn failure(&mut self, mut events: Vec<Sample>, error: io::Error) -> Batch {
        events.extend(self.state.cancel_all());
        self.initial.clear();
        self.closed = true;
        Batch {
            events,
            fault: Some(error),
        }
    }
    /// Exactly one awaited read. Once bytes are consumed, every resulting edge is returned
    /// without another await, so cancellation by the shell's select loop cannot lose events.
    pub async fn next(&mut self) -> Batch {
        if self.closed {
            return Batch {
                events: Vec::new(),
                fault: Some(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "touch input closed",
                )),
            };
        }
        if !self.initial.is_empty() {
            return Batch {
                events: std::mem::take(&mut self.initial),
                fault: None,
            };
        }
        let mut bytes = [0u8; EVENT_SIZE * BATCH_EVENTS];
        let n = match self
            .file
            .read_with(|fd| {
                loop {
                    match rustix::io::read(fd, &mut bytes) {
                        Err(rustix::io::Errno::INTR) => continue,
                        result => break result.map_err(io::Error::from),
                    }
                }
            })
            .await
        {
            Ok(0) => {
                return self.failure(
                    Vec::new(),
                    io::Error::new(io::ErrorKind::UnexpectedEof, "touch input disconnected"),
                );
            }
            Ok(n) => n,
            Err(e) => return self.failure(Vec::new(), e),
        };
        let input = match decode_events(&bytes[..n]) {
            Ok(events) => events,
            Err(e) => return self.failure(Vec::new(), e),
        };
        let mut events = Vec::new();
        for event in input {
            let step = match self.state.handle(event) {
                Ok(step) => step,
                Err(e) => return self.failure(events, e),
            };
            match step {
                Step::Events(out) => events.extend(out),
                Step::Resync => {
                    let result = self
                        .source
                        .snapshot(self.file.get_ref().as_fd(), self.count)
                        .and_then(|(slots, current)| self.state.restore(&slots, current));
                    match result {
                        Ok(out) => events.extend(out),
                        Err(e) => return self.failure(events, e),
                    }
                }
            }
        }
        Batch {
            events,
            fault: None,
        }
    }
}
