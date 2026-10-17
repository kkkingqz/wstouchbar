//! Initialized Linux evdev/uinput ABI boundary. No arbitrary input escapes it.
#![allow(unsafe_code)]
use super::{Capabilities, InputKind, KeyBitmap, KeyQueries, SYSTEM_KEYS, SystemKeys, UINPUT_NAME};
use rustix::{
    fs::{Mode, OFlags},
    ioctl::{self, IntegerSetter, NoArg, Opcode, Setter, Updater, opcode},
};
use std::{
    fs::File,
    io,
    os::fd::{AsFd, BorrowedFd},
    path::Path,
};
use touchbar_model::{KeyPhase, SystemKey};
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn read_bytes<const REQUEST: Opcode, const N: usize>(fd: BorrowedFd<'_>) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    // SAFETY: instantiated only with matching read ioctl/array lengths below. All
    // bytes are initialized, including tails that variable-length ioctls don't fill.
    unsafe {
        ioctl::ioctl(fd, Updater::<REQUEST, _>::new(&mut bytes))?;
    }
    Ok(bytes)
}
pub(crate) fn name(fd: BorrowedFd<'_>) -> io::Result<String> {
    let b = read_bytes::<{ opcode::read::<[u8; 256]>(b'E', 0x06) }, 256>(fd)?;
    terminated(&b)
}
fn terminated(b: &[u8]) -> io::Result<String> {
    let end = b.iter().position(|b| *b == 0).unwrap_or(b.len());
    std::str::from_utf8(&b[..end])
        .map(str::to_owned)
        .map_err(|_| invalid("invalid kernel input name"))
}
pub fn classify_input(sys: &Path, canonical: &Path, name: &str) -> io::Result<InputKind> {
    let devices = sys.join("devices");
    if !canonical.starts_with(&devices)
        || canonical.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(invalid("input outside canonical sysfs devices"));
    }
    if canonical.starts_with(devices.join("virtual")) {
        Ok(if name == UINPUT_NAME {
            InputKind::Own
        } else {
            InputKind::Virtual
        })
    } else {
        Ok(InputKind::Physical)
    }
}
pub(crate) fn capabilities(fd: BorrowedFd<'_>) -> io::Result<Option<Capabilities>> {
    let b = read_bytes::<{ opcode::read::<[u8; 4]>(b'E', 0x20) }, 4>(fd)?;
    let types = u32::from_ne_bytes(b);
    let lid = if types & (1 << 5) != 0 {
        read_bytes::<{ opcode::read::<[u8; 8]>(b'E', 0x25) }, 8>(fd)?[0] & 1 != 0
    } else {
        false
    };
    Ok(
        (types & ((1 << 1) | (1 << 2) | (1 << 3)) != 0 || lid).then_some(Capabilities {
            keys: types & (1 << 1) != 0,
            lid,
        }),
    )
}
#[derive(Clone, Copy)]
pub struct LinuxKeyQueries;
impl KeyQueries for LinuxKeyQueries {
    fn query(
        &self,
        fd: BorrowedFd<'_>,
        caps: Capabilities,
    ) -> io::Result<(KeyBitmap, Option<bool>)> {
        let keys = if caps.keys {
            KeyBitmap(read_bytes::<{ opcode::read::<[u8; 96]>(b'E', 0x18) }, 96>(
                fd,
            )?)
        } else {
            KeyBitmap::default()
        };
        let lid = if caps.lid {
            Some(read_bytes::<{ opcode::read::<[u8; 8]>(b'E', 0x1b) }, 8>(fd)?[0] & 1 != 0)
        } else {
            None
        };
        Ok((keys, lid))
    }
}
#[repr(C)]
struct Setup {
    id: [u16; 4],
    name: [u8; 80],
    ff_effects_max: u32,
}
pub struct Uinput {
    keys: SystemKeys<File>,
    sysname: String,
}
impl Uinput {
    pub fn open(path: &Path) -> io::Result<Self> {
        let fd = rustix::fs::open(
            path,
            OFlags::WRONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            Mode::empty(),
        )?;
        // SAFETY: UI_SET_EVBIT/KEYBIT use integer arguments, not pointers. The closed
        // code table bounds every key; EV_REP enables kernel autorepeat.
        unsafe {
            for event in [1, 0x14] {
                ioctl::ioctl(
                    &fd,
                    IntegerSetter::<{ opcode::write::<i32>(b'U', 100) }>::new_usize(event),
                )?;
            }
            for &(_, code) in SYSTEM_KEYS {
                ioctl::ioctl(
                    &fd,
                    IntegerSetter::<{ opcode::write::<i32>(b'U', 101) }>::new_usize(usize::from(
                        code,
                    )),
                )?;
            }
            let mut setup = Setup {
                id: [0x19, 0x1209, 0x5442, 1],
                name: [0; 80],
                ff_effects_max: 0,
            };
            setup.name[..UINPUT_NAME.len()].copy_from_slice(UINPUT_NAME.as_bytes());
            // Setup is exactly struct uinput_setup: input_id(8), name(80), u32(4), no padding.
            ioctl::ioctl(
                &fd,
                Setter::<{ opcode::write::<Setup>(b'U', 3) }, _>::new(setup),
            )?;
            ioctl::ioctl(&fd, NoArg::<{ opcode::none(b'U', 1) }>::new())?;
        }
        let b = read_bytes::<{ opcode::read::<[u8; 256]>(b'U', 44) }, 256>(fd.as_fd())?;
        let sysname = terminated(&b)?;
        if !sysname
            .strip_prefix("input")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(invalid("invalid uinput sysname"));
        }
        Ok(Self {
            keys: SystemKeys::from_writer(fd.into()),
            sysname,
        })
    }
    pub fn sysname(&self) -> io::Result<String> {
        Ok(self.sysname.clone())
    }
    pub fn send(&mut self, key: SystemKey, phase: KeyPhase) -> io::Result<()> {
        self.keys.send(key, phase)
    }
    pub fn release_all(&mut self) -> io::Result<()> {
        self.keys.release_all()
    }
}
