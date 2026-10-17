use std::{
    cell::RefCell,
    io::{self, Write},
    rc::Rc,
};
use touchbar_model::{KeyPhase, SystemKey};
use touchbard::{input::decode_events, keys::SystemKeys};
#[derive(Default)]
struct Data {
    bytes: Vec<u8>,
    fail: bool,
    closed: bool,
}
struct Writer(Rc<RefCell<Data>>);
impl Write for Writer {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let mut d = self.0.borrow_mut();
        if d.fail {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        d.bytes.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        self.0.borrow_mut().closed = true;
    }
}
#[test]
fn balanced_system_keys_release_on_shutdown_and_drop() {
    let d = Rc::new(RefCell::new(Data::default()));
    {
        let mut keys = SystemKeys::from_writer(Writer(d.clone()));
        keys.send(SystemKey::F5, KeyPhase::Pressed).unwrap();
        keys.send(SystemKey::F5, KeyPhase::Pressed).unwrap();
        keys.send(SystemKey::VolumeUp, KeyPhase::Pressed).unwrap();
        keys.release_all().unwrap();
        keys.send(SystemKey::Esc, KeyPhase::Pressed).unwrap();
    }
    let data = d.borrow();
    assert!(data.closed);
    let ev = decode_events(&data.bytes).unwrap();
    let keys: Vec<_> = ev
        .iter()
        .filter(|e| e.kind == 1)
        .map(|e| (e.code, e.value))
        .collect();
    assert_eq!(keys, [(63, 1), (115, 1), (63, 0), (115, 0), (1, 1), (1, 0)]);
    assert_eq!(ev.iter().filter(|e| e.kind == 0).count(), keys.len());
}
#[test]
fn write_failure_destroys_device_and_does_not_leave_unreleasable_keys() {
    let d = Rc::new(RefCell::new(Data::default()));
    let mut keys = SystemKeys::from_writer(Writer(d.clone()));
    keys.send(SystemKey::F5, KeyPhase::Pressed).unwrap();
    d.borrow_mut().fail = true;
    assert!(keys.release_all().is_err());
    assert!(d.borrow().closed);
    assert!(keys.send(SystemKey::Esc, KeyPhase::Pressed).is_err());
}
