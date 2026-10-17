//! Blocking sysfs drivers. H4 owns timing; the shell runs device IO on its worker.
mod autodim;
use crate::device::{Operations, Roots, SwitchPlan, discover};
pub use autodim::{Autodim, DriverSettings, PreparedAutodim};
use rustix::fs::{Mode, OFlags, open, openat};
use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    os::unix::fs::MetadataExt,
};
const NAMES: &[&str] = &[
    "appletb_backlight",
    "228200000.display-pipe.0",
    "228600000.dsi.0",
];
const DRIVERS: &[&str] = &["hid-appletb-bl", "panel-summit"];
fn invalid(s: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s)
}
fn attribute(directory: &File, name: &str, flags: OFlags) -> io::Result<File> {
    Ok(openat(
        directory,
        name,
        flags | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?
    .into())
}
/// Match v1 policy: T2 exposes off/dim/full directly, continuous panels use
/// the original 0/1/128 levels on a 255-scale. Never round full T2 down to dim.
pub fn level_value(max: u32, level: u8) -> io::Result<u32> {
    if max == 0 || level > 2 {
        return Err(invalid("invalid backlight range or level"));
    }
    if max <= 2 {
        return Ok(match level {
            0 => 0,
            1 => 1,
            _ => max,
        });
    }
    let policy = match level {
        0 => return Ok(0),
        1 => 1u64,
        _ => 128,
    };
    u32::try_from(((policy * u64::from(max) + 127) / 255).clamp(1, u64::from(max)))
        .map_err(|_| invalid("backlight value overflow"))
}
pub struct Backlight {
    file: File,
    max: u32,
}
impl Backlight {
    pub fn open(roots: &Roots) -> io::Result<Self> {
        for name in NAMES {
            let path = roots.sys.join("class/backlight").join(name);
            let path = match path.canonicalize() {
                Ok(p) => p,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            if !path.starts_with(roots.sys.join("devices")) {
                return Err(invalid("backlight outside sysfs devices"));
            }
            // The T2 backlight controller is a separate USB device (05ac:8102),
            // not a child of display/keyboard USB 05ac:8302. Pin its own directory.
            let directory = File::from(open(
                &path,
                OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?);
            let driver = match path.join("device/driver").canonicalize() {
                Ok(p) => p,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            if !driver
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| DRIVERS.contains(&n))
            {
                continue;
            }
            let pinned = directory.metadata()?;
            let current = fs::metadata(&path)?;
            if pinned.dev() != current.dev() || pinned.ino() != current.ino() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "backlight replaced during discovery",
                ));
            }
            let mut source = String::new();
            attribute(&directory, "max_brightness", OFlags::RDONLY)?
                .take(32)
                .read_to_string(&mut source)?;
            let max = source
                .trim()
                .parse()
                .map_err(|_| invalid("invalid max_brightness"))?;
            level_value(max, 2)?;
            return Ok(Self {
                file: attribute(&directory, "brightness", OFlags::WRONLY)?,
                max,
            });
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no supported Touch Bar backlight",
        ))
    }
    pub fn set_level(&mut self, level: u8) -> io::Result<()> {
        let value = level_value(self.max, level)?;
        self.file.seek(SeekFrom::Start(0))?;
        // Format once: some kernel handlers reject a value/newline split across writes.
        self.file.write_all(format!("{value}\n").as_bytes())?;
        Ok(())
    }
}
/// Crash fallback. Always return USB to native, including when autodim was
/// disabled before startup. Marker ownership alone authorizes changing dimming.
/// Caller must hold ExclusiveDevice and own/close its DRM and evdev handles.
pub fn hand_back_with(roots: &Roots, ops: &mut impl Operations) -> io::Result<()> {
    let native = match discover(roots) {
        Ok(Some(s)) if !s.native_safe() => {
            SwitchPlan::new(s.device, touchbar_protocol::Mode::Native).execute(ops)
        }
        Ok(_) => Ok(()),
        Err(e) => Err(e),
    };
    let dimming = Autodim::new(roots).restore();
    let light = if matches!(dimming, Ok(true)) {
        match discover(roots)? {
            Some(s) if s.native_safe() => match Backlight::open(roots) {
                Ok(mut b) => b.set_level(2),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            },
            _ => Ok(()),
        }
    } else {
        Ok(())
    };
    // Attempt independent cleanup even when another part failed.
    native.and(dimming.map(|_| ())).and(light)
}
/// Standalone ExecStopPost entry. Heavy USB operations remain off the event loop;
/// the worker holds the lock for their entire lifetime, including after caller drop.
pub async fn hand_back(roots: Roots) -> io::Result<()> {
    let mut worker = crate::worker::Worker::new(move |()| {
        let _guard = crate::device::ExclusiveDevice::acquire(&roots)?;
        hand_back_with(&roots, &mut crate::device::SystemOps { release: || Ok(()) })
    })?;
    worker
        .try_submit(1, ())
        .map_err(|e| io::Error::other(format!("hand-back queue: {e:?}")))?;
    worker
        .next()
        .await
        .map_err(|_| io::Error::other("hand-back worker stopped"))?
        .value
}
