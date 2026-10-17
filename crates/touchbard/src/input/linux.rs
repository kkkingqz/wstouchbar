//! Small Linux input ioctl boundary; all output buffers are initialized.
#![allow(unsafe_code)]
use super::{Axis, AxisTransform, Geometry, MAX_SLOTS, RawSlot, SnapshotSource, TouchReader};
use crate::device::{DIGITIZER_NAME, Device, Roots};
use rustix::{
    fs::{Mode, OFlags},
    ioctl::{self, IntegerSetter, Opcode, Updater, opcode},
};
use std::{
    fs::File,
    io,
    os::{
        fd::{AsFd, BorrowedFd},
        unix::fs::{FileTypeExt, MetadataExt},
    },
    path::Path,
};
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AbsInfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn abs<const REQUEST: Opcode>(fd: BorrowedFd<'_>) -> io::Result<AbsInfo> {
    let mut info = AbsInfo::default();
    // SAFETY: only EVIOCGABS requests below instantiate this helper. AbsInfo is exactly
    // Linux input_absinfo (six i32 fields), initialized, live and writable for the call.
    unsafe {
        ioctl::ioctl(fd, Updater::<REQUEST, AbsInfo>::new(&mut info))?;
    }
    Ok(info)
}
fn slot_axis(fd: BorrowedFd<'_>) -> io::Result<AbsInfo> {
    abs::<{ opcode::read::<AbsInfo>(b'E', 0x6f) }>(fd)
}
fn slots(fd: BorrowedFd<'_>, axis: i32) -> io::Result<[i32; MAX_SLOTS + 1]> {
    let mut values = [0; MAX_SLOTS + 1];
    values[0] = axis;
    // SAFETY: EVIOCGMTSLOTS is _IOC_READ with a buffer whose first i32 names the axis.
    // Remaining i32s hold at most MAX_SLOTS values. The whole array is initialized;
    // kernels that write fewer slots leave initialized unused elements, never uninit data.
    unsafe {
        ioctl::ioctl(
            fd,
            Updater::<{ opcode::read::<[i32; MAX_SLOTS + 1]>(b'E', 0x0a) }, _>::new(&mut values),
        )?;
    }
    Ok(values)
}
fn name(fd: BorrowedFd<'_>) -> io::Result<String> {
    let mut bytes = [0u8; 256];
    // SAFETY: EVIOCGNAME(256) writes up to 256 bytes; the initialized array is large
    // enough and remains live. Updater avoids assuming all 256 bytes were written.
    unsafe {
        ioctl::ioctl(
            fd,
            Updater::<{ opcode::read::<[u8; 256]>(b'E', 0x06) }, _>::new(&mut bytes),
        )?;
    }
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end])
        .map(str::to_owned)
        .map_err(|_| invalid("invalid evdev name"))
}
fn grab(fd: BorrowedFd<'_>) -> io::Result<()> {
    // SAFETY: EVIOCGRAB is _IOW('E', 0x90, int) but takes the integer value itself,
    // not a pointer to it. Value 1 requests exclusive ownership; closing fd releases it.
    unsafe {
        ioctl::ioctl(
            fd,
            IntegerSetter::<{ opcode::write::<i32>(b'E', 0x90) }>::new_usize(1),
        )?;
    }
    Ok(())
}
#[derive(Clone, Copy, Default)]
pub struct LinuxQueries;
fn snapshot_with(
    count: usize,
    mut query: impl FnMut(i32) -> io::Result<[i32; MAX_SLOTS + 1]>,
    mut current_slot: impl FnMut() -> io::Result<i32>,
) -> io::Result<(Vec<RawSlot>, usize)> {
    if count == 0 || count > MAX_SLOTS {
        return Err(invalid("unsupported multitouch slot count"));
    }
    // EVIOCGMTSLOTS requests are separate ioctls. Do not associate coordinates with a
    // finger that has already been replaced while the queries were in progress.
    for _ in 0..3 {
        let ids = query(0x39)?;
        let x = query(0x35)?;
        let y = query(0x36)?;
        let current =
            usize::try_from(current_slot()?).map_err(|_| invalid("negative current slot"))?;
        if current >= count {
            return Err(invalid("current slot outside range"));
        }
        let after = query(0x39)?;
        if ids[1..=count] != after[1..=count] {
            continue;
        }
        let mut result = Vec::with_capacity(count);
        for i in 1..=count {
            let tracking = match ids[i] {
                -1 => None,
                id if id >= 0 => Some(id.unsigned_abs()),
                _ => return Err(invalid("invalid kernel tracking ID")),
            };
            result.push(RawSlot {
                tracking,
                x: x[i],
                y: y[i],
            });
        }
        return Ok((result, current));
    }
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "multitouch tracking changed during recovery",
    ))
}
impl SnapshotSource for LinuxQueries {
    fn snapshot(&self, fd: BorrowedFd<'_>, count: usize) -> io::Result<(Vec<RawSlot>, usize)> {
        snapshot_with(count, |axis| slots(fd, axis), || Ok(slot_axis(fd)?.value))
    }
}

pub type TouchInput = TouchReader<LinuxQueries>;
impl TouchInput {
    /// The path must come from H5 discovery. Recheck the opened char device's identity
    /// and sysfs ancestry before EVIOCGRAB, so a recycled event number cannot grab a keyboard.
    pub fn open(
        roots: &Roots,
        device: &Device,
        path: &Path,
        width: u32,
        height: u32,
    ) -> io::Result<Self> {
        let fd = rustix::fs::open(
            path,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )?;
        let file = File::from(fd);
        let metadata = file.metadata()?;
        if !metadata.file_type().is_char_device() {
            return Err(invalid("touch input is not a character device"));
        }
        if name(file.as_fd())? != DIGITIZER_NAME {
            return Err(invalid("evdev node is not the Touch Bar digitizer"));
        }
        let sys = roots
            .sys
            .join(format!(
                "dev/char/{}:{}",
                rustix::fs::major(metadata.rdev()),
                rustix::fs::minor(metadata.rdev())
            ))
            .canonicalize()?;
        if !sys.starts_with(device.path()) {
            return Err(invalid(
                "digitizer does not belong to the selected USB device",
            ));
        }
        grab(file.as_fd())?;
        let x = abs::<{ opcode::read::<AbsInfo>(b'E', 0x75) }>(file.as_fd())?;
        let y = abs::<{ opcode::read::<AbsInfo>(b'E', 0x76) }>(file.as_fd())?;
        let slot = slot_axis(file.as_fd())?;
        if slot.minimum != 0 || slot.maximum < 0 {
            return Err(invalid("invalid protocol-B slot range"));
        }
        let count = usize::try_from(i64::from(slot.maximum) + 1)
            .map_err(|_| invalid("multitouch slot count overflow"))?;
        // Intel T2 input axes already run along the logical horizontal bar, independently
        // of the portrait DRM scanout. Preserve v1's mapping rather than swapping by DRM.
        let geometry = Geometry::new(
            width,
            height,
            Axis {
                minimum: x.minimum,
                maximum: x.maximum,
            },
            Axis {
                minimum: y.minimum,
                maximum: y.maximum,
            },
            AxisTransform::Identity,
        )?;
        Self::from_fd(file.into(), geometry, count, LinuxQueries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    fn values(axis: i32, id: i32) -> [i32; MAX_SLOTS + 1] {
        let mut a = [-1; MAX_SLOTS + 1];
        a[0] = axis;
        a[1] = id;
        a
    }
    #[test]
    fn recovery_retries_when_tracking_changes_between_axis_queries() {
        let mut ids = VecDeque::from([0, 1, 1, 1]);
        let (slots, current) = snapshot_with(
            1,
            |axis| {
                Ok(values(
                    axis,
                    if axis == 0x39 {
                        ids.pop_front().unwrap()
                    } else {
                        100
                    },
                ))
            },
            || Ok(0),
        )
        .unwrap();
        assert_eq!(slots[0].tracking, Some(1));
        assert_eq!(current, 0);
        assert!(ids.is_empty());
        let mut id = 0;
        assert!(
            snapshot_with(
                1,
                |axis| {
                    if axis == 0x39 {
                        id ^= 1;
                    }
                    Ok(values(axis, id))
                },
                || Ok(0)
            )
            .is_err()
        );
    }
}
