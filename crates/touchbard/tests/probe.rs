use std::process::Command;
#[test]
fn help_does_not_access_hardware_and_unknown_command_fails() {
    let help = Command::new(env!("CARGO_BIN_EXE_touchbard"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("probe"));
    let bad = Command::new(env!("CARGO_BIN_EXE_touchbard"))
        .arg("unknown")
        .output()
        .unwrap();
    assert!(!bad.status.success());
}
#[test]
#[ignore = "requires root, real Touch Bar, stopped hardware services and explicit stand approval"]
fn env_hardware_probe_roundtrip() {
    let output = Command::new(env!("CARGO_BIN_EXE_touchbard"))
        .arg("probe")
        .output()
        .unwrap();
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
