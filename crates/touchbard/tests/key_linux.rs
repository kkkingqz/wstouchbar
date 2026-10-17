use std::{fs, os::unix::fs::symlink};
use touchbard::{
    device::{Roots, discover},
    keys::{InputKind, ModeWriter, classify_input},
};
#[test]
fn classification_uses_sysfs_ancestry_not_forged_usb_bus_or_name() {
    let sys = std::path::Path::new("/sys");
    assert_eq!(
        classify_input(
            sys,
            std::path::Path::new("/sys/devices/virtual/input/input4/event4"),
            "USB Keyboard"
        )
        .unwrap(),
        InputKind::Virtual
    );
    assert_eq!(
        classify_input(
            sys,
            std::path::Path::new("/sys/devices/pci0000/usb/input4/event4"),
            "USB Keyboard"
        )
        .unwrap(),
        InputKind::Physical
    );
    assert_eq!(
        classify_input(
            sys,
            std::path::Path::new("/sys/devices/virtual/input/input4/event4"),
            "TouchBar System Keys"
        )
        .unwrap(),
        InputKind::Own
    );
    assert!(
        classify_input(
            sys,
            std::path::Path::new("/tmp/devices/physical"),
            "Keyboard"
        )
        .is_err()
    );
}
#[test]
fn fn_mode_writer_is_pinned_under_selected_usb_and_rejects_replacements() {
    let temp = tempfile::tempdir().unwrap();
    let sys = temp.path().join("sys");
    let usb = sys.join("devices/usb/1-1");
    let hid = usb.join("1-1:1.0/hid0");
    fs::create_dir_all(&hid).unwrap();
    fs::create_dir_all(sys.join("bus/usb/devices")).unwrap();
    for (name, value) in [
        ("idVendor", "05ac\n"),
        ("idProduct", "8302\n"),
        ("bConfigurationValue", "1\n"),
    ] {
        fs::write(usb.join(name), value).unwrap();
    }
    symlink(&usb, sys.join("bus/usb/devices/1-1")).unwrap();
    let mode = hid.join("mode");
    fs::write(&mode, "1\n").unwrap();
    let roots = Roots {
        sys,
        dev: temp.path().join("dev"),
        run: temp.path().join("run"),
    };
    let device = discover(&roots).unwrap().unwrap().device;
    let mut writer = ModeWriter::open(&device, &mode).unwrap();
    assert_eq!(writer.read_mode().unwrap(), 1);
    writer.write_mode(2).unwrap();
    assert_eq!(fs::read_to_string(&mode).unwrap(), "2\n");
    assert!(writer.write_mode(4).is_err());
    let outside = temp.path().join("mode");
    fs::write(&outside, "0\n").unwrap();
    assert!(ModeWriter::open(&device, &outside).is_err());
    fs::remove_file(&mode).unwrap();
    symlink(&outside, &mode).unwrap();
    assert!(ModeWriter::open(&device, &mode).is_err());
    fs::remove_file(&mode).unwrap();
    fs::rename(&usb, usb.with_file_name("old")).unwrap();
    fs::create_dir_all(&hid).unwrap();
    fs::write(&mode, "1\n").unwrap();
    assert!(ModeWriter::open(&device, &mode).is_err());
    assert_eq!(fs::read_to_string(&mode).unwrap(), "1\n");
}
#[test]
#[ignore = "requires host root and uinput; grabs only the newly created test keyboard before emitting F5"]
fn env_hardware_uinput_f5_roundtrip_and_device_removal() {
    use touchbar_model::{KeyPhase, SystemKey};
    use touchbard::keys::Uinput;
    let mut output = Uinput::open(std::path::Path::new("/dev/uinput")).unwrap();
    let sysname = output.sysname().unwrap();
    let (fd, event_path) =
        exclusive_test_reader(&sysname, std::time::Duration::from_secs(3)).unwrap();
    output.send(SystemKey::F5, KeyPhase::Pressed).unwrap();
    output.release_all().unwrap();
    futures_lite::future::block_on(async {
        let input = async_io::Async::new(fd).unwrap();
        let mut bytes = [0u8; touchbard::input::EVENT_SIZE * 64];
        let mut values = Vec::new();
        let read = async {
            while values.len() < 2 {
                let n = input
                    .read_with(|fd| rustix::io::read(fd, &mut bytes).map_err(std::io::Error::from))
                    .await
                    .unwrap();
                assert_ne!(n, 0);
                for event in touchbard::input::decode_events(&bytes[..n]).unwrap() {
                    if event.kind == 1 && event.code == 63 {
                        values.push(event.value);
                    }
                }
            }
        };
        futures_lite::future::or(read, async {
            async_io::Timer::after(std::time::Duration::from_secs(3)).await;
            panic!("F5 not delivered");
        })
        .await;
        assert_eq!(values, [1, 0]);
        drop(output);
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while event_path.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "test device not removed"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[allow(unsafe_code)]
fn exclusive_test_reader(
    sysname: &str,
    timeout: std::time::Duration,
) -> std::io::Result<(std::os::fd::OwnedFd, std::path::PathBuf)> {
    use rustix::{
        fs::{Mode, OFlags},
        ioctl::{self, IntegerSetter, Updater, opcode},
    };
    let sys = std::path::Path::new("/sys/devices/virtual/input").join(sysname);
    let deadline = std::time::Instant::now() + timeout;
    loop {
        for entry in fs::read_dir(&sys)? {
            let entry = entry?;
            let name = entry.file_name();
            if !name.as_encoded_bytes().starts_with(b"event") {
                continue;
            }
            let path = std::path::Path::new("/dev/input").join(name);
            match rustix::fs::open(
                &path,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
                Mode::empty(),
            ) {
                Ok(fd) => {
                    // SAFETY: EVIOCGRAB is an integer-valued ioctl. This fd is the event node of
                    // the unique UI_GET_SYSNAME returned by this test's own uinput device.
                    unsafe {
                        ioctl::ioctl(
                            &fd,
                            IntegerSetter::<{ opcode::write::<i32>(b'E', 0x90) }>::new_usize(1),
                        )?;
                    }
                    let mut types = [0u8; 4];
                    let mut keys = [0u8; 96];
                    // SAFETY: EVIOCGBIT buffers are initialized and exactly match the
                    // encoded lengths. Only this test's unique grabbed device is queried.
                    unsafe {
                        ioctl::ioctl(
                            &fd,
                            Updater::<{ opcode::read::<[u8; 4]>(b'E', 0x20) }, _>::new(&mut types),
                        )?;
                        ioctl::ioctl(
                            &fd,
                            Updater::<{ opcode::read::<[u8; 96]>(b'E', 0x21) }, _>::new(&mut keys),
                        )?;
                    }
                    assert_ne!(u32::from_ne_bytes(types) & (1 << 20), 0, "EV_REP missing");
                    for code in 0..768u16 {
                        let present = keys[usize::from(code) / 8] & (1 << (code % 8)) != 0;
                        assert_eq!(
                            present,
                            touchbard::keys::system_key(code).is_some(),
                            "unexpected capability {code}"
                        );
                    }
                    return Ok((fd, path));
                }
                Err(rustix::io::Errno::NOENT) => {}
                Err(e) => return Err(e.into()),
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
#[test]
fn directory_hotplug_skips_non_evdev_files_and_survives_missing_input_directory() {
    futures_lite::future::block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let dev = temp.path().join("dev");
        fs::create_dir_all(&dev).unwrap();
        let roots = Roots {
            sys: temp.path().join("sys"),
            dev: dev.clone(),
            run: temp.path().join("run"),
        };
        let mut inputs = touchbard::keys::KeyInputs::open(&roots).unwrap();
        assert!(inputs.next().await.unwrap().changes.keys.is_empty());
        fs::create_dir(dev.join("input")).unwrap();
        fs::write(dev.join("input/event1"), "not a character device").unwrap();
        let out = futures_lite::future::or(async { Some(inputs.next().await.unwrap()) }, async {
            async_io::Timer::after(std::time::Duration::from_secs(2)).await;
            None
        })
        .await
        .expect("directory change was not observed");
        assert!(out.changes.keys.is_empty());
        assert!(!out.changes.activity);
        fs::remove_file(dev.join("input/event1")).unwrap();
        fs::remove_dir(dev.join("input")).unwrap();
        let out = inputs.next().await.unwrap();
        assert!(out.changes.keys.is_empty());
    });
}
#[test]
fn bad_hotplug_scan_is_reported_locally_and_next_directory_change_recovers() {
    futures_lite::future::block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let roots = Roots {
            sys: temp.path().join("sys"),
            dev: temp.path().join("dev"),
            run: temp.path().join("run"),
        };
        fs::create_dir_all(&roots.dev).unwrap();
        let mut input = touchbard::keys::KeyInputs::open(&roots).unwrap();
        input.next().await.unwrap();
        fs::write(roots.dev.join("input"), "replaced by ordinary file").unwrap();
        let out = input
            .next()
            .await
            .expect("hotplug scan must not discard consumed key edges");
        assert_eq!(out.faults.len(), 1);
        fs::remove_file(roots.dev.join("input")).unwrap();
        fs::create_dir(roots.dev.join("input")).unwrap();
        assert!(input.next().await.unwrap().changes.keys.is_empty());
    });
}
