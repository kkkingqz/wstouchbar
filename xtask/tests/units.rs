use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .into()
}
fn unit(name: &str) -> String {
    fs::read_to_string(root().join("packaging/systemd").join(name)).unwrap()
}
fn values(text: &str, key: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.trim().strip_prefix(&format!("{key}=")))
        .flat_map(str::split_whitespace)
        .map(str::to_owned)
        .collect()
}
#[test]
fn service_contract_keeps_cleanup_recovery_and_process_group_ownership() {
    let daemon = unit("touchbard.service");
    assert!(
        values(&daemon, "ExecStartPre")
            .iter()
            .any(|v| v == "hid_appletb_kbd")
    );
    assert!(
        daemon
            .lines()
            .any(|l| l.starts_with("ExecStopPost=") && l.contains("--hand-back"))
    );
    assert_eq!(values(&daemon, "ProtectSystem"), ["strict"]);
    assert_eq!(values(&daemon, "ProtectKernelTunables"), ["true"]);
    assert_eq!(values(&daemon, "ProtectKernelModules"), ["false"]);
    assert_eq!(values(&daemon, "DevicePolicy"), ["closed"]);
    assert_eq!(
        values(&daemon, "RestrictAddressFamilies"),
        ["AF_UNIX", "AF_NETLINK"]
    );
    assert!(values(&daemon, "BindsTo").is_empty());
    assert_eq!(values(&daemon, "RuntimeDirectoryPreserve"), ["yes"]);
    assert_eq!(values(&daemon, "KillMode"), ["control-group"]);
    let session = unit("touchbar-session.service");
    assert_eq!(values(&session, "KillMode"), ["control-group"]);
    assert_eq!(values(&session, "Restart"), ["on-failure"]);
    assert_eq!(values(&session, "WantedBy"), ["graphical-session.target"]);
    assert!(values(&session, "PartOf").contains(&"graphical-session.target".into()));
    assert!(!values(&session, "RestartMaxDelaySec").is_empty());
}
fn write(path: &Path, value: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, value).unwrap();
}
fn state(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, p: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        if !p.exists() {
            return;
        }
        for e in fs::read_dir(p).unwrap() {
            let e = e.unwrap();
            let kind = e.file_type().unwrap();
            if kind.is_dir() {
                walk(root, &e.path(), out)
            } else if !kind.is_symlink() {
                out.insert(
                    e.path().strip_prefix(root).unwrap().into(),
                    if kind.is_file() {
                        fs::read(e.path()).unwrap()
                    } else {
                        b"socket".to_vec()
                    },
                );
            }
        }
    }
    let mut map = BTreeMap::new();
    walk(root, root, &mut map);
    map
}
#[test]
fn actual_sysfs_and_runtime_writes_fit_the_unit_mount_exceptions() {
    use std::os::unix::fs::PermissionsExt;
    use touchbard::{
        backlight::{Autodim, Backlight},
        device::{ExclusiveDevice, Roots, SwitchPlan, SystemOps, discover},
        keys::ModeWriter,
        runtime::{marker::Recovery, sockets::Sockets},
    };
    let dir = tempfile::tempdir().unwrap();
    let roots = Roots {
        sys: dir.path().join("sys"),
        dev: dir.path().join("dev"),
        run: dir.path().join("run/touchbar"),
    };
    for name in [
        "bus/usb/devices",
        "bus/usb/drivers/appletbdrm",
        "bus/hid/drivers/hid-appletb-kbd",
        "class/drm",
        "class/input",
        "class/backlight",
        "bus/hid/drivers/hid-appletb-bl",
    ] {
        fs::create_dir_all(roots.sys.join(name)).unwrap();
    }
    fs::create_dir_all(&roots.run).unwrap();
    fs::set_permissions(&roots.run, fs::Permissions::from_mode(0o755)).unwrap();
    let usb = roots.sys.join("devices/pci/usb1/1-1");
    for (name, value) in [
        ("idVendor", "05ac\n"),
        ("idProduct", "8302\n"),
        ("bConfigurationValue", "2\n"),
    ] {
        write(&usb.join(name), value);
    }
    symlink(&usb, roots.sys.join("bus/usb/devices/1-1")).unwrap();
    let hid = usb.join("1-1:1.0/hid0");
    write(&hid.join("mode"), "1\n");
    symlink(&hid, roots.sys.join("bus/hid/drivers/hid-appletb-kbd/hid0")).unwrap();
    let bl = roots
        .sys
        .join("devices/pci/usb1/1-2/hid0/backlight/appletb_backlight");
    write(&bl.join("brightness"), "0\n");
    write(&bl.join("max_brightness"), "2\n");
    symlink(&bl, roots.sys.join("class/backlight/appletb_backlight")).unwrap();
    let owner = bl.parent().unwrap().parent().unwrap();
    symlink(owner, bl.join("device")).unwrap();
    symlink(
        roots.sys.join("bus/hid/drivers/hid-appletb-bl"),
        owner.join("driver"),
    )
    .unwrap();
    write(
        &roots.sys.join("module/hid_appletb_kbd/parameters/autodim"),
        "Y\n",
    );
    let before = state(dir.path());
    let _lock = ExclusiveDevice::acquire(&roots).unwrap();
    let device = discover(&roots).unwrap().unwrap().device;
    SwitchPlan::new(device.clone(), touchbar_protocol::Mode::Native)
        .execute(&mut SystemOps { release: || Ok(()) })
        .unwrap();
    ModeWriter::open(&device, &hid.join("mode"))
        .unwrap()
        .write_mode(2)
        .unwrap();
    Backlight::open(&roots).unwrap().set_level(1).unwrap();
    Autodim::new(&roots).take_over().unwrap();
    Recovery::new(&roots.run).write(true).unwrap();
    let _sockets = Sockets::bind(&roots.run).unwrap();
    let writes = state(dir.path())
        .into_iter()
        .filter(|(path, value)| before.get(path) != Some(value))
        .map(|(p, _)| Path::new("/").join(p))
        .collect::<Vec<_>>();
    assert!(writes.len() >= 8, "missing exercised writes: {writes:?}");
    let exceptions = values(&unit("touchbard.service"), "ReadWritePaths")
        .into_iter()
        .map(|s| PathBuf::from(s.trim_start_matches('-')))
        .collect::<Vec<_>>();
    assert!(
        !exceptions
            .iter()
            .any(|p| p == Path::new("/sys") || p == Path::new("/sys/module"))
    );
    for path in writes {
        assert!(
            exceptions.iter().any(|p| path.starts_with(p)),
            "write {path:?} not covered by {exceptions:?}"
        );
    }
}

#[test]
#[ignore = "requires systemd-analyze, modprobe and udevadm; offline validation only"]
fn env_packaging_systemd_units_and_udev_rules_verify() {
    use std::os::unix::fs::PermissionsExt;
    fn command(program: &str, args: &[&std::ffi::OsStr]) {
        let output = std::process::Command::new(program)
            .args(args)
            .output()
            .unwrap_or_else(|e| panic!("{program} unavailable: {e}"));
        assert!(
            output.status.success(),
            "{program}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    for name in [
        "touchbard",
        "touchbar-session",
        "touchbar-spawn-helper",
        "touchbarctl",
    ] {
        write(&bin.join(name), "#!/bin/sh\nexit 0\n");
    }
    let plugins = dir.path().join("plugins");
    write(&plugins.join("controls/plugin.toml"), "id='controls'\n");
    let a = xtask::install::Artifacts {
        daemon: bin.join("touchbard"),
        session: bin.join("touchbar-session"),
        helper: bin.join("touchbar-spawn-helper"),
        ctl: bin.join("touchbarctl"),
        plugins,
    };
    let stage = dir.path().join("stage");
    xtask::install::assemble(&root(), &stage, Path::new("/usr"), &a).unwrap();
    for (name, scope) in [
        ("touchbard.service", "system"),
        ("touchbar-session.service", "user"),
    ] {
        let path = dir.path().join(name);
        let text =
            fs::read_to_string(stage.join("usr/lib/systemd").join(scope).join(name)).unwrap();
        // Offline verify checks syntax and executable availability. Relocate
        // executable paths to the staging stand-ins; never install/run a unit.
        fs::write(
            &path,
            text.replace(
                "/usr/lib/touchbar/",
                &format!("{}/usr/lib/touchbar/", stage.display()),
            ),
        )
        .unwrap();
        let mut args = vec![
            std::ffi::OsStr::new("verify"),
            std::ffi::OsStr::new("--man=no"),
        ];
        if scope == "user" {
            args.push(std::ffi::OsStr::new("--user"));
        }
        args.push(path.as_os_str());
        command("systemd-analyze", &args);
    }
    for name in ["90-touchbar.rules", "99-wstouchbar-isolation.rules"] {
        command(
            "udevadm",
            &[
                std::ffi::OsStr::new("verify"),
                root().join("packaging/udev").join(name).as_os_str(),
            ],
        );
    }
    assert_eq!(
        fs::metadata(stage.join("usr/lib/touchbar/touchbard"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
}

#[test]
#[ignore = "requires a running systemd user manager, user namespaces and python3; starts only a read-only diagnostic"]
fn env_systemd_sysfs_mount_exceptions_are_effective() {
    assert_ne!(
        rustix::process::geteuid().as_raw(),
        0,
        "run as a normal user"
    );
    let mut command = std::process::Command::new("systemd-run");
    let nonce = tempfile::tempdir().unwrap();
    command.args(["--user", "--wait", "--pipe", "--collect"]);
    command.arg(format!(
        "--unit=wstouchbar-sysfs-{}-{}",
        std::process::id(),
        nonce
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .trim_start_matches('.')
    ));
    command.args([
        "--property=PrivateUsers=yes",
        "--property=RuntimeMaxSec=10s",
        "--property=TimeoutStopSec=5s",
    ]);
    let text = unit("touchbard.service");
    for key in [
        "ProtectSystem",
        "ProtectKernelTunables",
        "ProtectKernelModules",
        "ProtectControlGroups",
        "BindReadOnlyPaths",
        "ReadWritePaths",
    ] {
        let mut paths = values(&text, key);
        if key == "ReadWritePaths" {
            // This diagnostic never opens the daemon runtime or any device.
            paths.retain(|p| p.trim_start_matches('-').starts_with("/sys/"));
        }
        if !paths.is_empty() {
            command.arg(format!("--property={key}={}", paths.join(" ")));
        }
    }
    let output = command
        .args([
            "/usr/bin/python3",
            "-c",
            "import os,pathlib; expected={'/sys':True,'/sys/module':True,'/sys/devices':False,'/usr':True,'/etc':True}; p='/sys/module/hid_appletb_kbd/parameters'; expected.update({p:False} if pathlib.Path(p).exists() else {}); actual={p:bool(os.statvfs(p).f_flag & os.ST_RDONLY) for p in expected}; print(actual); assert actual==expected, (actual,expected)",
        ])
        .output()
        .expect("systemd-run unavailable");
    assert!(
        output.status.success(),
        "sysfs mount protection differs from the unit contract:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
