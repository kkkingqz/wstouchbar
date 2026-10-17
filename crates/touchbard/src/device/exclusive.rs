use super::Roots;
use rustix::fs::{FlockOperation, Mode, OFlags, flock, open};
use std::{
    fs::{self, File, Permissions},
    io,
    os::unix::fs::{MetadataExt, PermissionsExt},
};
/// Shared by the daemon and all standalone hardware tools, including probe.
pub struct ExclusiveDevice {
    _file: File,
}
impl ExclusiveDevice {
    pub fn acquire(roots: &Roots) -> io::Result<Self> {
        fs::create_dir_all(&roots.run)?;
        let dir = fs::symlink_metadata(&roots.run)?;
        let uid = rustix::process::geteuid().as_raw();
        if !dir.is_dir() || dir.uid() != uid || dir.permissions().mode() & 0o022 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "runtime directory must belong to daemon UID and forbid group/other writes",
            ));
        }
        let fd = open(
            roots.run.join("device.lock"),
            OFlags::CREATE | OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::RUSR | Mode::WUSR,
        )?;
        let file = File::from(fd);
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.uid() != uid || metadata.nlink() != 1 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "invalid device lock file",
            ));
        }
        flock(&file, FlockOperation::NonBlockingLockExclusive).map_err(|e| {
            io::Error::new(
                io::Error::from(e).kind(),
                "Touch Bar is already owned by a daemon or hardware tool",
            )
        })?;
        file.set_permissions(Permissions::from_mode(0o600))?;
        Ok(Self { _file: file })
    }
}
