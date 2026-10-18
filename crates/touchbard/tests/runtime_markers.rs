use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};
use touchbard::runtime::marker::Recovery;
#[test]
fn recovery_survives_restart_and_false_removes_it() {
    let dir = tempfile::tempdir().unwrap();
    let marker = Recovery::new(dir.path());
    assert!(!marker.read().unwrap());
    marker.write(true).unwrap();
    assert!(Recovery::new(dir.path()).read().unwrap());
    assert_eq!(
        fs::metadata(dir.path().join("recovery"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    marker.write(false).unwrap();
    assert!(!marker.read().unwrap());
}
#[test]
fn recovery_rejects_links_corruption_and_writable_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let marker = Recovery::new(dir.path());
    let outside = dir.path().join("outside");
    fs::write(&outside, "keep").unwrap();
    symlink(&outside, dir.path().join("recovery")).unwrap();
    assert!(marker.read().is_err());
    assert!(marker.write(true).is_err());
    assert!(marker.write(false).is_err());
    assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
    fs::remove_file(dir.path().join("recovery")).unwrap();
    fs::write(dir.path().join("recovery"), "invalid").unwrap();
    assert!(marker.read().is_err());
    fs::remove_file(dir.path().join("recovery")).unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o777)).unwrap();
    assert!(marker.read().is_err());
    assert!(marker.write(true).is_err());
}
