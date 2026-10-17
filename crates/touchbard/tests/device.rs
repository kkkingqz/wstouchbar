use std::{fs, io, os::unix::fs::symlink, path::PathBuf};
use touchbar_protocol::Mode;
use touchbard::device::*;
struct Fixture {
    temp: tempfile::TempDir,
    roots: Roots,
    usb: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let roots = Roots {
            sys: temp.path().join("sys"),
            dev: temp.path().join("dev"),
            run: temp.path().join("run"),
        };
        for path in [
            "bus/usb/devices",
            "bus/usb/drivers/appletbdrm",
            "bus/hid/drivers/hid-appletb-kbd",
            "class/drm",
            "class/input",
        ] {
            fs::create_dir_all(roots.sys.join(path)).unwrap();
        }
        fs::create_dir_all(roots.dev.join("dri")).unwrap();
        fs::create_dir_all(roots.dev.join("input")).unwrap();
        let usb = roots.sys.join("devices/pci/usb1/1-1");
        fs::create_dir_all(&usb).unwrap();
        for (name, value) in [
            ("idVendor", "05ac\n"),
            ("idProduct", "8302\n"),
            ("bConfigurationValue", "1\n"),
        ] {
            fs::write(usb.join(name), value).unwrap();
        }
        symlink(&usb, roots.sys.join("bus/usb/devices/1-1")).unwrap();
        Self { temp, roots, usb }
    }
    fn clear(&self) {
        for path in [
            self.roots.sys.join("class/drm/card0"),
            self.roots.sys.join("class/input/input0"),
            self.roots.sys.join("bus/usb/drivers/appletbdrm/1-1:1.0"),
            self.roots.sys.join("bus/hid/drivers/hid-appletb-kbd/hid0"),
        ] {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_dir_all(self.usb.join("1-1:1.0"));
    }
    fn display(&self) {
        self.clear();
        let interface = self.usb.join("1-1:1.0");
        let card = interface.join("drm/card0");
        let input = interface.join("input/input0");
        fs::create_dir_all(&card).unwrap();
        fs::create_dir_all(input.join("event0")).unwrap();
        symlink(&interface, card.join("device")).unwrap();
        symlink(
            self.roots.sys.join("bus/usb/drivers/appletbdrm"),
            interface.join("driver"),
        )
        .unwrap();
        symlink(&card, self.roots.sys.join("class/drm/card0")).unwrap();
        symlink(&input, self.roots.sys.join("class/input/input0")).unwrap();
        symlink(
            &interface,
            self.roots.sys.join("bus/usb/drivers/appletbdrm/1-1:1.0"),
        )
        .unwrap();
        fs::write(
            input.join("name"),
            "Apple Inc. Touch Bar Display Touchpad\n",
        )
        .unwrap();
        fs::write(self.roots.dev.join("dri/card0"), "").unwrap();
        fs::write(self.roots.dev.join("input/event0"), "").unwrap();
    }
    fn native(&self) {
        self.clear();
        let hid = self.usb.join("1-1:1.0/hid0");
        fs::create_dir_all(&hid).unwrap();
        fs::write(hid.join("mode"), "1\n").unwrap();
        symlink(
            hid,
            self.roots.sys.join("bus/hid/drivers/hid-appletb-kbd/hid0"),
        )
        .unwrap();
    }
}
struct FakeOps<'a> {
    fixture: &'a Fixture,
    trace: Vec<String>,
    remove_at_zero: bool,
}
impl Operations for FakeOps<'_> {
    fn release_hardware(&mut self) -> io::Result<()> {
        self.trace.push("release".into());
        Ok(())
    }
    fn load_module(&mut self, name: &str) -> io::Result<()> {
        self.trace.push(format!("modprobe {name}"));
        Ok(())
    }
    fn select_config(&mut self, device: &Device, value: u8) -> io::Result<()> {
        self.trace.push(format!("config {value}"));
        device.write_config(value)?;
        if value == 0 {
            self.fixture.clear();
            if self.remove_at_zero {
                fs::remove_dir_all(&self.fixture.usb)?;
            }
        }
        if value == 2 {
            self.fixture.display();
        }
        if value == 1 {
            self.fixture.native();
        }
        Ok(())
    }
}
#[test]
fn discovers_only_matching_usb_and_children_belonging_to_it() {
    let f = Fixture::new();
    let snapshot = discover(&f.roots).unwrap().unwrap();
    assert_eq!(snapshot.device.path(), f.usb);
    assert!(snapshot.native_safe());
    assert!(snapshot.display.is_none());
    f.display();
    fs::write(f.usb.join("bConfigurationValue"), "2").unwrap();
    let snapshot = discover(&f.roots).unwrap().unwrap();
    let display = snapshot.display.as_ref().unwrap();
    assert_eq!(display.card, f.roots.dev.join("dri/card0"));
    assert_eq!(display.digitizer, f.roots.dev.join("input/event0"));
    assert!(!snapshot.native_safe());
    fs::remove_file(f.roots.sys.join("class/input/input0")).unwrap();
    let foreign = f.temp.path().join("foreign/input0");
    fs::create_dir_all(foreign.join("event9")).unwrap();
    fs::write(
        foreign.join("name"),
        "Apple Inc. Touch Bar Display Touchpad",
    )
    .unwrap();
    symlink(foreign, f.roots.sys.join("class/input/input0")).unwrap();
    assert!(discover(&f.roots).unwrap().unwrap().display.is_none());
    fs::write(f.usb.join("idProduct"), "8303").unwrap();
    assert!(discover(&f.roots).unwrap().is_none());
}
#[test]
fn transitions_force_config_and_release_before_native_with_fake_driver_rebinding() {
    let f = Fixture::new();
    let device = discover(&f.roots).unwrap().unwrap().device;
    let mut ops = FakeOps {
        fixture: &f,
        trace: Vec::new(),
        remove_at_zero: false,
    };
    SwitchPlan::new(device.clone(), Mode::Display)
        .execute(&mut ops)
        .unwrap();
    assert_eq!(ops.trace, ["modprobe appletbdrm", "config 0", "config 2"]);
    assert!(discover(&f.roots).unwrap().unwrap().display.is_some());
    ops.trace.clear();
    SwitchPlan::new(device.clone(), Mode::Display)
        .execute(&mut ops)
        .unwrap();
    assert_eq!(ops.trace, ["modprobe appletbdrm", "config 0", "config 2"]);
    ops.trace.clear();
    SwitchPlan::new(device, Mode::Native)
        .execute(&mut ops)
        .unwrap();
    assert_eq!(ops.trace, ["release", "config 0", "config 1"]);
    let snapshot = discover(&f.roots).unwrap().unwrap();
    assert!(snapshot.native_safe());
    assert_eq!(snapshot.native.unwrap().mode, 1);
}
#[test]
fn removal_and_path_reuse_cannot_write_a_replacement_device() {
    let f = Fixture::new();
    let device = discover(&f.roots).unwrap().unwrap().device;
    let mut ops = FakeOps {
        fixture: &f,
        trace: Vec::new(),
        remove_at_zero: true,
    };
    assert!(
        SwitchPlan::new(device.clone(), Mode::Display)
            .execute(&mut ops)
            .is_err()
    );
    fs::create_dir_all(&f.usb).unwrap();
    fs::write(f.usb.join("bConfigurationValue"), "1").unwrap();
    assert!(device.write_config(2).is_err());
    assert_eq!(
        fs::read_to_string(f.usb.join("bConfigurationValue")).unwrap(),
        "1"
    );
}
#[test]
fn native_driver_metadata_files_and_symlinked_attributes_are_not_treated_as_devices() {
    let f = Fixture::new();
    fs::write(f.roots.sys.join("bus/hid/drivers/hid-appletb-kbd/bind"), "").unwrap();
    let device = discover(&f.roots).unwrap().unwrap().device;
    let outside = f.temp.path().join("outside");
    fs::write(&outside, "keep").unwrap();
    fs::remove_file(f.usb.join("bConfigurationValue")).unwrap();
    symlink(&outside, f.usb.join("bConfigurationValue")).unwrap();
    assert!(device.write_config(2).is_err());
    assert_eq!(fs::read_to_string(outside).unwrap(), "keep");
}
#[test]
fn tracker_publishes_readiness_once_only_after_its_own_switch_finished() {
    let f = Fixture::new();
    let mut tracker = Tracker::new(f.roots.clone());
    assert_eq!(tracker.rescan().unwrap(), vec![Change::Device(Some(1))]);
    let display = tracker.begin_switch(10, 1, Mode::Display).unwrap();
    let mut ops = FakeOps {
        fixture: &f,
        trace: Vec::new(),
        remove_at_zero: false,
    };
    display.execute(&mut ops).unwrap();
    assert!(
        !tracker
            .rescan()
            .unwrap()
            .iter()
            .any(|c| matches!(c, Change::DisplayNodesReady { .. }))
    );
    assert!(tracker.selected(10));
    assert!(tracker.rescan().unwrap().iter().any(|c| matches!(
        c,
        Change::DisplayNodesReady {
            generation: 10,
            device: 1,
            ..
        }
    )));
    assert!(tracker.rescan().unwrap().is_empty());
    tracker
        .begin_switch(11, 1, Mode::Native)
        .unwrap()
        .execute(&mut ops)
        .unwrap();
    f.clear();
    assert!(!tracker.selected(10));
    assert!(tracker.selected(11));
    let changes = tracker.rescan().unwrap();
    assert!(changes.contains(&Change::NativeSafe { generation: 11 }));
    assert!(
        !changes
            .iter()
            .any(|c| matches!(c, Change::NativeDriver { node: Some(_), .. }))
    );
    assert!(tracker.rescan().unwrap().is_empty());
    f.native();
    assert!(
        tracker
            .rescan()
            .unwrap()
            .iter()
            .any(|c| matches!(c,Change::NativeDriver{device:1,node:Some(node)} if node.mode==1))
    );
}
fn changes_to_mode(tracker: &mut Tracker, m: &mut touchbard::machine::mode::ModeMachine, now: u64) {
    use touchbard::machine::mode::Input;
    for change in tracker.rescan().unwrap() {
        match change {
            Change::Device(device) => {
                m.handle(touchbar_model::Timestamp(now), Input::Device(device));
            }
            Change::NativeSafe { generation } => {
                m.handle(
                    touchbar_model::Timestamp(now),
                    Input::NativeSafe { generation },
                );
            }
            Change::NativeDriver { device, node } => {
                m.handle(
                    touchbar_model::Timestamp(now),
                    Input::Driver {
                        device,
                        mode: node.map(|n| n.mode),
                    },
                );
            }
            Change::DisplayNodesReady { generation, .. } => {
                m.handle(
                    touchbar_model::Timestamp(now),
                    Input::DisplayReady { generation },
                );
            }
        }
    }
}
#[test]
fn missing_display_times_out_then_retry_forces_rebind_instead_of_accepting_stale_nodes() {
    use touchbar_model::Timestamp;
    use touchbard::machine::mode::{Input, ModeMachine, State};
    let f = Fixture::new();
    let mut tracker = Tracker::new(f.roots.clone());
    let mut m = ModeMachine::new();
    let mut ops = FakeOps {
        fixture: &f,
        trace: Vec::new(),
        remove_at_zero: false,
    };
    changes_to_mode(&mut tracker, &mut m, 0);
    let generation = m.generation();
    tracker
        .begin_switch(generation, 1, Mode::Native)
        .unwrap()
        .execute(&mut ops)
        .unwrap();
    tracker.selected(generation);
    changes_to_mode(&mut tracker, &mut m, 0);
    m.handle(Timestamp(0), Input::Target(Mode::Display));
    m.advance(Timestamp(2000));
    let first = m.generation();
    tracker
        .begin_switch(first, 1, Mode::Display)
        .unwrap()
        .execute(&mut ops)
        .unwrap();
    f.clear();
    tracker.selected(first);
    changes_to_mode(&mut tracker, &mut m, 2001);
    assert!(matches!(m.state(), State::ToDisplay { .. }));
    m.advance(Timestamp(7000));
    assert_eq!(m.failures(), 1);
    let rollback = m.generation();
    tracker
        .begin_switch(rollback, 1, Mode::Native)
        .unwrap()
        .execute(&mut ops)
        .unwrap();
    tracker.selected(rollback);
    changes_to_mode(&mut tracker, &mut m, 7001);
    m.advance(Timestamp(8000));
    let retry = m.generation();
    ops.trace.clear();
    tracker
        .begin_switch(retry, 1, Mode::Display)
        .unwrap()
        .execute(&mut ops)
        .unwrap();
    assert_eq!(ops.trace, ["modprobe appletbdrm", "config 0", "config 2"]);
    assert!(!tracker.selected(first));
    tracker.selected(retry);
    changes_to_mode(&mut tracker, &mut m, 8001);
    assert!(matches!(m.state(),State::Display{generation} if *generation==retry));
}
#[test]
fn initial_absence_and_recreated_usb_path_invalidate_old_switch_and_device_stability() {
    let f = Fixture::new();
    let mut tracker = Tracker::new(f.roots.clone());
    tracker.rescan().unwrap();
    tracker.begin_switch(1, 1, Mode::Display).unwrap();
    let old = f.usb.with_file_name("old-usb");
    fs::rename(&f.usb, &old).unwrap();
    assert_eq!(tracker.rescan().unwrap(), vec![Change::Device(None)]);
    assert!(!tracker.selected(1));
    fs::create_dir_all(&f.usb).unwrap();
    for (name, value) in [
        ("idVendor", "05ac"),
        ("idProduct", "8302"),
        ("bConfigurationValue", "1"),
    ] {
        fs::write(f.usb.join(name), value).unwrap();
    }
    assert_eq!(tracker.rescan().unwrap(), vec![Change::Device(Some(2))]);
    assert!(tracker.begin_switch(2, 1, Mode::Display).is_err());
    let empty = Roots {
        sys: f.temp.path().join("empty-sys"),
        dev: f.roots.dev.clone(),
        run: f.roots.run.clone(),
    };
    fs::create_dir_all(&empty.sys).unwrap();
    assert_eq!(
        Tracker::new(empty).rescan().unwrap(),
        vec![Change::Device(None)]
    );
}
#[test]
fn exclusive_guard_prevents_probe_and_daemon_from_switching_together() {
    let f = Fixture::new();
    let first = ExclusiveDevice::acquire(&f.roots).unwrap();
    assert!(ExclusiveDevice::acquire(&f.roots).is_err());
    drop(first);
    assert!(ExclusiveDevice::acquire(&f.roots).is_ok());
}
#[test]
fn reenumeration_restarts_full_two_second_settle_interval() {
    use touchbar_model::Timestamp;
    use touchbard::machine::mode::{Command, Input, ModeMachine};
    let f = Fixture::new();
    let mut t = Tracker::new(f.roots.clone());
    let mut m = ModeMachine::new();
    changes_to_mode(&mut t, &mut m, 0);
    m.handle(Timestamp(0), Input::Target(Mode::Display));
    let old = f.usb.with_file_name("old-usb");
    fs::rename(&f.usb, &old).unwrap();
    changes_to_mode(&mut t, &mut m, 1000);
    fs::create_dir_all(&f.usb).unwrap();
    for (name, value) in [
        ("idVendor", "05ac"),
        ("idProduct", "8302"),
        ("bConfigurationValue", "1"),
    ] {
        fs::write(f.usb.join(name), value).unwrap();
    }
    changes_to_mode(&mut t, &mut m, 1500);
    let generation = m.generation();
    let mut ops = FakeOps {
        fixture: &f,
        trace: Vec::new(),
        remove_at_zero: false,
    };
    t.begin_switch(generation, 2, Mode::Native)
        .unwrap()
        .execute(&mut ops)
        .unwrap();
    t.selected(generation);
    changes_to_mode(&mut t, &mut m, 1501);
    assert!(m.advance(Timestamp(3499)).is_empty());
    assert!(matches!(
        m.advance(Timestamp(3500)).as_slice(),
        [Command::ToDisplay { device: 2, .. }]
    ));
}
#[test]
fn exclusive_guard_rejects_runtime_directory_writable_by_other_users() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    fs::create_dir_all(&f.roots.run).unwrap();
    fs::set_permissions(&f.roots.run, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(ExclusiveDevice::acquire(&f.roots).is_err());
}
#[test]
fn native_safety_requires_no_remaining_appletbdrm_binding() {
    let f = Fixture::new();
    f.display();
    fs::write(f.usb.join("bConfigurationValue"), "1").unwrap();
    assert!(!discover(&f.roots).unwrap().unwrap().native_safe());
    // An interface driver link still counts while the bus-level link is being removed.
    fs::remove_file(f.roots.sys.join("bus/usb/drivers/appletbdrm/1-1:1.0")).unwrap();
    assert!(!discover(&f.roots).unwrap().unwrap().native_safe());
    f.clear();
    assert!(discover(&f.roots).unwrap().unwrap().native_safe());
}
