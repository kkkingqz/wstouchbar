//! Test-only GBM allocation and dma-buf readback; never opens a DRM primary/card node.
#![allow(unsafe_code)]
use anyhow::{Context, Result, ensure};
use libloading::Library;
use std::{
    ffi::c_void,
    fs::{File, OpenOptions},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::Path,
    ptr,
};
use touchbar_protocol::{SwapchainHeader, XRGB8888};
type Create = unsafe extern "C" fn(i32) -> *mut c_void;
type Destroy = unsafe extern "C" fn(*mut c_void);
type CreateBo = unsafe extern "C" fn(*mut c_void, u32, u32, u32, u32) -> *mut c_void;
type Stride = unsafe extern "C" fn(*mut c_void) -> u32;
type Export = unsafe extern "C" fn(*mut c_void) -> i32;
type Modifier = unsafe extern "C" fn(*mut c_void) -> u64;
struct Api {
    create: Create,
    destroy: Destroy,
    bo: CreateBo,
    destroy_bo: Destroy,
    stride: Stride,
    export: Export,
    modifier: Modifier,
}
pub struct Gbm {
    api: Api,
    device: *mut c_void,
    bos: Vec<*mut c_void>,
    _file: File,
    _library: Library,
}
impl Gbm {
    pub fn new(path: &Path, w: u32, h: u32) -> Result<Self> {
        ensure!(
            path.file_name().and_then(|n| n.to_str()).is_some_and(|s| s
                .strip_prefix("renderD")
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))),
            "GBM test accepts render nodes only"
        );
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        // SAFETY: libgbm's stable C ABI; copied symbols remain backed by the owned library.
        let library = unsafe { Library::new("libgbm.so.1") }?;
        unsafe fn symbol<T: Copy>(l: &Library, name: &[u8]) -> Result<T> {
            Ok(*unsafe { l.get::<T>(name) }?)
        }
        let api = unsafe {
            Api {
                create: symbol(&library, b"gbm_create_device\0")?,
                destroy: symbol(&library, b"gbm_device_destroy\0")?,
                bo: symbol(&library, b"gbm_bo_create\0")?,
                destroy_bo: symbol(&library, b"gbm_bo_destroy\0")?,
                stride: symbol(&library, b"gbm_bo_get_stride\0")?,
                export: symbol(&library, b"gbm_bo_get_fd\0")?,
                modifier: symbol(&library, b"gbm_bo_get_modifier\0")?,
            }
        };
        // SAFETY: owned render-node fd outlives this device and all its BOs.
        let device = unsafe { (api.create)(file.as_raw_fd()) };
        ensure!(!device.is_null(), "gbm_create_device failed");
        let mut fixture = Self {
            api,
            device,
            bos: Vec::new(),
            _file: file,
            _library: library,
        };
        for _ in 0..3 {
            // gbm.h: RENDERING=1<<2, LINEAR=1<<4; no scanout/master/modeset operations.
            let bo = unsafe { (fixture.api.bo)(device, w, h, XRGB8888, (1 << 2) | (1 << 4)) };
            ensure!(!bo.is_null(), "linear rendering BO unavailable");
            fixture.bos.push(bo);
            let modifier = unsafe { (fixture.api.modifier)(bo) };
            ensure!(
                modifier == 0 || modifier == u64::MAX,
                "test BO is not linear"
            );
        }
        Ok(fixture)
    }
    pub fn export(
        &self,
        logical: (u32, u32),
        physical: (u32, u32),
    ) -> Result<(SwapchainHeader, Vec<OwnedFd>)> {
        // SAFETY: these are live BOs owned by the fixture, export returns a fresh fd each time.
        let pitch = unsafe { (self.api.stride)(self.bos[0]) };
        let mut fds = Vec::new();
        for &bo in &self.bos {
            ensure!(
                unsafe { (self.api.stride)(bo) } == pitch,
                "inconsistent BO pitches"
            );
            let fd = unsafe { (self.api.export)(bo) };
            ensure!(fd >= 0, "gbm_bo_get_fd failed");
            fds.push(unsafe { OwnedFd::from_raw_fd(fd) });
        }
        let header = SwapchainHeader {
            logical_width: logical.0,
            logical_height: logical.1,
            physical_width: physical.0,
            physical_height: physical.1,
            format: XRGB8888,
            pitch,
            buffer_size: u64::from(pitch) * u64::from(physical.1),
            buffer_count: 3,
            frame_damage: true,
        };
        header.validate()?;
        Ok((header, fds))
    }
}
impl Drop for Gbm {
    fn drop(&mut self) {
        // SAFETY: BOs are destroyed before their device, fd and library; none are borrowed here.
        unsafe {
            for bo in self.bos.drain(..) {
                (self.api.destroy_bo)(bo);
            }
            (self.api.destroy)(self.device);
        }
    }
}
pub fn read(fd: &OwnedFd, len: u64) -> Result<Vec<u8>> {
    let len = usize::try_from(len)?;
    ensure!(
        len > 0 && len <= isize::MAX as usize,
        "invalid readback extent"
    );
    use rustix::ioctl::{self, Setter, opcode};
    // SAFETY: dma_buf_sync is one initialized u64, for a live exported BO.
    unsafe {
        ioctl::ioctl(
            fd,
            Setter::<{ opcode::write::<u64>(b'b', 0) }, _>::new(1u64),
        )
    }?;
    let address = unsafe {
        libc::mmap(
            ptr::null_mut(),
            len,
            libc::PROT_READ,
            libc::MAP_SHARED,
            fd.as_raw_fd(),
            0,
        )
    };
    let result = if address == libc::MAP_FAILED {
        Err(std::io::Error::last_os_error()).context("map DMA readback")
    } else if address.is_null() {
        unsafe {
            libc::munmap(address, len);
        }
        Err(anyhow::anyhow!("readback mapping at address zero"))
    } else {
        // SAFETY: mapped extent is bounded by the caller's validated buffer size; copy before unmap.
        let bytes = unsafe { std::slice::from_raw_parts(address.cast::<u8>(), len).to_vec() };
        unsafe {
            libc::munmap(address, len);
        }
        Ok(bytes)
    };
    unsafe {
        ioctl::ioctl(
            fd,
            Setter::<{ opcode::write::<u64>(b'b', 0) }, _>::new(1u64 | 4),
        )
    }?;
    result
}
