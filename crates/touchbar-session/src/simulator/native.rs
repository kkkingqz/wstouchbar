// Copyright (c) 2026 Cameron Cooper
// SPDX-License-Identifier: MIT OR Apache-2.0
// Wayland/shm transport adapted from the original TouchBar preview_output.rs.
use super::{InputRouter, SimulatorEvent, argb8888, physical_size};
use crate::surface::*;
use anyhow::{Result, anyhow, bail, ensure};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs::File,
    io,
    os::{
        fd::{AsFd, OwnedFd},
        unix::{fs::FileExt, net::UnixStream},
    },
    path::Path,
    rc::Rc,
    sync::Arc,
    time::Duration,
};
use touchbar_model::*;
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop,
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry,
        wl_seat, wl_shm, wl_shm_pool, wl_surface, wl_touch,
    },
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
#[derive(Clone, Copy)]
struct WindowId(SurfaceId);
#[derive(Clone, Copy)]
struct BufferId {
    window: SurfaceId,
    serial: u64,
}
#[derive(Clone, Copy)]
struct FrameId {
    window: SurfaceId,
    serial: u64,
    generation: Generation,
}
#[derive(Clone, Copy)]
struct OutputId(u32);
struct Slot {
    serial: u64,
    buffer: wl_buffer::WlBuffer,
    file: File,
    size: (u32, u32),
    busy: bool,
}
struct Pending {
    frame: Arc<PresentedFrame>,
    notify: bool,
}
struct Window {
    info: SurfaceInfo,
    base: wl_surface::WlSurface,
    xdg: xdg_surface::XdgSurface,
    top: xdg_toplevel::XdgToplevel,
    configured: bool,
    closed: bool,
    scale: u32,
    preferred: Option<u32>,
    outputs: BTreeSet<u32>,
    slots: Vec<Slot>,
    pending: Option<Pending>,
    cached: Option<Arc<PresentedFrame>>,
    awaiting: Option<u64>,
    force: bool,
}
impl Window {
    fn logical(&self) -> (u32, u32) {
        (
            self.info.width_px.div_ceil(self.scale),
            self.info.height_px.div_ceil(self.scale),
        )
    }
    fn hints(&self) -> Result<()> {
        let logical = self.logical();
        physical_size(logical, self.scale).map_err(|e| anyhow!(e))?;
        let w = i32::try_from(logical.0)?;
        let h = i32::try_from(logical.1)?;
        self.top.set_min_size(w, h);
        self.top.set_max_size(w, h);
        self.xdg.set_window_geometry(0, 0, w, h);
        self.base.set_buffer_scale(i32::try_from(self.scale)?);
        Ok(())
    }
    fn destroy(self) {
        for slot in self.slots {
            slot.buffer.destroy();
        }
        self.top.destroy();
        self.xdg.destroy();
        self.base.destroy();
    }
}
struct World {
    connection: Connection,
    qh: QueueHandle<Self>,
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm: Option<xdg_wm_base::XdgWmBase>,
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    touch: Option<wl_touch::WlTouch>,
    outputs: BTreeMap<u32, (wl_output::WlOutput, u32)>,
    windows: BTreeMap<SurfaceId, Window>,
    router: InputRouter,
    events: VecDeque<SimulatorEvent>,
    serial: u64,
    error: Option<String>,
}
impl World {
    fn emit_inputs(&mut self) {
        self.events.extend(self.router.drain());
        if self.events.len() > 8192 {
            self.error = Some("Wayland event queue overflow".into());
        }
    }
    fn ready(&self) -> bool {
        self.compositor.is_some() && self.shm.is_some() && self.wm.is_some()
    }
    fn create(&mut self, info: SurfaceInfo) -> Result<()> {
        let compositor = self
            .compositor
            .as_ref()
            .ok_or_else(|| anyhow!("Wayland compositor unavailable"))?;
        ensure!(
            compositor.version() >= 3,
            "simulator requires wl_compositor v3"
        );
        let base = compositor.create_surface(&self.qh, WindowId(info.id));
        let xdg =
            self.wm
                .as_ref()
                .expect("wm ready")
                .get_xdg_surface(&base, &self.qh, WindowId(info.id));
        let top = xdg.get_toplevel(&self.qh, WindowId(info.id));
        top.set_title(format!("wstouchbar — {}", info.name.as_str()));
        top.set_app_id("io.github.kkkingqz.wstouchbar.simulator".into());
        let window = Window {
            info: info.clone(),
            base,
            xdg,
            top,
            configured: false,
            closed: false,
            scale: 1,
            preferred: None,
            outputs: BTreeSet::new(),
            slots: Vec::new(),
            pending: None,
            cached: None,
            awaiting: None,
            force: false,
        };
        window.hints()?;
        window.base.commit();
        self.router.add(info.clone(), 1);
        self.windows.insert(info.id, window);
        Ok(())
    }
    fn rescale(&mut self, id: SurfaceId) -> Result<()> {
        let Some(window) = self.windows.get_mut(&id) else {
            return Ok(());
        };
        let scale = window.preferred.unwrap_or_else(|| {
            window
                .outputs
                .iter()
                .filter_map(|id| self.outputs.get(id).map(|(_, scale)| *scale))
                .max()
                .unwrap_or(1)
        });
        ensure!(
            (1..=8).contains(&scale),
            "unsupported Wayland output scale {scale}"
        );
        if scale == window.scale {
            return Ok(());
        }
        window.scale = scale;
        window.hints()?;
        window.force = true;
        self.router.scale(id, scale);
        if window.pending.is_none()
            && let Some(frame) = &window.cached
        {
            window.pending = Some(Pending {
                frame: frame.clone(),
                notify: false,
            });
        }
        self.try_submit(id)?;
        self.emit_inputs();
        Ok(())
    }
    fn present(&mut self, id: SurfaceId, frame: Arc<PresentedFrame>) -> Result<Presentation> {
        let Some(window) = self.windows.get_mut(&id) else {
            return Ok(Presentation::Complete);
        };
        if window.closed {
            return Ok(Presentation::Complete);
        }
        if let Some(pixels) = &frame.pixels {
            ensure!(
                (pixels.width, pixels.height) == (window.info.width_px, window.info.height_px),
                "simulator frame geometry mismatch"
            );
            window.cached = Some(frame.clone());
        } else if !window.force {
            return Ok(Presentation::Complete);
        }
        ensure!(
            window.awaiting.is_none() && window.pending.as_ref().is_none_or(|p| !p.notify),
            "simulator already has a frame in flight"
        );
        window.pending = Some(Pending {
            frame,
            notify: true,
        });
        self.try_submit(id)?;
        Ok(Presentation::Pending)
    }
    fn try_submit(&mut self, id: SurfaceId) -> Result<()> {
        let shm = self
            .shm
            .as_ref()
            .ok_or_else(|| anyhow!("wl_shm unavailable"))?;
        let Some(window) = self.windows.get_mut(&id) else {
            return Ok(());
        };
        if !window.configured || window.closed || window.pending.is_none() {
            return Ok(());
        }
        let size = physical_size(window.logical(), window.scale).map_err(|e| anyhow!(e))?;
        let index = if let Some(index) = window.slots.iter().position(|s| !s.busy) {
            index
        } else if window.slots.len() < 3 {
            window.slots.len()
        } else {
            return Ok(());
        };
        if window.slots.get(index).is_none_or(|s| s.size != size) {
            self.serial = self
                .serial
                .checked_add(1)
                .ok_or_else(|| anyhow!("Wayland buffer serial exhausted"))?;
            let fd = rustix::fs::memfd_create(
                "wstouchbar-simulator",
                rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
            )?;
            let file = File::from(fd);
            let bytes = u64::from(size.0) * u64::from(size.1) * 4;
            file.set_len(bytes)?;
            rustix::fs::fcntl_add_seals(
                &file,
                rustix::fs::SealFlags::SHRINK
                    | rustix::fs::SealFlags::GROW
                    | rustix::fs::SealFlags::SEAL,
            )?;
            let pool = shm.create_pool(file.as_fd(), i32::try_from(bytes)?, &self.qh, ());
            let buffer = pool.create_buffer(
                0,
                i32::try_from(size.0)?,
                i32::try_from(size.1)?,
                i32::try_from(size.0 * 4)?,
                wl_shm::Format::Argb8888,
                &self.qh,
                BufferId {
                    window: id,
                    serial: self.serial,
                },
            );
            pool.destroy();
            let slot = Slot {
                serial: self.serial,
                buffer,
                file,
                size,
                busy: false,
            };
            if index < window.slots.len() {
                let old = std::mem::replace(&mut window.slots[index], slot);
                old.buffer.destroy();
            } else {
                window.slots.push(slot);
            }
        }
        let pending = window.pending.take().expect("pending frame");
        let pixels = pending
            .frame
            .pixels
            .as_ref()
            .or_else(|| window.cached.as_ref().and_then(|f| f.pixels.as_ref()))
            .ok_or_else(|| anyhow!("first simulator frame has no pixels"))?;
        let bytes = padded_argb(&pixels.pixels, (pixels.width, pixels.height), size)?;
        let slot = &mut window.slots[index];
        slot.file.write_all_at(&bytes, 0)?;
        slot.busy = true;
        window.base.attach(Some(&slot.buffer), 0, 0);
        window.base.set_buffer_scale(i32::try_from(window.scale)?);
        if window.base.version() >= 4 {
            window
                .base
                .damage_buffer(0, 0, i32::try_from(size.0)?, i32::try_from(size.1)?);
        } else {
            let logical = window.logical();
            window
                .base
                .damage(0, 0, i32::try_from(logical.0)?, i32::try_from(logical.1)?);
        }
        if pending.notify {
            self.serial = self
                .serial
                .checked_add(1)
                .ok_or_else(|| anyhow!("Wayland frame serial exhausted"))?;
            window.base.frame(
                &self.qh,
                FrameId {
                    window: id,
                    serial: self.serial,
                    generation: pending.frame.generation,
                },
            );
            window.awaiting = Some(self.serial);
        }
        window.force = false;
        window.base.commit();
        tracing::debug!(
            event = "simulator_frame_committed",
            surface = id.0,
            generation = pending.frame.generation.0
        );
        Ok(())
    }
    fn remove(&mut self, id: SurfaceId) {
        self.router.remove(id);
        self.emit_inputs();
        if let Some(window) = self.windows.remove(&id) {
            window.destroy();
        }
        let _ = self.connection.flush();
    }
    fn fail(&mut self, error: impl std::fmt::Display) {
        self.error = Some(error.to_string());
    }
}
fn padded_argb(rgba: &[u8], source: (u32, u32), target: (u32, u32)) -> Result<Vec<u8>> {
    ensure!(
        source.0 > 0 && source.1 > 0 && target.0 >= source.0 && target.1 >= source.1,
        "invalid shm padding"
    );
    let source_stride = usize::try_from(source.0)? * 4;
    let stride = usize::try_from(target.0)? * 4;
    let height = usize::try_from(source.1)?;
    ensure!(
        rgba.len() == source_stride * height,
        "RGBA payload dimensions mismatch"
    );
    let converted = argb8888(rgba).map_err(|e| anyhow!(e))?;
    if source == target {
        return Ok(converted);
    }
    let mut output = vec![0; stride * usize::try_from(target.1)?];
    for y in 0..height {
        let row = &converted[y * source_stride..(y + 1) * source_stride];
        output[y * stride..y * stride + source_stride].copy_from_slice(row);
        for x in (source_stride..stride).step_by(4) {
            output[y * stride + x..y * stride + x + 4].copy_from_slice(&row[source_stride - 4..]);
        }
    }
    for y in height..usize::try_from(target.1)? {
        output.copy_within((height - 1) * stride..height * stride, y * stride);
    }
    Ok(output)
}
pub struct Simulator {
    queue: EventQueue<World>,
    world: Rc<RefCell<World>>,
    io: async_io::Async<OwnedFd>,
    flush_blocked: bool,
}
impl Simulator {
    pub async fn connect(infos: Vec<SurfaceInfo>) -> Result<Self> {
        Self::build(Connection::connect_to_env()?, infos).await
    }
    pub async fn connect_at(path: &Path, infos: Vec<SurfaceInfo>) -> Result<Self> {
        Self::build(Connection::from_socket(UnixStream::connect(path)?)?, infos).await
    }
    async fn build(connection: Connection, infos: Vec<SurfaceInfo>) -> Result<Self> {
        ensure!(
            !infos.is_empty() && infos.len() <= 8,
            "simulator requires 1..8 surfaces"
        );
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        for info in &infos {
            info.validate().map_err(|e| anyhow!(e))?;
            ensure!(
                info.width_px <= 8192
                    && info.height_px <= 8192
                    && u64::from(info.width_px) * u64::from(info.height_px) <= 4 * 1024 * 1024,
                "simulator surface exceeds size budget"
            );
            ensure!(
                ids.insert(info.id) && names.insert(info.name.clone()),
                "duplicate simulator surface"
            );
        }
        let queue = connection.new_event_queue::<World>();
        let qh = queue.handle();
        let io = async_io::Async::new(rustix::io::dup(queue.as_fd())?)?;
        connection.display().get_registry(&qh, ());
        let world = Rc::new(RefCell::new(World {
            connection,
            qh,
            compositor: None,
            shm: None,
            wm: None,
            seat: None,
            pointer: None,
            keyboard: None,
            touch: None,
            outputs: BTreeMap::new(),
            windows: BTreeMap::new(),
            router: InputRouter::default(),
            events: VecDeque::new(),
            serial: 0,
            error: None,
        }));
        let mut simulator = Self {
            queue,
            world,
            io,
            flush_blocked: false,
        };
        futures_lite::future::or(
            async {
                loop {
                    simulator.pump()?;
                    if simulator.world.borrow().ready() {
                        break;
                    }
                    simulator.wait_io().await?;
                }
                for info in infos {
                    simulator.world.borrow_mut().create(info)?;
                }
                loop {
                    simulator.pump()?;
                    if simulator
                        .world
                        .borrow()
                        .windows
                        .values()
                        .all(|w| w.configured)
                    {
                        break;
                    }
                    simulator.wait_io().await?;
                }
                simulator.world.borrow_mut().events.clear();
                Ok::<(), anyhow::Error>(())
            },
            async {
                async_io::Timer::after(Duration::from_secs(5)).await;
                bail!("Wayland simulator initialization timed out")
            },
        )
        .await?;
        Ok(simulator)
    }
    fn pump(&mut self) -> Result<()> {
        self.queue.dispatch_pending(&mut self.world.borrow_mut())?;
        if fd_ready(self.io.get_ref(), rustix::event::PollFlags::IN)?
            && let Some(guard) = self.queue.prepare_read()
        {
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(e))
                    if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(anyhow!(e)),
            }
        }
        self.queue.dispatch_pending(&mut self.world.borrow_mut())?;
        let mut world = self.world.borrow_mut();
        world.emit_inputs();
        if let Some(error) = world.error.take() {
            bail!(error)
        }
        self.flush_blocked = match world.connection.flush() {
            Ok(()) => false,
            Err(wayland_client::backend::WaylandError::Io(e))
                if e.kind() == io::ErrorKind::WouldBlock =>
            {
                true
            }
            Err(e) => return Err(anyhow!(e)),
        };
        Ok(())
    }
    async fn wait_io(&self) -> Result<()> {
        let read = self.io.read_with(|fd| {
            if fd_ready(fd, rustix::event::PollFlags::IN)? {
                Ok(())
            } else {
                Err(io::ErrorKind::WouldBlock.into())
            }
        });
        let write = async {
            if self.flush_blocked {
                self.io
                    .write_with(|fd| {
                        if fd_ready(fd, rustix::event::PollFlags::OUT)? {
                            Ok(())
                        } else {
                            Err(io::ErrorKind::WouldBlock.into())
                        }
                    })
                    .await
            } else {
                std::future::pending::<io::Result<()>>().await
            }
        };
        futures_lite::future::or(read, write).await?;
        Ok(())
    }
    pub async fn next_event(&mut self) -> Result<SimulatorEvent> {
        loop {
            self.pump()?;
            if let Some(event) = self.world.borrow_mut().events.pop_front() {
                return Ok(event);
            }
            self.wait_io().await?;
        }
    }
    pub fn surfaces(&self) -> Vec<Box<dyn Surface>> {
        self.world
            .borrow()
            .windows
            .keys()
            .map(|id| {
                Box::new(SimulatorSurface {
                    world: self.world.clone(),
                    id: *id,
                }) as Box<dyn Surface>
            })
            .collect()
    }
    pub fn closed(&self) -> bool {
        self.world.borrow().windows.values().all(|w| w.closed)
    }
}
fn fd_ready(fd: &OwnedFd, flags: rustix::event::PollFlags) -> io::Result<bool> {
    let mut fds = [rustix::event::PollFd::new(fd, flags)];
    rustix::event::poll(
        &mut fds,
        Some(&rustix::event::Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        }),
    )?;
    Ok(!fds[0].revents().is_empty())
}
struct SimulatorSurface {
    world: Rc<RefCell<World>>,
    id: SurfaceId,
}
impl Surface for SimulatorSurface {
    fn info(&self) -> SurfaceInfo {
        self.world.borrow().windows[&self.id].info.clone()
    }
    fn needs_pixels(&self) -> bool {
        true
    }
    fn present(&mut self, frame: Arc<PresentedFrame>) -> Result<Presentation, String> {
        let result = self
            .world
            .borrow_mut()
            .present(self.id, frame)
            .map_err(|e| e.to_string());
        let world = self.world.borrow();
        let _ = world.connection.flush();
        result
    }
    fn send_key(&mut self, key: SystemKey, phase: KeyPhase) -> Result<(), String> {
        tracing::info!(event = "simulator_key", surface = self.id.0, ?key, ?phase);
        Ok(())
    }
    fn watch_keys(&mut self, keys: &[WatchKey]) -> Result<(), String> {
        let mut world = self.world.borrow_mut();
        world.router.watch(self.id, keys);
        world.emit_inputs();
        Ok(())
    }
    fn close(&mut self) {
        self.world.borrow_mut().remove(self.id);
    }
}
impl Drop for SimulatorSurface {
    fn drop(&mut self) {
        self.world.borrow_mut().remove(self.id);
    }
}
impl Dispatch<wl_registry::WlRegistry, ()> for World {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_compositor" if state.compositor.is_none() => {
                    state.compositor = Some(registry.bind(name, version.min(6), qh, ()))
                }
                "wl_shm" if state.shm.is_none() => state.shm = Some(registry.bind(name, 1, qh, ())),
                "xdg_wm_base" if state.wm.is_none() => {
                    state.wm = Some(registry.bind(name, 1, qh, ()))
                }
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind(name, version.min(7), qh, ()))
                }
                "wl_output" if state.outputs.len() < 32 => {
                    let output = registry.bind(name, version.min(4), qh, OutputId(name));
                    state.outputs.insert(name, (output, 1));
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                if let Some((output, _)) = state.outputs.remove(&name) {
                    if output.version() >= 3 {
                        output.release();
                    }
                    let ids = state
                        .windows
                        .values_mut()
                        .filter_map(|w| w.outputs.remove(&name).then_some(w.info.id))
                        .collect::<Vec<_>>();
                    for id in ids {
                        if let Err(error) = state.rescale(id) {
                            state.fail(error);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
impl Dispatch<xdg_wm_base::XdgWmBase, ()> for World {
    fn event(
        _: &mut Self,
        wm: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm.pong(serial);
        }
    }
}
impl Dispatch<xdg_surface::XdgSurface, WindowId> for World {
    fn event(
        state: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
            if let Some(w) = state.windows.get_mut(&id.0) {
                w.configured = true;
                tracing::info!(
                    event = "simulator_surface_ready",
                    name = w.info.name.as_str(),
                    surface = id.0.0
                );
            }
            if let Err(error) = state.try_submit(id.0) {
                state.fail(error);
            }
        }
    }
}
impl Dispatch<xdg_toplevel::XdgToplevel, WindowId> for World {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Close = event {
            if let Some(w) = state.windows.get_mut(&id.0) {
                w.closed = true;
            }
            state.router.remove(id.0);
            state.emit_inputs();
            state
                .events
                .push_back(SimulatorEvent::Surface(SurfaceEvent::Closed(id.0)));
        }
    }
}
impl Dispatch<wl_surface::WlSurface, WindowId> for World {
    fn event(
        state: &mut Self,
        _: &wl_surface::WlSurface,
        event: wl_surface::Event,
        id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_surface::Event::Enter { output } => {
                if let Some(output) = output.data::<OutputId>()
                    && let Some(w) = state.windows.get_mut(&id.0)
                {
                    w.outputs.insert(output.0);
                }
            }
            wl_surface::Event::Leave { output } => {
                if let Some(output) = output.data::<OutputId>()
                    && let Some(w) = state.windows.get_mut(&id.0)
                {
                    w.outputs.remove(&output.0);
                }
            }
            wl_surface::Event::PreferredBufferScale { factor } => {
                if let Some(w) = state.windows.get_mut(&id.0) {
                    w.preferred = u32::try_from(factor).ok();
                }
            }
            _ => {}
        }
        if let Err(error) = state.rescale(id.0) {
            state.fail(error);
        }
    }
}
impl Dispatch<wl_output::WlOutput, OutputId> for World {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        id: &OutputId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Scale { factor } = event {
            if let Some((_, scale)) = state.outputs.get_mut(&id.0) {
                *scale = u32::try_from(factor).unwrap_or(0);
            }
            let ids = state
                .windows
                .values()
                .filter(|w| w.outputs.contains(&id.0))
                .map(|w| w.info.id)
                .collect::<Vec<_>>();
            for id in ids {
                if let Err(error) = state.rescale(id) {
                    state.fail(error);
                }
            }
        }
    }
}
impl Dispatch<wl_buffer::WlBuffer, BufferId> for World {
    fn event(
        state: &mut Self,
        _: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        id: &BufferId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            if let Some(window) = state.windows.get_mut(&id.window)
                && let Some(slot) = window.slots.iter_mut().find(|s| s.serial == id.serial)
            {
                slot.busy = false;
            }
            if let Err(error) = state.try_submit(id.window) {
                state.fail(error);
            }
        }
    }
}
impl Dispatch<wl_callback::WlCallback, FrameId> for World {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        id: &FrameId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event
            && let Some(w) = state.windows.get_mut(&id.window)
            && w.awaiting == Some(id.serial)
        {
            w.awaiting = None;
            state
                .events
                .push_back(SimulatorEvent::Surface(SurfaceEvent::Presented {
                    surface: id.window,
                    generation: id.generation,
                }));
            tracing::debug!(event = "simulator_frame_presented", surface = id.window.0);
        }
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for World {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            if caps.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            } else if !caps.contains(wl_seat::Capability::Pointer)
                && let Some(p) = state.pointer.take()
            {
                state.router.pointer_unavailable();
                p.release();
            }
            if caps.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            } else if !caps.contains(wl_seat::Capability::Keyboard)
                && let Some(k) = state.keyboard.take()
            {
                state.router.keyboard_leave();
                k.release();
            }
            if caps.contains(wl_seat::Capability::Touch) && state.touch.is_none() {
                state.touch = Some(seat.get_touch(qh, ()));
            } else if !caps.contains(wl_seat::Capability::Touch)
                && let Some(t) = state.touch.take()
            {
                state.router.touch_cancel();
                t.release();
            }
        }
    }
}
impl Dispatch<wl_pointer::WlPointer, ()> for World {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                if let Some(id) = surface.data::<WindowId>() {
                    state.router.pointer_enter(id.0, surface_x, surface_y);
                }
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => state.router.pointer_motion(surface_x, surface_y),
            wl_pointer::Event::Leave { .. } => state.router.pointer_leave(),
            wl_pointer::Event::Button {
                button: 0x110,
                state: WEnum::Value(button),
                ..
            } => state
                .router
                .button(button == wl_pointer::ButtonState::Pressed),
            _ => {}
        }
    }
}
impl Dispatch<wl_keyboard::WlKeyboard, ()> for World {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { surface, keys, .. } => {
                if let Some(id) = surface.data::<WindowId>() {
                    let keys = keys
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .take(768)
                        .map(|c| u32::from_ne_bytes(*c))
                        .collect::<Vec<_>>();
                    state.router.keyboard_enter(id.0, &keys);
                }
            }
            wl_keyboard::Event::Leave { .. } => state.router.keyboard_leave(),
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(phase),
                ..
            } => state
                .router
                .key(key, phase == wl_keyboard::KeyState::Pressed),
            _ => {}
        }
    }
}
impl Dispatch<wl_touch::WlTouch, ()> for World {
    fn event(
        state: &mut Self,
        _: &wl_touch::WlTouch,
        event: wl_touch::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_touch::Event::Down {
                surface, id, x, y, ..
            } => {
                if let Some(surface) = surface.data::<WindowId>() {
                    state.router.touch_down(surface.0, id, x, y);
                }
            }
            wl_touch::Event::Motion { id, x, y, .. } => state.router.touch_motion(id, x, y),
            wl_touch::Event::Up { id, .. } => state.router.touch_up(id),
            wl_touch::Event::Cancel => state.router.touch_cancel(),
            _ => {}
        }
    }
}
delegate_noop!(World:ignore wl_compositor::WlCompositor);
delegate_noop!(World:ignore wl_shm::WlShm);
delegate_noop!(World:ignore wl_shm_pool::WlShmPool);
