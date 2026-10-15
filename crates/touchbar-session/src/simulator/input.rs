use super::SimulatorEvent;
use crate::surface::SurfaceEvent;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use touchbar_core::{Input, KeySource};
use touchbar_model::*;
struct Route {
    info: SurfaceInfo,
    scale: u32,
    watched: BTreeSet<WatchKey>,
}
#[derive(Clone, Copy)]
struct Contact {
    surface: SurfaceId,
    id: ContactId,
    x: f32,
    y: f32,
}
#[derive(Default)]
pub struct InputRouter {
    routes: BTreeMap<SurfaceId, Route>,
    pointer: Option<(SurfaceId, f64, f64)>,
    mouse: Option<Contact>,
    keyboard: Option<SurfaceId>,
    raw: BTreeSet<u32>,
    forwarded: BTreeSet<(SurfaceId, WatchKey)>,
    touches: BTreeMap<i32, Contact>,
    events: VecDeque<SimulatorEvent>,
    overflow: bool,
}
impl InputRouter {
    pub fn add(&mut self, info: SurfaceInfo, scale: u32) {
        self.routes.insert(
            info.id,
            Route {
                info,
                scale: scale.clamp(1, 8),
                watched: BTreeSet::new(),
            },
        );
    }
    fn emit(&mut self, event: SimulatorEvent) {
        if self.overflow {
            return;
        }
        if self.events.len() >= 4096 {
            self.events.clear();
            self.events.push_back(SimulatorEvent::Fault(
                "simulator input queue overflow".into(),
            ));
            self.overflow = true;
        } else {
            self.events.push_back(event);
        }
    }
    fn input(&mut self, input: Input) {
        self.emit(SimulatorEvent::Surface(SurfaceEvent::Input(input)));
    }
    fn touch(&mut self, contact: Contact, phase: TouchPhase) {
        self.input(Input::Touch {
            surface: contact.surface,
            contact: contact.id,
            phase,
            x: contact.x,
            y: contact.y,
        });
    }
    fn key_event(&mut self, surface: SurfaceId, code: WatchKey, phase: KeyPhase) {
        self.input(Input::Key {
            source: KeySource::Surface(surface),
            code,
            phase,
        });
    }
    pub fn drain(&mut self) -> Vec<SimulatorEvent> {
        self.events.drain(..).collect()
    }
    pub fn pointer_enter(&mut self, surface: SurfaceId, x: f64, y: f64) {
        if !self.routes.contains_key(&surface) {
            return;
        }
        if self.mouse.is_some_and(|c| c.surface != surface)
            && let Some(c) = self.mouse.take()
        {
            self.touch(c, TouchPhase::Cancel);
        }
        self.pointer = Some((surface, x, y));
    }
    pub fn pointer_leave(&mut self) {
        if self.mouse.is_none() {
            self.pointer = None;
        }
    }
    pub fn pointer_unavailable(&mut self) {
        if let Some(contact) = self.mouse.take() {
            self.touch(contact, TouchPhase::Cancel);
        }
        self.pointer = None;
    }
    pub fn pointer_motion(&mut self, x: f64, y: f64) {
        let Some((surface, _, _)) = self.pointer else {
            return;
        };
        self.pointer = Some((surface, x, y));
        if let Some(old) = self.mouse
            && let Some((x, y)) = self.point(old.surface, x, y)
        {
            let contact = Contact { x, y, ..old };
            self.mouse = Some(contact);
            self.touch(contact, TouchPhase::Move);
        }
    }
    pub fn button(&mut self, pressed: bool) {
        if pressed && self.mouse.is_none() {
            if let Some((surface, x, y)) = self.pointer
                && let Some((x, y)) = self.point(surface, x, y)
            {
                let contact = Contact {
                    surface,
                    id: ContactId::new(if self.raw.contains(&42) || self.raw.contains(&54) {
                        2
                    } else {
                        1
                    })
                    .expect("mouse contact"),
                    x,
                    y,
                };
                self.mouse = Some(contact);
                self.touch(contact, TouchPhase::Down);
            }
        } else if !pressed && let Some(contact) = self.mouse.take() {
            self.touch(contact, TouchPhase::Up);
        }
    }
    pub fn keyboard_enter(&mut self, surface: SurfaceId, held: &[u32]) {
        self.keyboard_leave();
        if self.routes.contains_key(&surface) {
            self.keyboard = Some(surface);
            self.raw.extend(held.iter().copied().take(768));
            self.sync_keys();
        }
    }
    pub fn keyboard_leave(&mut self) {
        let keys = std::mem::take(&mut self.forwarded);
        for (surface, key) in keys {
            self.key_event(surface, key, KeyPhase::Released);
        }
        self.keyboard = None;
        self.raw.clear();
    }
    pub fn key(&mut self, raw: u32, pressed: bool) {
        if self.keyboard.is_none() || raw > 767 {
            return;
        }
        let changed = if pressed {
            self.raw.insert(raw)
        } else {
            self.raw.remove(&raw)
        };
        if !changed {
            return;
        }
        if pressed && raw == 20 && (self.raw.contains(&29) || self.raw.contains(&97)) {
            self.emit(SimulatorEvent::ToggleTheme);
        }
        self.sync_keys();
    }
    pub fn watch(&mut self, surface: SurfaceId, keys: &[WatchKey]) {
        if let Some(route) = self.routes.get_mut(&surface) {
            route.watched = if route.info.caps.watch_keys {
                keys.iter().copied().collect()
            } else {
                BTreeSet::new()
            };
        }
        self.sync_keys();
    }
    fn sync_keys(&mut self) {
        let wanted = if let Some(surface) = self.keyboard
            && let Some(route) = self.routes.get(&surface)
        {
            self.raw
                .iter()
                .filter_map(|raw| semantic_key(*raw))
                .filter(|key| route.watched.contains(key) && route.info.caps.watch_keys)
                .map(|key| (surface, key))
                .collect::<BTreeSet<_>>()
        } else {
            BTreeSet::new()
        };
        let released = self
            .forwarded
            .difference(&wanted)
            .copied()
            .collect::<Vec<_>>();
        let pressed = wanted
            .difference(&self.forwarded)
            .copied()
            .collect::<Vec<_>>();
        self.forwarded = wanted;
        for (surface, key) in released {
            self.key_event(surface, key, KeyPhase::Released);
        }
        for (surface, key) in pressed {
            self.key_event(surface, key, KeyPhase::Pressed);
        }
    }
    pub fn touch_down(&mut self, surface: SurfaceId, raw: i32, x: f64, y: f64) {
        let Some(id) = u32::try_from(raw)
            .ok()
            .and_then(|n| n.checked_add(3))
            .and_then(ContactId::new)
        else {
            return;
        };
        let Some((x, y)) = self.point(surface, x, y) else {
            return;
        };
        if let Some(old) = self.touches.remove(&raw) {
            self.touch(old, TouchPhase::Cancel);
        }
        if self.touches.len() >= 64 {
            return;
        }
        let c = Contact { surface, id, x, y };
        self.touches.insert(raw, c);
        self.touch(c, TouchPhase::Down);
    }
    pub fn touch_motion(&mut self, raw: i32, x: f64, y: f64) {
        let Some(old) = self.touches.get(&raw).copied() else {
            return;
        };
        let Some((x, y)) = self.point(old.surface, x, y) else {
            return;
        };
        let c = Contact { x, y, ..old };
        self.touches.insert(raw, c);
        self.touch(c, TouchPhase::Move);
    }
    pub fn touch_up(&mut self, raw: i32) {
        if let Some(c) = self.touches.remove(&raw) {
            self.touch(c, TouchPhase::Up);
        }
    }
    pub fn touch_cancel(&mut self) {
        for (_, c) in std::mem::take(&mut self.touches) {
            self.touch(c, TouchPhase::Cancel);
        }
    }
    pub fn cancel_contacts(&mut self, surface: SurfaceId) {
        if self.mouse.is_some_and(|c| c.surface == surface)
            && let Some(c) = self.mouse.take()
        {
            self.touch(c, TouchPhase::Cancel);
        }
        let contacts = self
            .touches
            .iter()
            .filter(|(_, c)| c.surface == surface)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in contacts {
            let c = self.touches.remove(&id).expect("contact");
            self.touch(c, TouchPhase::Cancel);
        }
    }
    pub fn scale(&mut self, surface: SurfaceId, scale: u32) {
        self.cancel_contacts(surface);
        if let Some(route) = self.routes.get_mut(&surface) {
            route.scale = scale.clamp(1, 8);
        }
    }
    pub fn remove(&mut self, surface: SurfaceId) {
        self.cancel_contacts(surface);
        if self.keyboard == Some(surface) {
            self.keyboard_leave();
        }
        if self.pointer.is_some_and(|(id, _, _)| id == surface) {
            self.pointer = None;
        }
        self.routes.remove(&surface);
    }
    #[allow(clippy::cast_possible_truncation)]
    fn point(&self, surface: SurfaceId, x: f64, y: f64) -> Option<(f32, f32)> {
        let scale = f64::from(self.routes.get(&surface)?.scale);
        let (x, y) = (x * scale, y * scale);
        if !x.is_finite() || !y.is_finite() || x.abs() > 1e8 || y.abs() > 1e8 {
            return None;
        }
        Some((x as f32, y as f32))
    }
}
fn semantic_key(raw: u32) -> Option<WatchKey> {
    Some(match raw {
        100 => WatchKey::Fn,
        1 => WatchKey::Esc,
        59 => WatchKey::F1,
        60 => WatchKey::F2,
        61 => WatchKey::F3,
        62 => WatchKey::F4,
        63 => WatchKey::F5,
        64 => WatchKey::F6,
        65 => WatchKey::F7,
        66 => WatchKey::F8,
        67 => WatchKey::F9,
        68 => WatchKey::F10,
        87 => WatchKey::F11,
        88 => WatchKey::F12,
        42 => WatchKey::LeftShift,
        54 => WatchKey::RightShift,
        29 => WatchKey::LeftControl,
        97 => WatchKey::RightControl,
        56 => WatchKey::LeftAlt,
        125 => WatchKey::LeftMeta,
        126 => WatchKey::RightMeta,
        _ => return None,
    })
}
