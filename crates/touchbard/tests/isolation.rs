use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};
use touchbard::device::isolation::{RULES, verify_rules_at};
#[test]
fn isolation_preflight_rejects_missing_stale_untrusted_and_symlink_rules() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("isolation.rules");
    let uid = rustix::process::geteuid().as_raw();
    assert!(verify_rules_at(&file, uid).is_err());
    fs::write(&file, RULES).unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    verify_rules_at(&file, uid).unwrap();
    assert!(verify_rules_at(&file, uid.wrapping_add(1)).is_err());
    fs::set_permissions(&file, fs::Permissions::from_mode(0o664)).unwrap();
    assert!(verify_rules_at(&file, uid).is_err());
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    fs::write(&file, "# older rule without isolation\n").unwrap();
    assert!(verify_rules_at(&file, uid).is_err());
    fs::write(&file, RULES).unwrap();
    let link = dir.path().join("link.rules");
    symlink(&file, &link).unwrap();
    assert!(verify_rules_at(&link, uid).is_err());
    fs::write(&file, vec![b' '; 100_000]).unwrap();
    assert!(verify_rules_at(&file, uid).is_err());
}
