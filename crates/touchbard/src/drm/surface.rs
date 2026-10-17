//! This object and all of its destructors live exclusively on the device worker.
use super::{
    Frame, Presentation, Presented,
    dimensions::{self, PhysicalSize},
    events::PageFlip,
};
use drm::Device as DrmDevice;
use drm::{
    ClientCapability, DriverCapability,
    buffer::{Buffer, DrmFourcc},
    control::{
        self, AtomicCommitFlags, Device as ControlDevice, ResourceHandle, atomic::AtomicModeReq,
        connector, crtc, dumbbuffer::DumbBuffer, framebuffer, plane, property,
    },
};
use std::{
    fs::File,
    io,
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    path::Path,
};
use touchbar_protocol::{
    DamageRect, SwapchainHeader,
    damage::{Damage, merge_damage},
};

struct Card(File);
impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}
impl DrmDevice for Card {}
impl ControlDevice for Card {}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn prop<T: ResourceHandle>(card: &Card, handle: T, name: &str) -> io::Result<property::Handle> {
    for id in card.get_properties(handle)?.as_props_and_values().0 {
        if card.get_property(*id)?.name().to_bytes() == name.as_bytes() {
            return Ok(*id);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!("DRM property {name} missing"),
    ))
}
struct Selection {
    connector: connector::Info,
    crtc: crtc::Handle,
    plane: plane::Handle,
    mode: control::Mode,
}
fn select(card: &Card) -> io::Result<Selection> {
    let resources = card.resource_handles()?;
    for id in resources.connectors() {
        let connector = card.get_connector(*id, true)?;
        if connector.state() != connector::State::Connected
            || connector.interface() != connector::Interface::USB
        {
            continue;
        }
        let Some(mode) = connector
            .modes()
            .iter()
            .find(|m| {
                let (w, h) = m.size();
                w > 0 && h > 0 && w.max(h) / w.min(h) >= 30 && w.max(h) <= 16384
            })
            .copied()
        else {
            continue;
        };
        for encoder in connector.encoders() {
            let encoder = card.get_encoder(*encoder)?;
            for crtc in resources.filter_crtcs(encoder.possible_crtcs()) {
                for plane in card.plane_handles()? {
                    let info = card.get_plane(plane)?;
                    if info.formats().contains(&u32::from_le_bytes(*b"XR24"))
                        && resources
                            .filter_crtcs(info.possible_crtcs())
                            .contains(&crtc)
                    {
                        return Ok(Selection {
                            connector,
                            crtc,
                            plane,
                            mode,
                        });
                    }
                }
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "no compatible appletbdrm USB output",
    ))
}
struct Allocation {
    dumb: DumbBuffer,
    framebuffer: framebuffer::Handle,
}
struct Pending {
    started_us: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitMode {
    Nonblocking,
    Blocking,
}
#[derive(Debug)]
pub struct Completion {
    pub frames: Presented,
    pub commit_us: u32,
    pub mode: CommitMode,
}
#[derive(Debug)]
pub enum Submission {
    Pending,
    Complete(Completion),
}
/// The event descriptor is local to the daemon; only `buffers` are sent to the session.
pub struct Exports {
    pub header: SwapchainHeader,
    pub physical_size: PhysicalSize,
    pub buffers: Vec<OwnedFd>,
    pub events: OwnedFd,
}
pub struct Surface {
    card: Card,
    selection: Selection,
    allocations: Vec<Allocation>, // private black first, then the three shared buffers
    header: SwapchainHeader,
    physical_size: PhysicalSize,
    fb_property: property::Handle,
    damage_property: Option<property::Handle>,
    presentation: Presentation,
    pending: Option<Pending>,
    nonblocking: bool,
}
pub fn monotonic_us() -> u64 {
    let t = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    u64::try_from(t.tv_sec)
        .unwrap_or(0)
        .saturating_mul(1_000_000)
        .saturating_add(u64::try_from(t.tv_nsec).unwrap_or(0) / 1000)
}
fn duration(start: u64, end: u64) -> io::Result<u32> {
    let elapsed = end
        .checked_sub(start)
        .ok_or_else(|| invalid("DRM event predates submitted commit"))?;
    // Wire x is a positive i32, not an arbitrary u32.
    i32::try_from(elapsed)
        .map(|v| v.unsigned_abs())
        .map_err(|_| invalid("DRM commit duration exceeds protocol range"))
}
impl Surface {
    /// Check driver and master before any USB changes when probing an already bound display.
    pub fn check_master(path: &Path) -> io::Result<()> {
        let card = Self::card(path)?;
        card.acquire_master_lock()?;
        card.release_master_lock()
    }
    fn card(path: &Path) -> io::Result<Card> {
        let fd = rustix::fs::open(
            path,
            rustix::fs::OFlags::RDWR
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NONBLOCK
                | rustix::fs::OFlags::NOFOLLOW,
            rustix::fs::Mode::empty(),
        )?;
        let card = Card(File::from(fd));
        if card.get_driver()?.name() != "appletbdrm" {
            return Err(invalid("DRM node is not appletbdrm"));
        }
        Ok(card)
    }
    pub fn open(path: &Path, dmi_model: Option<&str>) -> io::Result<Self> {
        let card = Self::card(path)?;
        card.set_client_capability(ClientCapability::UniversalPlanes, true)?;
        card.set_client_capability(ClientCapability::Atomic, true)?;
        card.acquire_master_lock()?;
        let selection = select(&card)?;
        let (w, h) = selection.mode.size();
        let (w, h) = (u32::from(w), u32::from(h));
        let mut header = SwapchainHeader {
            logical_width: w.max(h),
            logical_height: w.min(h),
            physical_width: w,
            physical_height: h,
            format: u32::from_le_bytes(*b"XR24"),
            pitch: 0,
            buffer_size: 0,
            buffer_count: 3,
            frame_damage: false,
        };
        let physical_size = dimensions::resolve(
            (header.logical_width, header.logical_height),
            header
                .transform()
                .ok_or_else(|| invalid("invalid DRM output transform"))?,
            selection.connector.size(),
            dmi_model,
            dimensions::MEASURED_MODELS,
        )?;
        let fb_property = prop(&card, selection.plane, "FB_ID")?;
        let damage_property = prop(&card, selection.plane, "FB_DAMAGE_CLIPS").ok();
        header.frame_damage = damage_property.is_some();
        let nonblocking = card
            .get_driver_capability(DriverCapability::MonotonicTimestamp)
            .unwrap_or(0)
            == 1;
        let mut surface = Self {
            card,
            selection,
            allocations: Vec::with_capacity(4),
            header,
            physical_size,
            fb_property,
            damage_property,
            presentation: Presentation::new(3)?,
            pending: None,
            nonblocking,
        };
        for _ in 0..4 {
            let dumb = surface.card.create_dumb_buffer(
                (w.div_ceil(64) * 64, h),
                DrmFourcc::Xrgb8888,
                32,
            )?;
            let framebuffer = match surface.card.add_framebuffer(&dumb, 24, 32) {
                Ok(fb) => fb,
                Err(e) => {
                    let _ = surface.card.destroy_dumb_buffer(dumb);
                    return Err(e);
                }
            };
            surface.allocations.push(Allocation { dumb, framebuffer });
            // Explicitly clear every allocation, including padding, before exporting it.
            surface
                .card
                .map_dumb_buffer(&mut surface.allocations.last_mut().expect("just allocated").dumb)?
                .fill(0);
        }
        let pitch = surface.allocations[0].dumb.pitch();
        if surface.allocations.iter().any(|a| a.dumb.pitch() != pitch) {
            return Err(invalid("inconsistent DRM buffer pitches"));
        }
        surface.header.pitch = pitch;
        surface.header.buffer_size = u64::from(pitch) * u64::from(h);
        surface.header.validate()?;
        surface.modeset()?;
        Ok(surface)
    }
    pub fn header(&self) -> SwapchainHeader {
        self.header
    }
    pub fn nonblocking_supported(&self) -> bool {
        self.nonblocking
    }
    pub fn exports(&self) -> io::Result<Exports> {
        let buffers = self
            .allocations
            .iter()
            .skip(1)
            .map(|a| {
                self.card
                    .buffer_to_prime_fd(a.dumb.handle(), drm::CLOEXEC | drm::RDWR)
            })
            .collect::<io::Result<_>>()?;
        Ok(Exports {
            header: self.header,
            physical_size: self.physical_size.clone(),
            buffers,
            events: rustix::io::fcntl_dupfd_cloexec(&self.card, 0)?,
        })
    }
    fn modeset(&self) -> io::Result<()> {
        let s = &self.selection;
        let mut r = AtomicModeReq::new();
        r.add_property(
            s.connector.handle(),
            prop(&self.card, s.connector.handle(), "CRTC_ID")?,
            property::Value::CRTC(Some(s.crtc)),
        );
        let blob = self.card.create_property_blob(&s.mode)?;
        // The blob is destroyed on both property lookup failure and commit failure.
        let result = (|| {
            r.add_property(s.crtc, prop(&self.card, s.crtc, "MODE_ID")?, blob);
            r.add_property(
                s.crtc,
                prop(&self.card, s.crtc, "ACTIVE")?,
                property::Value::Boolean(true),
            );
            r.add_property(
                s.plane,
                self.fb_property,
                property::Value::Framebuffer(Some(self.allocations[0].framebuffer)),
            );
            r.add_property(
                s.plane,
                prop(&self.card, s.plane, "CRTC_ID")?,
                property::Value::CRTC(Some(s.crtc)),
            );
            for (name, value) in [
                ("SRC_X", 0),
                ("SRC_Y", 0),
                ("SRC_W", u64::from(s.mode.size().0) << 16),
                ("SRC_H", u64::from(s.mode.size().1) << 16),
                ("CRTC_W", u64::from(s.mode.size().0)),
                ("CRTC_H", u64::from(s.mode.size().1)),
            ] {
                r.add_property(
                    s.plane,
                    prop(&self.card, s.plane, name)?,
                    property::Value::UnsignedRange(value),
                );
            }
            for name in ["CRTC_X", "CRTC_Y"] {
                r.add_property(
                    s.plane,
                    prop(&self.card, s.plane, name)?,
                    property::Value::SignedRange(0),
                );
            }
            if let Some(p) = self.damage_property {
                r.add_property(s.plane, p, property::Value::Blob(0));
            }
            self.card.atomic_commit(AtomicCommitFlags::ALLOW_MODESET, r)
        })();
        let _ = self.card.destroy_property_blob(blob.into());
        result
    }
    fn flip(
        &self,
        index: u32,
        damage: &Damage,
        flags: AtomicCommitFlags,
    ) -> io::Result<(u64, u64)> {
        let mut request = AtomicModeReq::new();
        request.add_property(
            self.selection.plane,
            self.fb_property,
            property::Value::Framebuffer(Some(
                self.allocations[usize::try_from(index).expect("validated index") + 1].framebuffer,
            )),
        );
        let blob = if let (Some(property), Damage::Rects(rects)) = (self.damage_property, damage) {
            let mut data = rects
                .iter()
                .flat_map(|r| {
                    [
                        i32::from(r.x),
                        i32::from(r.y),
                        i32::from(r.x) + i32::from(r.width),
                        i32::from(r.y) + i32::from(r.height),
                    ]
                })
                .flat_map(i32::to_ne_bytes)
                .collect::<Vec<_>>();
            let blob = drm_ffi::mode::create_property_blob(self.card.as_fd(), &mut data)?.blob_id;
            request.add_property(
                self.selection.plane,
                property,
                property::Value::Blob(u64::from(blob)),
            );
            Some(blob)
        } else {
            // Full must clear the *previous* clips, which otherwise persist in plane state.
            if let Some(property) = self.damage_property {
                request.add_property(self.selection.plane, property, property::Value::Blob(0));
            }
            None
        };
        let start = monotonic_us();
        let result = self.card.atomic_commit(flags, request);
        let end = monotonic_us();
        if let Some(blob) = blob {
            let _ = self.card.destroy_property_blob(u64::from(blob));
        }
        result.map(|()| (start, end))
    }
    pub fn submit(
        &mut self,
        frame: Frame,
        damage: &[DamageRect],
        requested: CommitMode,
    ) -> io::Result<Submission> {
        if damage.len() > touchbar_protocol::MAX_FRAME_DAMAGE {
            return Err(invalid("too many damage rectangles"));
        }
        let damage = merge_damage(
            damage,
            self.header.physical_width,
            self.header.physical_height,
        )?;
        let damage = self.presentation.begin(frame, damage)?;
        if requested == CommitMode::Nonblocking && self.nonblocking {
            match self.flip(
                frame.index,
                &damage,
                AtomicCommitFlags::NONBLOCK | AtomicCommitFlags::PAGE_FLIP_EVENT,
            ) {
                Ok((start, _)) => {
                    self.pending = Some(Pending { started_us: start });
                    return Ok(Submission::Pending);
                }
                Err(e) if matches!(e.raw_os_error(), Some(22 | 95 | 38)) => {
                    self.nonblocking = false
                }
                Err(e) => {
                    self.presentation.failed();
                    return Err(e);
                }
            }
        }
        // Unsupported nonblocking is retried here, on the worker, never on the event loop.
        let (start, end) = match self.flip(frame.index, &damage, AtomicCommitFlags::empty()) {
            Ok(timing) => timing,
            Err(e) => {
                self.presentation.failed();
                return Err(e);
            }
        };
        Ok(Submission::Complete(Completion {
            frames: self.presentation.complete()?,
            commit_us: duration(start, end)?,
            mode: CommitMode::Blocking,
        }))
    }
    pub fn page_flip(&mut self, event: PageFlip) -> io::Result<Option<Completion>> {
        if event.crtc != 0 && event.crtc != u32::from(self.selection.crtc) {
            return Ok(None);
        }
        let Some(pending) = self.pending.take() else {
            return Ok(None);
        };
        let commit_us = duration(pending.started_us, event.timestamp_us)?;
        Ok(Some(Completion {
            frames: self.presentation.complete()?,
            commit_us,
            mode: CommitMode::Nonblocking,
        }))
    }
    /// Diagnostic only: a complete initialized buffer, optionally with a changed physical rectangle.
    pub fn paint_probe(&mut self, index: u32, patch: Option<DamageRect>) -> io::Result<()> {
        if index >= 3
            || patch
                .is_some_and(|r| !r.fits(self.header.physical_width, self.header.physical_height))
        {
            return Err(invalid("invalid probe paint region"));
        }
        let pitch = usize::try_from(self.header.pitch).map_err(|_| invalid("pitch overflow"))?;
        let (w, h) = (self.header.physical_width, self.header.physical_height);
        let allocation = &mut self.allocations[usize::try_from(index).expect("checked index") + 1];
        let mut pixels = self.card.map_dumb_buffer(&mut allocation.dumb)?;
        for y in 0..h {
            for x in 0..w {
                let changed = patch.is_some_and(|r| {
                    x >= u32::from(r.x)
                        && x < u32::from(r.x) + u32::from(r.width)
                        && y >= u32::from(r.y)
                        && y < u32::from(r.y) + u32::from(r.height)
                });
                let shade = if changed {
                    0x00ff_ffffu32
                } else if (x / 8 + y / 8) % 2 == 0 {
                    0x0040_4040
                } else {
                    0x0080_8080
                };
                let offset = usize::try_from(y).expect("bounded") * pitch
                    + usize::try_from(x).expect("bounded") * 4;
                pixels[offset..offset + 4].copy_from_slice(&shade.to_ne_bytes());
            }
        }
        Ok(())
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        // All calls, including cleanup after partial initialization, stay on the worker.
        for a in self.allocations.drain(..) {
            let _ = self.card.destroy_framebuffer(a.framebuffer);
            let _ = self.card.destroy_dumb_buffer(a.dumb);
        }
        let _ = self.card.release_master_lock();
    }
}
