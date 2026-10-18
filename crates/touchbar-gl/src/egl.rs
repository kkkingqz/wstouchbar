//! Owns the EGL ABI and enforces a current, thread-local GLES3 context.
#![allow(unsafe_code)]
use crate::Backend;
use anyhow::{Context, Result, ensure};
use glow::HasContext;
use khronos_egl as e;
use std::{collections::BTreeMap, sync::Mutex};
use std::{
    ffi::{CStr, c_char, c_void},
    marker::PhantomData,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::Path,
    ptr,
    rc::Rc,
};
// EGL displays are process-wide: repeated initialize does not add ownership.
static DISPLAYS: Mutex<BTreeMap<usize, DisplayLease>> = Mutex::new(BTreeMap::new());
struct DisplayLease {
    owners: usize,
    terminate: bool,
}
const DEVICE_PLATFORM: e::Enum = 0x313f;
const SURFACELESS_PLATFORM: e::Enum = 0x31dd;
const RENDER_NODE: e::Int = 0x3377;
type Device = *mut c_void;
type QueryDevices = unsafe extern "system" fn(e::Int, *mut Device, *mut e::Int) -> e::Boolean;
type QueryDeviceString = unsafe extern "system" fn(Device, e::Int) -> *const c_char;

pub(crate) struct Egl {
    api: e::DynamicInstance<e::EGL1_5>,
    display: e::Display,
    context: Option<e::Context>,
    surface: Option<e::Surface>,
    initialized: bool,
    _thread: PhantomData<Rc<()>>,
}
pub(crate) struct DeviceContext {
    pub owner: Egl,
    pub gl: glow::Context,
    pub description: String,
}
impl DeviceContext {
    pub fn new(backend: &Backend) -> Result<Self> {
        // SAFETY: libEGL exposes its stable ABI; its owner remains alive for all calls.
        let api =
            unsafe { e::DynamicInstance::<e::EGL1_5>::load_required() }.context("load libEGL")?;
        let node = match backend {
            Backend::Surfaceless => None,
            Backend::Auto => crate::device::discover(),
            Backend::RenderNode(path) => Some(path.clone()),
        };
        let device = node
            .as_deref()
            .map(|path| device_display(&api, path))
            .transpose()?
            .flatten();
        let (display, description) = if let Some(display) = device {
            (
                display,
                format!("EGL device {}", node.as_ref().unwrap().display()),
            )
        } else {
            // SAFETY: the surfaceless platform has no native display, attributes are terminated.
            let display = unsafe {
                api.get_platform_display(SURFACELESS_PLATFORM, ptr::null_mut(), &[e::ATTRIB_NONE])
            }
            .context("open surfaceless EGL display")?;
            (display, "EGL surfaceless".into())
        };
        let mut owner = Egl {
            api,
            display,
            context: None,
            surface: None,
            initialized: false,
            _thread: PhantomData,
        };
        {
            let mut displays = DISPLAYS.lock().unwrap_or_else(|poison| poison.into_inner());
            let key = display.as_ptr() as usize;
            let borrowed = !displays.contains_key(&key)
                && owner.api.query_string(Some(display), e::VERSION).is_ok();
            owner
                .api
                .initialize(display)
                .context("initialize EGL display")?;
            let lease = displays.entry(key).or_insert(DisplayLease {
                owners: 0,
                terminate: !borrowed,
            });
            lease.owners += 1;
            owner.initialized = true;
        }
        owner
            .api
            .bind_api(e::OPENGL_ES_API)
            .context("select GLES API")?;
        let config = owner
            .api
            .choose_first_config(
                display,
                &[
                    e::SURFACE_TYPE,
                    e::PBUFFER_BIT,
                    e::RENDERABLE_TYPE,
                    e::OPENGL_ES3_BIT,
                    e::RED_SIZE,
                    8,
                    e::GREEN_SIZE,
                    8,
                    e::BLUE_SIZE,
                    8,
                    e::ALPHA_SIZE,
                    8,
                    e::NONE,
                ],
            )
            .context("choose GLES3 EGL config")?
            .context("no GLES3 EGL config")?;
        owner.context = Some(
            owner
                .api
                .create_context(
                    display,
                    config,
                    None,
                    &[e::CONTEXT_CLIENT_VERSION, 3, e::NONE],
                )
                .context("create GLES3 context")?,
        );
        owner.surface = Some(
            owner
                .api
                .create_pbuffer_surface(display, config, &[e::WIDTH, 1, e::HEIGHT, 1, e::NONE])
                .context("create EGL pbuffer")?,
        );
        owner.current()?;
        // SAFETY: this context is current; resolved functions live with the EGL library.
        let gl = unsafe {
            glow::Context::from_loader_function(|name| {
                owner
                    .api
                    .get_proc_address(name)
                    .map_or(ptr::null(), |f| f as *const () as *const c_void)
            })
        };
        // SAFETY: querying GL strings needs only the current context.
        let version = unsafe { gl.get_parameter_string(glow::VERSION) };
        ensure!(
            version.starts_with("OpenGL ES 3"),
            "GLES3 required, received {version}"
        );
        let description = format!("{description}; {version}");
        Ok(Self {
            owner,
            gl,
            description,
        })
    }
}
impl Egl {
    pub fn current(&self) -> Result<()> {
        self.api
            .make_current(self.display, self.surface, self.surface, self.context)
            .context("make GLES context current")
    }
}
impl Egl {
    pub fn import_dma_buf(
        &self,
        fd: std::os::fd::BorrowedFd<'_>,
        h: touchbar_protocol::SwapchainHeader,
    ) -> Result<e::Image> {
        use std::os::fd::AsRawFd;
        let extensions = self
            .api
            .query_string(Some(self.display), e::EXTENSIONS)?
            .to_str()?;
        ensure!(
            extensions
                .split_whitespace()
                .any(|s| s == "EGL_EXT_image_dma_buf_import"),
            "EGL_EXT_image_dma_buf_import unavailable"
        );
        let attrs = [
            e::WIDTH as e::Attrib,
            h.physical_width as e::Attrib,
            e::HEIGHT as e::Attrib,
            h.physical_height as e::Attrib,
            0x3271,
            h.format as e::Attrib,
            0x3272,
            e::Attrib::try_from(fd.as_raw_fd())?,
            0x3273,
            0,
            0x3274,
            h.pitch as e::Attrib,
            e::ATTRIB_NONE,
        ];
        // SAFETY: dma-buf import explicitly requires EGL_NO_CONTEXT and NULL client buffer;
        // attrs supplies the single-plane XRGB layout and owned fd remains alive.
        self.api
            .create_image(
                self.display,
                unsafe { e::Context::from_ptr(ptr::null_mut()) },
                0x3270,
                unsafe { e::ClientBuffer::from_ptr(ptr::null_mut()) },
                &attrs,
            )
            .context("import dma-buf EGLImage")
    }
    pub fn destroy_dma_image(&self, image: e::Image) {
        let _ = self.api.destroy_image(self.display, image);
    }
    pub fn image_renderbuffer(&self) -> Result<unsafe extern "system" fn(u32, *mut c_void)> {
        let f = self
            .api
            .get_proc_address("glEGLImageTargetRenderbufferStorageOES")
            .context("GL_OES_EGL_image entrypoint unavailable")?;
        // SAFETY: this named EGL proc has the OES signature; API library outlives its use.
        Ok(unsafe {
            std::mem::transmute::<extern "system" fn(), unsafe extern "system" fn(u32, *mut c_void)>(
                f,
            )
        })
    }
}
impl Drop for Egl {
    fn drop(&mut self) {
        if !self.initialized {
            return;
        }
        // The owning DeviceContext outlives every renderer/imported target sharing it.
        let _ = self.api.make_current(self.display, None, None, None);
        if let Some(surface) = self.surface.take() {
            let _ = self.api.destroy_surface(self.display, surface);
        }
        if let Some(context) = self.context.take() {
            let _ = self.api.destroy_context(self.display, context);
        }
        let mut displays = DISPLAYS.lock().unwrap_or_else(|poison| poison.into_inner());
        let key = self.display.as_ptr() as usize;
        if let Some(lease) = displays.get_mut(&key) {
            lease.owners -= 1;
            if lease.owners == 0 {
                let terminate = lease.terminate;
                displays.remove(&key);
                if terminate {
                    let _ = self.api.terminate(self.display);
                }
            }
        }
    }
}
fn device_display(api: &e::DynamicInstance<e::EGL1_5>, path: &Path) -> Result<Option<e::Display>> {
    let metadata =
        std::fs::metadata(path).with_context(|| format!("stat render node {}", path.display()))?;
    ensure!(
        metadata.file_type().is_char_device(),
        "render node must be a character device"
    );
    let extensions = api
        .query_string(None, e::EXTENSIONS)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !extensions
        .split_whitespace()
        .any(|s| s == "EGL_EXT_platform_device")
    {
        return Ok(None);
    }
    let (Some(query), Some(string)) = (
        api.get_proc_address("eglQueryDevicesEXT"),
        api.get_proc_address("eglQueryDeviceStringEXT"),
    ) else {
        return Ok(None);
    };
    // SAFETY: EGL returned the entry points for these exact extension ABI signatures.
    let query = unsafe { std::mem::transmute::<unsafe extern "system" fn(), QueryDevices>(query) };
    // SAFETY: as above; a returned device string remains owned by EGL.
    let string =
        unsafe { std::mem::transmute::<unsafe extern "system" fn(), QueryDeviceString>(string) };
    let mut devices = [ptr::null_mut(); 16];
    let mut count = 0;
    // SAFETY: the output buffer has exactly the advertised capacity and count is writable.
    if unsafe { query(16, devices.as_mut_ptr(), &mut count) } != e::TRUE {
        return Ok(None);
    }
    let count = usize::try_from(count)
        .unwrap_or_default()
        .min(devices.len());
    for device in devices.into_iter().take(count) {
        // SAFETY: each device came from EGL; null is the no-property sentinel.
        let name = unsafe { string(device, RENDER_NODE) };
        if name.is_null() {
            continue;
        }
        // SAFETY: this non-null property is a library-owned NUL-terminated string.
        let name = unsafe { CStr::from_ptr(name) }.to_string_lossy();
        if std::fs::metadata(name.as_ref()).is_ok_and(|m| m.rdev() == metadata.rdev()) {
            // SAFETY: EGL_EXT_platform_device defines this device as the native display.
            return Ok(unsafe {
                api.get_platform_display(DEVICE_PLATFORM, device, &[e::ATTRIB_NONE])
            }
            .ok());
        }
    }
    Ok(None)
}
