use std::{
    collections::VecDeque,
    io::{self, Write},
    os::{fd::BorrowedFd, unix::net::UnixStream},
    sync::Mutex,
};
use touchbar_model::TouchPhase;
use touchbard::input::{
    Axis, AxisTransform, EVENT_SIZE, Geometry, InputEvent, RawSlot, SnapshotSource, TouchReader,
    decode_events,
};
struct Fake {
    snapshots: Mutex<VecDeque<(Vec<RawSlot>, usize)>>,
}
impl SnapshotSource for Fake {
    fn snapshot(&self, _fd: BorrowedFd<'_>, _count: usize) -> io::Result<(Vec<RawSlot>, usize)> {
        self.snapshots
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| io::Error::other("snapshot unavailable"))
    }
}
fn bytes(events: &[InputEvent]) -> Vec<u8> {
    events
        .iter()
        .flat_map(|e| {
            let mut b = vec![0; EVENT_SIZE];
            let p = EVENT_SIZE - 8;
            b[p..p + 2].copy_from_slice(&e.kind.to_ne_bytes());
            b[p + 2..p + 4].copy_from_slice(&e.code.to_ne_bytes());
            b[p + 4..].copy_from_slice(&e.value.to_ne_bytes());
            b
        })
        .collect()
}
fn ev(kind: u16, code: u16, value: i32) -> InputEvent {
    InputEvent { kind, code, value }
}
fn empty() -> RawSlot {
    RawSlot {
        tracking: None,
        x: 0,
        y: 0,
    }
}
fn reader(snapshots: Vec<(Vec<RawSlot>, usize)>) -> (TouchReader<Fake>, UnixStream) {
    let (fd, write) = UnixStream::pair().unwrap();
    let g = Geometry::new(
        2008,
        60,
        Axis {
            minimum: 0,
            maximum: 2000,
        },
        Axis {
            minimum: 0,
            maximum: 60,
        },
        AxisTransform::Identity,
    )
    .unwrap();
    (
        TouchReader::from_fd(
            fd.into(),
            g,
            2,
            Fake {
                snapshots: Mutex::new(snapshots.into()),
            },
        )
        .unwrap(),
        write,
    )
}
#[test]
fn reader_delivers_cancel_on_eof_and_resync_fault_without_losing_prior_edges() {
    futures_lite::future::block_on(async {
        let (mut r, mut w) = reader(vec![(vec![empty(); 2], 0)]);
        w.write_all(&bytes(&[ev(3, 0x39, 0), ev(0, 0, 0)])).unwrap();
        let batch = r.next().await;
        assert!(batch.fault.is_none());
        assert_eq!(batch.events[0].phase, TouchPhase::Down);
        w.write_all(&bytes(&[ev(0, 3, 0), ev(3, 0x39, 999), ev(0, 0, 0)]))
            .unwrap();
        let batch = r.next().await;
        assert!(batch.fault.is_some());
        assert_eq!(batch.events.len(), 1);
        assert_eq!(batch.events[0].phase, TouchPhase::Cancel);
        let (mut r, mut w) = reader(vec![(vec![empty(); 2], 0)]);
        w.write_all(&bytes(&[ev(3, 0x39, 7), ev(0, 0, 0)])).unwrap();
        r.next().await;
        drop(w);
        let batch = r.next().await;
        assert!(batch.fault.is_some());
        assert_eq!(batch.events[0].tracking, 7);
        assert_eq!(batch.events[0].phase, TouchPhase::Cancel);
    });
}
#[test]
fn reader_restores_initial_active_slots_and_uses_query_only_after_dropped_report() {
    futures_lite::future::block_on(async {
        let (mut r, mut w) = reader(vec![
            (
                vec![
                    RawSlot {
                        tracking: Some(0),
                        x: 2000,
                        y: 60,
                    },
                    empty(),
                ],
                0,
            ),
            (vec![empty(); 2], 1),
        ]);
        let first = r.next().await;
        assert_eq!(first.events.len(), 1);
        assert_eq!(first.events[0].tracking, 0);
        w.write_all(&bytes(&[ev(0, 3, 0), ev(3, 0x39, 7)])).unwrap();
        let first = r.next().await;
        assert!(first.events.is_empty());
        assert!(first.fault.is_none());
        w.write_all(&bytes(&[ev(0, 0, 0)])).unwrap();
        let batch = r.next().await;
        assert!(batch.fault.is_none());
        assert_eq!(batch.events.len(), 1);
        assert_eq!(batch.events[0].phase, TouchPhase::Cancel);
    });
}
#[test]
fn native_endian_input_events_are_checked_and_partial_records_rejected() {
    let events = [ev(3, 0x39, i32::MAX), ev(3, 0x39, -1)];
    let b = bytes(&events);
    assert_eq!(decode_events(&b).unwrap(), events);
    for n in 1..EVENT_SIZE {
        assert!(decode_events(&b[..n]).is_err());
    }
}
proptest::proptest! {#[test] fn arbitrary_evdev_bytes_never_panic(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(),0..2048)) {let _=decode_events(&bytes);}}

#[test]
fn production_open_rejects_non_evdev_and_symlink_paths_without_grabbing() {
    use std::{fs, os::unix::fs::symlink};
    use touchbard::{
        device::{Roots, discover},
        input::TouchInput,
    };
    let tmp = tempfile::tempdir().unwrap();
    let roots = Roots {
        sys: tmp.path().join("sys"),
        dev: tmp.path().join("dev"),
        run: tmp.path().join("run"),
    };
    let usb = roots.sys.join("bus/usb/devices/1-1");
    fs::create_dir_all(&usb).unwrap();
    fs::write(usb.join("idVendor"), "05ac").unwrap();
    fs::write(usb.join("idProduct"), "8302").unwrap();
    let device = discover(&roots).unwrap().unwrap().device;
    let file = tmp.path().join("event0");
    fs::write(&file, "not evdev").unwrap();
    assert!(TouchInput::open(&roots, &device, &file, 2008, 60).is_err());
    let link = tmp.path().join("link");
    symlink(&file, &link).unwrap();
    assert!(TouchInput::open(&roots, &device, &link, 2008, 60).is_err());
    assert!(
        TouchInput::open(&roots, &device, std::path::Path::new("/dev/null"), 2008, 60).is_err()
    );
}

#[test]
fn valid_down_and_local_fault_in_same_read_return_both_down_and_cancel() {
    futures_lite::future::block_on(async {
        let (mut r, mut w) = reader(vec![(vec![empty(); 2], 0)]);
        w.write_all(&bytes(&[ev(3, 0x39, 0), ev(0, 0, 0), ev(3, 0x2f, 99)]))
            .unwrap();
        let batch = r.next().await;
        assert!(batch.fault.is_some());
        assert_eq!(
            batch
                .events
                .iter()
                .map(|t| (t.tracking, t.phase))
                .collect::<Vec<_>>(),
            vec![(0, TouchPhase::Down), (0, TouchPhase::Cancel)]
        );
    });
}
