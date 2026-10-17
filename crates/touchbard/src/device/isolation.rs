//! Display-mode precondition: compositor isolation must precede the DRM hotplug.
use std::{
    fs::File,
    io::{self, Read},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};
pub const RULE_NAME: &str = "99-wstouchbar-isolation.rules";
pub const RULES: &str = include_str!("../../../../packaging/udev/99-wstouchbar-isolation.rules");
/// A configurable expected UID makes the trust check testable without chown/root.
/// Production always calls this with UID 0 through `require_installed`.
pub fn verify_rules_at(path: &Path, expected_uid: u32) -> io::Result<()> {
    let fd = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    )?;
    let file = File::from(fd);
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != expected_uid
        || metadata.permissions().mode() & 0o022 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "untrusted Touch Bar isolation rule",
        ));
    }
    let mut content = Vec::with_capacity(RULES.len() + 1);
    file.take(u64::try_from(RULES.len() + 1).expect("small bundled rule"))
        .read_to_end(&mut content)?;
    if content != RULES.as_bytes() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Touch Bar isolation rule differs from the bundled version",
        ));
    }
    Ok(())
}
pub fn require_installed() -> io::Result<PathBuf> {
    // udev's filename priority: a higher-priority override must also pass validation.
    for directory in [
        "/etc/udev/rules.d",
        "/run/udev/rules.d",
        "/usr/local/lib/udev/rules.d",
        "/usr/lib/udev/rules.d",
    ] {
        let path = Path::new(directory).join(RULE_NAME);
        match verify_rules_at(&path, 0) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(io::Error::new(e.kind(), format!("{}: {e}", path.display()))),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "Touch Bar compositor isolation is not installed; install packaging/udev/99-wstouchbar-isolation.rules before probe or display mode",
    ))
}
/// Blocking control command: called only from the device worker, before modprobe/config 2.
pub fn reload() -> io::Result<()> {
    require_installed()?;
    let status = std::process::Command::new("udevadm")
        .args(["control", "--reload"])
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .status()?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "udev isolation rules reload failed: {status}"
        )));
    }
    Ok(())
}
