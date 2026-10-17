use std::{collections::BTreeMap, io};
use touchbar_model::TouchPhase;
pub const MAX_SLOTS: usize = 32;
const MAX_EDGES: usize = MAX_SLOTS * 2;
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputEvent {
    pub kind: u16,
    pub code: u16,
    pub value: i32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Axis {
    pub minimum: i32,
    pub maximum: i32,
}
impl Axis {
    fn coordinate(self, value: i32, extent: u32) -> i32 {
        let offset = i64::from(value.clamp(self.minimum, self.maximum)) - i64::from(self.minimum);
        let range = i64::from(self.maximum) - i64::from(self.minimum);
        i32::try_from((offset * i64::from(extent - 1) + range / 2) / range)
            .expect("validated extent")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisTransform {
    Identity,
    Swap,
}
#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    width: u32,
    height: u32,
    x: Axis,
    y: Axis,
    transform: AxisTransform,
}
impl Geometry {
    pub fn new(
        width: u32,
        height: u32,
        x: Axis,
        y: Axis,
        transform: AxisTransform,
    ) -> io::Result<Self> {
        if width == 0
            || height == 0
            || width > 16384
            || height > 16384
            || x.maximum <= x.minimum
            || y.maximum <= y.minimum
        {
            return Err(invalid("invalid touch geometry"));
        }
        Ok(Self {
            width,
            height,
            x,
            y,
            transform,
        })
    }
    pub fn point(self, x: i32, y: i32) -> (i32, i32) {
        match self.transform {
            AxisTransform::Identity => (
                self.x.coordinate(x, self.width),
                self.y.coordinate(y, self.height),
            ),
            AxisTransform::Swap => (
                self.y.coordinate(y, self.width),
                self.x.coordinate(x, self.height),
            ),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawSlot {
    pub tracking: Option<u32>,
    pub x: i32,
    pub y: i32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub tracking: u32,
    pub phase: TouchPhase,
    pub x: i32,
    pub y: i32,
}
#[derive(Debug)]
pub enum Step {
    Events(Vec<Sample>),
    Resync,
}
type Contacts = BTreeMap<u32, (i32, i32)>;
fn sample(tracking: u32, phase: TouchPhase, point: (i32, i32)) -> Sample {
    Sample {
        tracking,
        phase,
        x: point.0,
        y: point.1,
    }
}
fn reconcile(old: &Contacts, new: &Contacts, missing: TouchPhase) -> Vec<Sample> {
    let mut out = Vec::new();
    for (&id, &point) in old {
        if !new.contains_key(&id) {
            out.push(sample(id, missing, point));
        }
    }
    for (&id, &point) in new {
        match old.get(&id) {
            None => out.push(sample(id, TouchPhase::Down, point)),
            Some(previous) if *previous != point => out.push(sample(id, TouchPhase::Move, point)),
            _ => {}
        }
    }
    out
}
pub struct TouchState {
    geometry: Geometry,
    slots: Vec<RawSlot>,
    current: usize,
    reported: Contacts,
    departures: Vec<RawSlot>,
    dropped: bool,
}
impl TouchState {
    pub fn new(geometry: Geometry, count: usize, current: usize) -> io::Result<Self> {
        if count == 0 || count > MAX_SLOTS || current >= count {
            return Err(invalid("invalid multitouch slots"));
        }
        let empty = RawSlot {
            tracking: None,
            x: geometry.x.minimum,
            y: geometry.y.minimum,
        };
        Ok(Self {
            geometry,
            slots: vec![empty; count],
            current,
            reported: Contacts::new(),
            departures: Vec::new(),
            dropped: false,
        })
    }
    fn contacts(&self, slots: &[RawSlot]) -> io::Result<Contacts> {
        let mut contacts = Contacts::new();
        for slot in slots {
            if let Some(id) = slot.tracking
                && (id > i32::MAX.unsigned_abs()
                    || contacts
                        .insert(id, self.geometry.point(slot.x, slot.y))
                        .is_some())
            {
                return Err(invalid("duplicate or invalid multitouch tracking ID"));
            }
        }
        Ok(contacts)
    }
    pub fn handle(&mut self, event: InputEvent) -> io::Result<Step> {
        if self.dropped {
            return Ok(if (event.kind, event.code) == (0, 0) {
                Step::Resync
            } else {
                Step::Events(Vec::new())
            });
        }
        match (event.kind, event.code) {
            (0, 3) => {
                self.dropped = true;
                self.departures.clear();
            }
            (0, 0) => {
                // Validate the entire frame before changing reported contacts. On any error,
                // the device wrapper can still cancel exactly those contacts already delivered.
                let new = self.contacts(&self.slots)?;
                let mut reported = self.reported.clone();
                let mut out = Vec::new();
                for slot in self.departures.drain(..) {
                    let id = slot.tracking.expect("departures always active");
                    let point = self.geometry.point(slot.x, slot.y);
                    if !reported.contains_key(&id) {
                        out.push(sample(id, TouchPhase::Down, point));
                    }
                    out.push(sample(id, TouchPhase::Up, point));
                    reported.remove(&id);
                }
                out.extend(reconcile(&reported, &new, TouchPhase::Up));
                self.reported = new;
                return Ok(Step::Events(out));
            }
            (3, 0x2f) => {
                let current = usize::try_from(event.value)
                    .map_err(|_| invalid("negative multitouch slot"))?;
                if current >= self.slots.len() {
                    return Err(invalid("multitouch slot outside range"));
                }
                self.current = current;
            }
            (3, 0x39) => {
                let tracking = match event.value {
                    -1 => None,
                    n if n >= 0 => Some(n.unsigned_abs()),
                    _ => return Err(invalid("invalid tracking ID")),
                };
                let slot = &mut self.slots[self.current];
                if slot.tracking != tracking {
                    if slot.tracking.is_some() {
                        if self.departures.len() >= MAX_EDGES {
                            return Err(invalid("too many touch edges without SYN_REPORT"));
                        }
                        self.departures.push(*slot);
                    }
                    slot.tracking = tracking;
                }
            }
            (3, 0x35) => self.slots[self.current].x = event.value,
            (3, 0x36) => self.slots[self.current].y = event.value,
            _ => {}
        }
        Ok(Step::Events(Vec::new()))
    }
    /// EVIOCGMTSLOTS is authoritative after SYN_DROPPED. Match globally by tracking ID,
    /// including when a surviving finger moved between slots during the lost interval.
    pub fn restore(&mut self, slots: &[RawSlot], current: usize) -> io::Result<Vec<Sample>> {
        if slots.len() != self.slots.len() || current >= slots.len() {
            return Err(invalid("invalid multitouch snapshot size"));
        }
        let new = self.contacts(slots)?;
        let out = reconcile(&self.reported, &new, TouchPhase::Cancel);
        self.slots.clone_from_slice(slots);
        self.current = current;
        self.reported = new;
        self.departures.clear();
        self.dropped = false;
        Ok(out)
    }
    pub fn cancel_all(&mut self) -> Vec<Sample> {
        let out = self
            .reported
            .iter()
            .map(|(&id, &point)| sample(id, TouchPhase::Cancel, point))
            .collect();
        self.reported.clear();
        self.departures.clear();
        self.dropped = false;
        for slot in &mut self.slots {
            slot.tracking = None;
        }
        out
    }
}
