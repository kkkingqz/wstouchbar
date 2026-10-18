//! Durable boot-scoped recovery state, relative to a trusted daemon directory.
use rustix::fs::{AtFlags, Mode, OFlags, open, openat, renameat, unlinkat};
use std::{
    fs::File,
    io::{self, Read, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
pub struct Recovery {
    dir: PathBuf,
}
impl Recovery {
    pub fn new(dir: &Path) -> Self {
        Self { dir: dir.into() }
    }
    fn dir(&self) -> io::Result<File> {
        let dir = File::from(open(
            &self.dir,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        let m = dir.metadata()?;
        if m.uid() != rustix::process::geteuid().as_raw() || m.mode() & 0o022 != 0 {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        Ok(dir)
    }
    fn entry(dir: &File, name: &str) -> io::Result<Option<File>> {
        let file = match openat(
            dir,
            name,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        ) {
            Ok(fd) => File::from(fd),
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let m = file.metadata()?;
        if !m.is_file()
            || m.uid() != rustix::process::geteuid().as_raw()
            || m.mode() & 0o022 != 0
            || m.nlink() != 1
        {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        Ok(Some(file))
    }
    pub fn read(&self) -> io::Result<bool> {
        let dir = self.dir()?;
        let Some(file) = Self::entry(&dir, "recovery")? else {
            return Ok(false);
        };
        let mut bytes = Vec::new();
        file.take(8).read_to_end(&mut bytes)?;
        if bytes != b"1\n" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "corrupt recovery marker",
            ));
        }
        Ok(true)
    }
    pub fn write(&self, value: bool) -> io::Result<()> {
        let dir = self.dir()?;
        // Never replace or remove a link, including a stale temporary file.
        let exists = Self::entry(&dir, "recovery")?.is_some();
        if !value {
            if exists {
                unlinkat(&dir, "recovery", AtFlags::empty())?;
                dir.sync_all()?;
            }
            return Ok(());
        }
        if Self::entry(&dir, "recovery.pending")?.is_some() {
            unlinkat(&dir, "recovery.pending", AtFlags::empty())?;
        }
        let mut file = File::from(openat(
            &dir,
            "recovery.pending",
            OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )?);
        file.write_all(b"1\n")?;
        file.sync_all()?;
        renameat(&dir, "recovery.pending", &dir, "recovery")?;
        dir.sync_all()
    }
}
