use std::{path::PathBuf, process::Command};
// Hardware tests are compiled in a container and executed on the host. Cargo's
// CARGO_BIN_EXE absolute path belongs to the container; use the same target profile.
fn binary() -> PathBuf {
    std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("touchbard{}", std::env::consts::EXE_SUFFIX))
}
#[test]
fn help_does_not_access_hardware_and_unknown_command_fails() {
    let help = Command::new(binary()).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("probe"));
    let bad = Command::new(binary()).arg("unknown").output().unwrap();
    assert!(!bad.status.success());
}
#[test]
#[ignore = "requires root, real Touch Bar, stopped hardware services and explicit stand approval"]
fn env_hardware_probe_roundtrip() {
    let output = Command::new(binary()).arg("probe").output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["native_safe"], true);
    assert_eq!(report["buffer_count"], 3);
    assert_eq!(report["format"], touchbar_protocol::XRGB8888);
    assert_eq!(report["commits"].as_array().unwrap().len(), 4);
    assert_eq!(report["prime_cloexec"], true);
}
