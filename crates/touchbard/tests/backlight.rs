use std::{fs, os::unix::fs::symlink, path::PathBuf};
use touchbard::{
    backlight::{Autodim, Backlight, hand_back_with},
    device::Roots,
};
struct Fixture {
    _temp: tempfile::TempDir,
    roots: Roots,
    usb: PathBuf,
    params: PathBuf,
    bl: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let roots = Roots {
            sys: temp.path().join("sys"),
            dev: temp.path().join("dev"),
            run: temp.path().join("run"),
        };
        let usb = roots.sys.join("devices/usb/1-1");
        let hid = roots.sys.join("devices/usb/1-2/1-2:1.0/hid0");
        let bl = hid.join("backlight/appletb_backlight");
        let params = roots.sys.join("module/hid_appletb_kbd/parameters");
        for p in [
            &usb,
            &bl,
            &params,
            &roots.sys.join("class/backlight"),
            &roots.sys.join("bus/usb/devices"),
        ] {
            fs::create_dir_all(p).unwrap();
        }
        for (name, value) in [
            ("idVendor", "05ac\n"),
            ("idProduct", "8302\n"),
            ("bConfigurationValue", "2\n"),
        ] {
            fs::write(usb.join(name), value).unwrap();
        }
        symlink(&usb, roots.sys.join("bus/usb/devices/1-1")).unwrap();
        symlink(&bl, roots.sys.join("class/backlight/appletb_backlight")).unwrap();
        let driver = roots.sys.join("bus/hid/drivers/hid-appletb-bl");
        fs::create_dir_all(&driver).unwrap();
        symlink(&hid, bl.join("device")).unwrap();
        symlink(&driver, hid.join("driver")).unwrap();
        fs::write(bl.join("max_brightness"), "2\n").unwrap();
        fs::write(bl.join("brightness"), "0\n").unwrap();
        for (name, value) in [
            ("autodim", "Y\n"),
            ("dim_timeout", "30\n"),
            ("idle_timeout", "15\n"),
        ] {
            fs::write(params.join(name), value).unwrap();
        }
        Self {
            _temp: temp,
            roots,
            usb,
            params,
            bl,
        }
    }
    fn autodim(&self) -> String {
        fs::read_to_string(self.params.join("autodim")).unwrap()
    }
    fn brightness(&self) -> String {
        fs::read_to_string(self.bl.join("brightness")).unwrap()
    }
}
#[test]
fn t2_steps_are_not_scaled_onto_dim_and_closed_lid_can_turn_off() {
    let f = Fixture::new();
    let mut b = Backlight::open(&f.roots).unwrap();
    for n in [2, 1, 0, 2] {
        b.set_level(n).unwrap();
        assert_eq!(f.brightness(), format!("{n}\n"));
    }
    assert!(b.set_level(3).is_err());
    drop(b);
    assert_eq!(f.brightness(), "2\n");
}
#[test]
fn marker_survives_process_loss_and_restores_driver_setting() {
    let f = Fixture::new();
    let a = Autodim::new(&f.roots);
    let settings = a.settings().unwrap();
    assert_eq!(settings.autodim, Some(true));
    assert_eq!(settings.dim_seconds, Some(30));
    assert_eq!(settings.idle_seconds, Some(15));
    assert!(a.take_over().unwrap());
    assert_eq!(f.autodim(), "N\n");
    assert_eq!(
        fs::read_to_string(f.roots.run.join("autodim")).unwrap(),
        "Y\n"
    );
    assert!(Autodim::new(&f.roots).restore().unwrap());
    assert_eq!(f.autodim(), "Y\n");
    assert!(!f.roots.run.join("autodim").exists());
}
#[test]
fn failure_after_marker_before_parameter_write_keeps_recovery_record() {
    let f = Fixture::new();
    let a = Autodim::new(&f.roots);
    let prepared = a.prepare_take_over().unwrap().unwrap();
    assert_eq!(f.autodim(), "Y\n");
    assert_eq!(
        fs::read_to_string(f.roots.run.join("autodim")).unwrap(),
        "Y\n"
    );
    drop(prepared);
    assert!(Autodim::new(&f.roots).restore().unwrap());
    assert_eq!(f.autodim(), "Y\n");
    let prepared = a.prepare_take_over().unwrap().unwrap();
    let target = f.params.join("autodim");
    fs::remove_file(&target).unwrap();
    symlink("dim_timeout", &target).unwrap();
    assert!(prepared.disable().is_err());
    assert!(f.roots.run.join("autodim").exists());
    fs::remove_file(target).unwrap();
    fs::write(f.params.join("autodim"), "Y\n").unwrap();
    assert!(a.restore().unwrap());
}
#[test]
fn disabled_autodim_without_marker_stays_disabled_and_continuous_levels_match_v1() {
    let f = Fixture::new();
    fs::write(f.params.join("autodim"), "N\n").unwrap();
    assert!(!Autodim::new(&f.roots).take_over().unwrap());
    assert!(!f.roots.run.exists());
    for (max, expected) in [
        (1, [0, 1, 1]),
        (2, [0, 1, 2]),
        (255, [0, 1, 128]),
        (510, [0, 2, 256]),
    ] {
        for level in 0..=2 {
            assert_eq!(
                touchbard::backlight::level_value(max, level).unwrap(),
                expected[usize::from(level)]
            );
        }
    }
}
#[test]
fn untrusted_marker_and_runtime_cannot_disable_autodim() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    fs::create_dir(&f.roots.run).unwrap();
    fs::set_permissions(&f.roots.run, fs::Permissions::from_mode(0o777)).unwrap();
    let a = Autodim::new(&f.roots);
    assert!(a.take_over().is_err());
    assert_eq!(f.autodim(), "Y\n");
    fs::set_permissions(&f.roots.run, fs::Permissions::from_mode(0o755)).unwrap();
    let outside = f._temp.path().join("outside");
    fs::write(&outside, "Y\n").unwrap();
    symlink(&outside, f.roots.run.join("autodim")).unwrap();
    assert!(a.take_over().is_err());
    assert!(a.restore().is_err());
    assert_eq!(f.autodim(), "Y\n");
    assert_eq!(fs::read_to_string(outside).unwrap(), "Y\n");
}
struct Ops {
    trace: Vec<u8>,
}
impl touchbard::device::Operations for Ops {
    fn release_hardware(&mut self) -> std::io::Result<()> {
        self.trace.push(9);
        Ok(())
    }
    fn load_module(&mut self, _: &str) -> std::io::Result<()> {
        panic!("native must not modprobe")
    }
    fn select_config(&mut self, d: &touchbard::device::Device, v: u8) -> std::io::Result<()> {
        self.trace.push(v);
        d.write_config(v)
    }
}
#[test]
fn hand_back_without_marker_only_returns_usb_native() {
    let f = Fixture::new();
    fs::write(f.params.join("autodim"), "N\n").unwrap();
    let mut ops = Ops { trace: vec![] };
    hand_back_with(&f.roots, &mut ops).unwrap();
    assert_eq!(ops.trace, [9, 0, 1]);
    assert_eq!(
        fs::read_to_string(f.usb.join("bConfigurationValue")).unwrap(),
        "1\n"
    );
    assert_eq!(f.autodim(), "N\n");
    assert_eq!(f.brightness(), "0\n");
    assert!(!f.roots.run.join("autodim").exists());
}
#[test]
fn hand_back_with_marker_restores_autodim_and_lights_native_bar() {
    let f = Fixture::new();
    let a = Autodim::new(&f.roots);
    a.take_over().unwrap();
    hand_back_with(&f.roots, &mut Ops { trace: vec![] }).unwrap();
    assert_eq!(f.autodim(), "Y\n");
    assert_eq!(f.brightness(), "2\n");
    assert!(!f.roots.run.join("autodim").exists());
}
#[test]
fn broken_usb_discovery_does_not_prevent_autodim_return() {
    let f = Fixture::new();
    let a = Autodim::new(&f.roots);
    a.take_over().unwrap();
    fs::write(f.usb.join("bConfigurationValue"), "invalid\n").unwrap();
    assert!(hand_back_with(&f.roots, &mut Ops { trace: vec![] }).is_err());
    assert_eq!(f.autodim(), "Y\n");
    assert!(!f.roots.run.join("autodim").exists());
}
#[test]
fn missing_driver_keeps_marker_until_restoration_can_succeed() {
    let f = Fixture::new();
    let a = Autodim::new(&f.roots);
    a.take_over().unwrap();
    assert!(a.take_over().unwrap());
    assert_eq!(
        fs::read_to_string(f.roots.run.join("autodim")).unwrap(),
        "Y\n"
    );
    fs::remove_file(f.params.join("autodim")).unwrap();
    assert!(a.restore().is_err());
    assert!(f.roots.run.join("autodim").exists());
    fs::write(f.params.join("autodim"), "N\n").unwrap();
    assert!(a.restore().unwrap());
    assert_eq!(f.autodim(), "Y\n");
}
#[test]
fn backlight_class_symlink_cannot_escape_sysfs_devices() {
    let f = Fixture::new();
    let outside = f._temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("brightness"), "untouched").unwrap();
    let class = f.roots.sys.join("class/backlight/appletb_backlight");
    fs::remove_file(&class).unwrap();
    symlink(&outside, class).unwrap();
    assert!(Backlight::open(&f.roots).is_err());
    assert_eq!(
        fs::read_to_string(outside.join("brightness")).unwrap(),
        "untouched"
    );
}
