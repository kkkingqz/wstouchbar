//! Thread-owned EGL imports and shared-memory copy targets.
#![allow(unsafe_code)]
use crate::{
    DamageRect, TargetFrame, TargetPath,
    egl::DeviceContext,
    native::Prepared,
    target_history::History,
    target_memory::{Buffer, Kind},
};
use anyhow::{Result, anyhow, ensure};
use glow::HasContext;
use std::{
    os::fd::{AsFd, OwnedFd},
    rc::Rc,
};
use touchbar_model::SurfaceId;
use touchbar_protocol::SwapchainHeader;
struct Image {
    device: Rc<DeviceContext>,
    image: Option<khronos_egl::Image>,
    renderbuffer: Option<glow::Renderbuffer>,
    fbo: Option<glow::Framebuffer>,
}
impl Image {
    fn new(device: Rc<DeviceContext>, buffer: &Buffer, header: SwapchainHeader) -> Result<Self> {
        ensure!(
            device
                .gl
                .supported_extensions()
                .contains("GL_OES_EGL_image"),
            "GL_OES_EGL_image unavailable"
        );
        let image = device.owner.import_dma_buf(buffer.fd.as_fd(), header)?;
        let mut target = Self {
            device,
            image: Some(image),
            renderbuffer: None,
            fbo: None,
        };
        let bind = target.device.owner.image_renderbuffer()?;
        let gl = &target.device.gl;
        // SAFETY: imported image and all attachments belong to this current context;
        // partial resources are cleaned by Image::drop on every error path.
        unsafe {
            let renderbuffer = gl.create_renderbuffer().map_err(|e| anyhow!(e))?;
            target.renderbuffer = Some(renderbuffer);
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(renderbuffer));
            bind(glow::RENDERBUFFER, image.as_ptr());
            ensure!(
                gl.get_error() == glow::NO_ERROR,
                "EGLImage renderbuffer import failed"
            );
            let fbo = gl.create_framebuffer().map_err(|e| anyhow!(e))?;
            target.fbo = Some(fbo);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::RENDERBUFFER,
                Some(renderbuffer),
            );
            ensure!(
                gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE
                    && gl.get_error() == glow::NO_ERROR,
                "imported framebuffer incomplete"
            );
        }
        Ok(target)
    }
}
impl Drop for Image {
    fn drop(&mut self) {
        if self.device.owner.current().is_ok() {
            // SAFETY: handles are uniquely owned and their device/context is current.
            unsafe {
                if let Some(f) = self.fbo.take() {
                    self.device.gl.delete_framebuffer(f);
                }
                if let Some(r) = self.renderbuffer.take() {
                    self.device.gl.delete_renderbuffer(r);
                }
            }
        }
        if let Some(image) = self.image.take() {
            self.device.owner.destroy_dma_image(image);
        }
    }
}
pub(crate) struct Target {
    pub device: Rc<DeviceContext>,
    pub header: SwapchainHeader,
    buffers: Vec<Buffer>,
    images: Vec<Image>,
    path: TargetPath,
    warning: Option<String>,
    history: History,
    surface: Option<SurfaceId>,
}
impl Target {
    pub fn new(
        device: Rc<DeviceContext>,
        fds: Vec<OwnedFd>,
        header: SwapchainHeader,
    ) -> Result<Self> {
        header.validate()?;
        ensure!(
            fds.len() == usize::from(header.buffer_count),
            "swapchain descriptor count mismatch"
        );
        let mut buffers = fds
            .into_iter()
            .map(|fd| Buffer::new(fd, header))
            .collect::<Result<Vec<_>>>()?;
        let imported = if buffers.iter().all(|b| b.kind == Kind::DmaBuf) {
            buffers
                .iter()
                .map(|b| Image::new(device.clone(), b, header))
                .collect::<Result<Vec<_>>>()
        } else {
            Err(anyhow!("memfd target requires copy path"))
        };
        let (images, path, warning) = match imported {
            Ok(images) => (images, TargetPath::Direct, None),
            Err(error) => {
                // Failed attempts can leave GL's error flag set; consume a bounded batch.
                for _ in 0..8 {
                    if unsafe { device.gl.get_error() } == glow::NO_ERROR {
                        break;
                    }
                }
                for b in &mut buffers {
                    b.map(header)?;
                }
                (
                    Vec::new(),
                    TargetPath::Copy,
                    Some(format!("dma-buf import unavailable; copying: {error:#}")),
                )
            }
        };
        Ok(Self {
            device,
            header,
            buffers,
            images,
            path,
            warning,
            history: History::new(
                (header.physical_width, header.physical_height),
                usize::from(header.buffer_count),
            ),
            surface: None,
        })
    }
    pub fn path(&self) -> TargetPath {
        self.path
    }
    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }
    pub fn bind(&mut self, surface: SurfaceId) -> bool {
        if self.surface == Some(surface) {
            false
        } else {
            self.invalidate();
            self.surface = Some(surface);
            true
        }
    }
    pub fn invalidate(&mut self) {
        self.history = History::new(
            (self.header.physical_width, self.header.physical_height),
            self.buffers.len(),
        );
        self.surface = None;
    }
    pub fn write(&mut self, index: u32, prepared: Prepared) -> Result<TargetFrame> {
        let index = usize::try_from(index)?;
        ensure!(
            index < self.buffers.len(),
            "target buffer index outside swapchain"
        );
        self.history.record(&prepared.damage);
        let copied = if self.path == TargetPath::Direct {
            let gl = &self.device.gl;
            let w = i32::try_from(self.header.physical_width)?;
            let h = i32::try_from(self.header.physical_height)?;
            // SAFETY: source and imported destination belong to this current context;
            // reverse Y so shared memory row zero is the top physical row.
            unsafe {
                gl.disable(glow::SCISSOR_TEST);
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(prepared.fbo));
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, self.images[index].fbo);
                gl.blit_framebuffer(
                    0,
                    0,
                    w,
                    h,
                    0,
                    h,
                    w,
                    0,
                    glow::COLOR_BUFFER_BIT,
                    glow::NEAREST,
                );
                gl.finish();
                ensure!(
                    gl.get_error() == glow::NO_ERROR,
                    "direct target rendering failed"
                );
            }
            vec![DamageRect {
                x: 0,
                y: 0,
                width: self.header.physical_width,
                height: self.header.physical_height,
            }]
        } else {
            let rects = self.history.coverage(index);
            let mut access = self.buffers[index].begin()?;
            let gl = &self.device.gl;
            for r in &rects {
                ensure!(
                    r.x.checked_add(r.width)
                        .is_some_and(|x| x <= self.header.physical_width)
                        && r.y
                            .checked_add(r.height)
                            .is_some_and(|y| y <= self.header.physical_height),
                    "copy damage outside target"
                );
                let len = usize::try_from(u64::from(r.width) * u64::from(r.height) * 4)?;
                let mut pixels = vec![0; len];
                // SAFETY: bounded dimensions and output slice of exactly width*height*4;
                // read only regions needed to bring this particular buffer up to date.
                unsafe {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(prepared.fbo));
                    gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
                    gl.read_pixels(
                        i32::try_from(r.x)?,
                        i32::try_from(self.header.physical_height - r.y - r.height)?,
                        i32::try_from(r.width)?,
                        i32::try_from(r.height)?,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelPackData::Slice(Some(&mut pixels)),
                    );
                    ensure!(
                        gl.get_error() == glow::NO_ERROR,
                        "copy target readback failed"
                    );
                }
                for y in 0..r.height {
                    for x in 0..r.width {
                        let src = usize::try_from(((r.height - 1 - y) * r.width + x) * 4)?;
                        let dst = usize::try_from(
                            u64::from(r.y + y) * u64::from(self.header.pitch)
                                + u64::from(r.x + x) * 4,
                        )?;
                        access.write_pixel(
                            dst,
                            [pixels[src + 2], pixels[src + 1], pixels[src], 255],
                        )?;
                    }
                }
            }
            access.finish()?;
            rects
        };
        self.history.copied(index);
        Ok(TargetFrame {
            damage: prepared.damage,
            copied,
        })
    }
}
