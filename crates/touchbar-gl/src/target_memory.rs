//! Owned mapping and dma-buf CPU-access boundary. No pointers escape this module.
#![allow(unsafe_code)]
use anyhow::{Context, Result, bail, ensure};
use std::{
    io,
    os::fd::{AsRawFd, OwnedFd},
    ptr::NonNull,
};
use touchbar_protocol::SwapchainHeader;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Memfd,
    DmaBuf,
}
struct Mapping {
    address: NonNull<u8>,
    len: usize,
}
impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: this exact nonempty mapping is uniquely owned until Drop.
        unsafe {
            libc::munmap(self.address.as_ptr().cast(), self.len);
        }
    }
}
pub(crate) struct Buffer {
    pub fd: OwnedFd,
    pub kind: Kind,
    map: Option<Mapping>,
}
impl Buffer {
    pub fn new(fd: OwnedFd, header: SwapchainHeader) -> Result<Self> {
        let kind = match rustix::fs::fcntl_get_seals(&fd) {
            Ok(seals) => {
                ensure!(
                    seals.contains(rustix::fs::SealFlags::SHRINK | rustix::fs::SealFlags::GROW),
                    "memfd must be sealed against resizing"
                );
                Kind::Memfd
            }
            Err(_) => {
                sync(&fd, 2).context("expected dma-buf or sealed memfd")?;
                sync(&fd, 2 | 4)?;
                Kind::DmaBuf
            }
        };
        let size = rustix::fs::seek(&fd, rustix::fs::SeekFrom::End(0))
            .context("query shared buffer size")?;
        ensure!(
            size >= header.buffer_size,
            "shared buffer is shorter than swapchain"
        );
        Ok(Self {
            fd,
            kind,
            map: None,
        })
    }
    pub fn map(&mut self, header: SwapchainHeader) -> Result<()> {
        let len = usize::try_from(header.buffer_size)?;
        ensure!(
            len > 0 && len <= isize::MAX as usize,
            "invalid mapping size"
        );
        // SAFETY: validated descriptor extent; sealed memfd or fixed-size dma-buf prevents
        // shrink/SIGBUS. Shared read/write mapping is retained with its owning fd.
        let address = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                self.fd.as_raw_fd(),
                0,
            )
        };
        if address == libc::MAP_FAILED {
            return Err(io::Error::last_os_error()).context("map shared render target");
        }
        let Some(address) = NonNull::new(address.cast::<u8>()) else {
            // SAFETY: an unusual mapping at address zero is still a valid mapping to release.
            unsafe {
                libc::munmap(address, len);
            }
            bail!("shared mapping returned address zero");
        };
        self.map = Some(Mapping { address, len });
        Ok(())
    }
    pub fn begin(&mut self) -> Result<Access<'_>> {
        let map = self.map.as_mut().context("copy target has no mapping")?;
        if self.kind == Kind::DmaBuf {
            sync(&self.fd, 2)?;
        }
        Ok(Access {
            fd: &self.fd,
            kind: self.kind,
            map,
            ended: false,
        })
    }
}
pub(crate) struct Access<'a> {
    fd: &'a OwnedFd,
    kind: Kind,
    map: &'a mut Mapping,
    ended: bool,
}
impl Access<'_> {
    pub fn write_pixel(&mut self, offset: usize, pixel: [u8; 4]) -> Result<()> {
        ensure!(
            offset.is_multiple_of(4)
                && offset.checked_add(4).is_some_and(|end| end <= self.map.len),
            "pixel write outside aligned mapping"
        );
        // SAFETY: mmap is page-aligned, offset and extent were checked. Volatile IO
        // writes exactly one XRGB pixel without creating Rust references to externally
        // shared memory. CPU access is bracketed by dma-buf synchronization.
        unsafe {
            self.map
                .address
                .as_ptr()
                .add(offset)
                .cast::<u32>()
                .write_volatile(u32::from_ne_bytes(pixel));
        }
        Ok(())
    }
    pub fn finish(mut self) -> Result<()> {
        if self.kind == Kind::DmaBuf {
            sync(self.fd, 2 | 4)?;
        }
        self.ended = true;
        Ok(())
    }
}
impl Drop for Access<'_> {
    fn drop(&mut self) {
        if !self.ended && self.kind == Kind::DmaBuf {
            let _ = sync(self.fd, 2 | 4);
        }
    }
}
fn sync(fd: &OwnedFd, flags: u64) -> Result<()> {
    use rustix::ioctl::{self, Setter, opcode};
    loop {
        // SAFETY: Linux dma_buf_sync is one initialized u64, read only by this ioctl;
        // opcode encoding is selected by rustix for the target Linux architecture.
        match unsafe {
            ioctl::ioctl(
                fd,
                Setter::<{ opcode::write::<u64>(b'b', 0) }, _>::new(flags),
            )
        } {
            Ok(()) => return Ok(()),
            Err(rustix::io::Errno::INTR | rustix::io::Errno::AGAIN) => continue,
            Err(e) => return Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::File, os::unix::fs::FileExt};
    fn header() -> SwapchainHeader {
        SwapchainHeader {
            logical_width: 8,
            logical_height: 4,
            physical_width: 8,
            physical_height: 4,
            format: touchbar_protocol::XRGB8888,
            pitch: 32,
            buffer_size: 128,
            buffer_count: 3,
            frame_damage: true,
        }
    }
    fn file(size: u64, sealed: bool) -> File {
        let file = File::from(
            rustix::fs::memfd_create(
                "target-memory-test",
                rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
            )
            .unwrap(),
        );
        file.set_len(size).unwrap();
        if sealed {
            rustix::fs::fcntl_add_seals(
                &file,
                rustix::fs::SealFlags::GROW | rustix::fs::SealFlags::SHRINK,
            )
            .unwrap();
        }
        file
    }
    #[test]
    fn mutable_short_and_readonly_memfds_do_not_create_writable_targets() {
        let h = header();
        assert!(Buffer::new(file(128, false).into(), h).is_err());
        assert!(Buffer::new(file(127, true).into(), h).is_err());
        let f = file(128, true);
        let readonly = File::open(format!("/proc/self/fd/{}", f.as_raw_fd())).unwrap();
        let mut b = Buffer::new(readonly.into(), h).unwrap();
        assert!(b.map(h).is_err());
    }
    #[test]
    fn sealed_mapping_writes_shared_bytes_inside_a_cpu_access_guard() {
        let h = header();
        let f = file(128, true);
        let mut b = Buffer::new(f.try_clone().unwrap().into(), h).unwrap();
        b.map(h).unwrap();
        let mut access = b.begin().unwrap();
        access.write_pixel(0, [9, 8, 7, 255]).unwrap();
        access.finish().unwrap();
        let mut bytes = [0; 8];
        f.read_exact_at(&mut bytes, 0).unwrap();
        assert_eq!(bytes, [9, 8, 7, 255, 0, 0, 0, 0]);
    }
    #[test]
    fn pixel_access_rejects_unaligned_or_out_of_range_writes() {
        let h = header();
        let mut b = Buffer::new(file(128, true).into(), h).unwrap();
        b.map(h).unwrap();
        let mut access = b.begin().unwrap();
        assert!(access.write_pixel(1, [1; 4]).is_err());
        assert!(access.write_pixel(128, [1; 4]).is_err());
        access.finish().unwrap();
    }
}
