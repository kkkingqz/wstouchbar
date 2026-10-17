//! Linux device discovery and configuration operations. No polling or implicit sleeps.
//! Blocking switch operations must run on H6's device worker, never the main event loop.
use rustix::fs::{Mode as FsMode, OFlags, fstat, open, openat};
use std::{
    fs::{self, File},
    io::{self, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
use touchbar_protocol::Mode;
pub const DIGITIZER_NAME: &str = "Apple Inc. Touch Bar Display Touchpad";
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roots {
    pub sys: PathBuf,
    pub dev: PathBuf,
    pub run: PathBuf,
}
impl Default for Roots {
    fn default() -> Self {
        Self {
            sys: "/sys".into(),
            dev: "/dev".into(),
            run: "/run/touchbar".into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    path: PathBuf,
    dev: u64,
    inode: u64,
}
impl Device {
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Pins and checks the USB directory before opening the attribute relative to it.
    pub fn write_config(&self, value: u8) -> io::Result<()> {
        if value > 2 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported USB configuration",
            ));
        }
        let fd = open(
            &self.path,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            FsMode::empty(),
        )?;
        let metadata = fstat(&fd)?;
        if metadata.st_dev != self.dev || metadata.st_ino != self.inode {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "USB device was replaced",
            ));
        }
        let attr = openat(
            &fd,
            "bConfigurationValue",
            OFlags::WRONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            FsMode::empty(),
        )?;
        let mut file = File::from(attr);
        file.write_all(format!("{value}\n").as_bytes())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayNodes {
    pub card: PathBuf,
    pub digitizer: PathBuf,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeNode {
    pub path: PathBuf,
    pub mode: u8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub device: Device,
    pub config: Option<u8>,
    pub display: Option<DisplayNodes>,
    pub native: Option<NativeNode>,
    pub drm_bound: bool,
}
impl Snapshot {
    pub fn native_safe(&self) -> bool {
        self.config == Some(1) && !self.drm_bound
    }
}
fn children(path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = match fs::read_dir(path) {
        Ok(entries) => entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<io::Result<Vec<_>>>()?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    paths.sort();
    Ok(paths)
}
fn canonical(path: &Path) -> io::Result<Option<PathBuf>> {
    match path.canonicalize() {
        Ok(path) => Ok(Some(path)),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}
fn read_optional(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(Some(value.trim().into())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
fn under(path: &Path, device: &Device) -> io::Result<bool> {
    Ok(canonical(path)?.is_some_and(|path| path.starts_with(&device.path)))
}
fn numbered(path: &Path, prefix: &str) -> bool {
    path.file_name()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_prefix(prefix))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}
fn select(roots: &Roots) -> io::Result<Option<Device>> {
    let root = roots.sys.canonicalize()?;
    for path in children(&roots.sys.join("bus/usb/devices"))? {
        let Some(path) = canonical(&path)? else {
            continue;
        };
        if !path.starts_with(&root) {
            continue;
        }
        let matches = read_optional(&path.join("idVendor"))?
            .is_some_and(|s| u16::from_str_radix(&s, 16) == Ok(0x05ac))
            && read_optional(&path.join("idProduct"))?
                .is_some_and(|s| u16::from_str_radix(&s, 16) == Ok(0x8302));
        if matches {
            let metadata = match fs::metadata(&path) {
                Ok(m) => m,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            return Ok(Some(Device {
                path,
                dev: metadata.dev(),
                inode: metadata.ino(),
            }));
        }
    }
    Ok(None)
}
pub fn discover(roots: &Roots) -> io::Result<Option<Snapshot>> {
    let Some(device) = select(roots)? else {
        return Ok(None);
    };
    let config = read_optional(&device.path.join("bConfigurationValue"))?
        .map(|s| {
            s.parse::<u8>().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid USB configuration value",
                )
            })
        })
        .transpose()?;
    let mut card = None;
    let mut drm_bound = false;
    for path in children(&roots.sys.join("class/drm"))? {
        if !numbered(&path, "card") || !under(&path, &device)? {
            continue;
        }
        if canonical(&path.join("device/driver"))?
            .is_some_and(|p| p.file_name().is_some_and(|s| s == "appletbdrm"))
        {
            drm_bound = true;
            let dev = roots
                .dev
                .join("dri")
                .join(path.file_name().expect("named card"));
            if dev.exists() {
                card = Some(dev);
                break;
            }
        }
    }
    let mut digitizer = None;
    for path in children(&roots.sys.join("class/input"))? {
        if !numbered(&path, "input")
            || !under(&path, &device)?
            || read_optional(&path.join("name"))?.as_deref() != Some(DIGITIZER_NAME)
        {
            continue;
        }
        for event in children(&path)? {
            if numbered(&event, "event") {
                let dev = roots
                    .dev
                    .join("input")
                    .join(event.file_name().expect("named event"));
                if dev.exists() {
                    digitizer = Some(dev);
                    break;
                }
            }
        }
        if digitizer.is_some() {
            break;
        }
    }
    let mut native = None;
    for path in children(&roots.sys.join("bus/hid/drivers/hid-appletb-kbd"))? {
        let path = path.join("mode");
        if !under(&path, &device)? {
            continue;
        }
        if let Some(value) = read_optional(&path)? {
            let mode = value
                .parse::<u8>()
                .ok()
                .filter(|v| *v <= 3)
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid native driver mode")
                })?;
            native = Some(NativeNode { path, mode });
            break;
        }
    }
    for path in children(&roots.sys.join("bus/usb/drivers/appletbdrm"))? {
        if under(&path, &device)? {
            drm_bound = true;
            break;
        }
    }
    Ok(Some(Snapshot {
        device,
        config,
        display: card
            .zip(digitizer)
            .map(|(card, digitizer)| DisplayNodes { card, digitizer }),
        native,
        drm_bound,
    }))
}
/// Execution is sequential. The native path cannot write USB until handles have been released.
pub trait Operations {
    fn release_hardware(&mut self) -> io::Result<()>;
    fn load_module(&mut self, name: &str) -> io::Result<()>;
    fn select_config(&mut self, device: &Device, value: u8) -> io::Result<()>;
}
#[derive(Clone, Debug)]
pub struct SwitchPlan {
    pub device: Device,
    pub mode: Mode,
}
impl SwitchPlan {
    pub fn new(device: Device, mode: Mode) -> Self {
        Self { device, mode }
    }
    pub fn execute(&self, ops: &mut impl Operations) -> io::Result<()> {
        match self.mode {
            Mode::Display => ops.load_module("appletbdrm")?,
            Mode::Native => ops.release_hardware()?,
        }
        ops.select_config(&self.device, 0)?;
        ops.select_config(&self.device, if self.mode == Mode::Display { 2 } else { 1 })
    }
}
/// Production executor; release closes the caller's DRM and digitizer handles first.
pub struct SystemOps<F: FnMut() -> io::Result<()>> {
    pub release: F,
}
impl<F: FnMut() -> io::Result<()>> Operations for SystemOps<F> {
    fn release_hardware(&mut self) -> io::Result<()> {
        (self.release)()
    }
    fn load_module(&mut self, name: &str) -> io::Result<()> {
        if name != "appletbdrm" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported module",
            ));
        }
        let status = std::process::Command::new("modprobe")
            .arg(name)
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "modprobe appletbdrm failed: {status}"
            )))
        }
    }
    fn select_config(&mut self, device: &Device, value: u8) -> io::Result<()> {
        device.write_config(value)
    }
}
mod tracker;
pub use tracker::*;
mod events;
pub use events::*;
mod exclusive;
pub use exclusive::*;
