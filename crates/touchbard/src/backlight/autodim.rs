use super::invalid;
use crate::device::Roots;
use rustix::fs::{AtFlags, Mode, OFlags, open, openat, renameat, unlinkat};
use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    os::{
        fd::AsFd,
        unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    },
};
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DriverSettings {
    pub autodim: Option<bool>,
    pub dim_seconds: Option<u64>,
    pub idle_seconds: Option<u64>,
}
#[derive(Clone)]
pub struct Autodim {
    roots: Roots,
}
pub struct PreparedAutodim {
    owner: Autodim,
}
impl PreparedAutodim {
    pub fn disable(self) -> io::Result<()> {
        self.owner.write_param("N\n")
    }
}
impl Autodim {
    pub fn new(roots: &Roots) -> Self {
        Self {
            roots: roots.clone(),
        }
    }
    fn dir(&self, create: bool) -> io::Result<File> {
        if create {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o755)
                .create(&self.roots.run)?;
        }
        let file = File::from(open(
            &self.roots.run,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        let m = file.metadata()?;
        if m.uid() != rustix::process::geteuid().as_raw() || m.permissions().mode() & 0o022 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "untrusted autodim runtime directory",
            ));
        }
        Ok(file)
    }
    fn trusted(file: &File) -> io::Result<()> {
        let m = file.metadata()?;
        if !m.is_file()
            || m.uid() != rustix::process::geteuid().as_raw()
            || m.nlink() != 1
            || m.permissions().mode() & 0o022 != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "untrusted autodim marker",
            ));
        }
        Ok(())
    }
    fn marker(&self) -> io::Result<Option<File>> {
        let dir = match self.dir(false) {
            Ok(d) => d,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut file = match openat(
            &dir,
            "autodim",
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        ) {
            Ok(f) => File::from(f),
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        Self::trusted(&file)?;
        let mut bytes = Vec::new();
        std::io::Read::by_ref(&mut file)
            .take(4)
            .read_to_end(&mut bytes)?;
        if bytes != b"Y\n" {
            return Err(invalid("invalid autodim recovery marker"));
        }
        Ok(Some(dir))
    }
    fn persist(&self) -> io::Result<()> {
        let dir = self.dir(true)?;
        // A failed/crashed write may leave a pending file, but never a partial committed
        // marker. The directory is private to the daemon UID; validate before cleanup.
        match openat(
            &dir,
            "autodim.pending",
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => {
                Self::trusted(&File::from(fd))?;
                unlinkat(&dir, "autodim.pending", AtFlags::empty())?;
            }
            Err(rustix::io::Errno::NOENT) => {}
            Err(e) => return Err(e.into()),
        }
        let mut file = File::from(openat(
            &dir,
            "autodim.pending",
            OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )?);
        file.write_all(b"Y\n")?;
        file.sync_all()?;
        renameat(&dir, "autodim.pending", &dir, "autodim")?;
        dir.sync_all()
    }
    fn param(&self, name: &str) -> io::Result<File> {
        Ok(open(
            self.roots
                .sys
                .join("module/hid_appletb_kbd/parameters")
                .join(name),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )?
        .into())
    }
    fn read(&self, name: &str) -> io::Result<Option<String>> {
        let file = match self.param(name) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut s = String::new();
        file.take(64).read_to_string(&mut s)?;
        Ok(Some(s.trim().to_owned()))
    }
    pub fn settings(&self) -> io::Result<DriverSettings> {
        let autodim = match self.read("autodim")?.as_deref() {
            None => None,
            Some("Y") => Some(true),
            Some("N") => Some(false),
            _ => return Err(invalid("invalid autodim parameter")),
        };
        let seconds = |name| {
            self.read(name)?
                .map(|s| {
                    s.parse::<u64>()
                        .ok()
                        .filter(|n| n.checked_mul(1000).is_some())
                        .ok_or_else(|| invalid("invalid dimming timeout"))
                })
                .transpose()
        };
        Ok(DriverSettings {
            autodim,
            dim_seconds: seconds("dim_timeout")?,
            idle_seconds: seconds("idle_timeout")?,
        })
    }
    fn write_param(&self, value: &str) -> io::Result<()> {
        let mut file = File::from(open(
            self.roots
                .sys
                .join("module/hid_appletb_kbd/parameters/autodim"),
            OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )?);
        file.seek(SeekFrom::Start(0))?;
        file.write_all(value.as_bytes())
    }
    /// This phase is complete only after the committed marker and directory are
    /// synced. Dropping it models a crash before the parameter is changed.
    pub fn prepare_take_over(&self) -> io::Result<Option<PreparedAutodim>> {
        let owned = self.marker()?.is_some();
        let enabled = match self.read("autodim")?.as_deref() {
            Some("Y") => true,
            Some("N") => false,
            None => return Ok(None),
            _ => return Err(invalid("invalid autodim parameter")),
        };
        if !enabled && !owned {
            return Ok(None);
        }
        if !owned {
            self.persist()?;
        }
        Ok(Some(PreparedAutodim {
            owner: self.clone(),
        }))
    }
    /// A previous daemon may have disabled the parameter before crashing.
    pub fn owned(&self) -> io::Result<bool> {
        Ok(self.marker()?.is_some())
    }
    pub fn take_over(&self) -> io::Result<bool> {
        match self.prepare_take_over()? {
            Some(prepared) => {
                prepared.disable()?;
                Ok(true)
            }
            None => Ok(false),
        }
    }
    pub fn restore(&self) -> io::Result<bool> {
        let Some(dir) = self.marker()? else {
            return Ok(false);
        };
        self.write_param("Y\n")?;
        unlinkat(dir.as_fd(), "autodim", AtFlags::empty())?;
        dir.sync_all()?;
        Ok(true)
    }
}
