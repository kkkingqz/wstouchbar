use std::{
    cell::RefCell,
    collections::BTreeMap,
    io,
    os::{
        fd::{AsRawFd, BorrowedFd, OwnedFd},
        unix::net::UnixStream,
    },
    rc::Rc,
};
use touchbar_model::{KeyPhase, WatchKey};
use touchbard::keys::{Capabilities, InputKind, KeyBitmap, KeyQueries, KeyReader, watch_code};
#[derive(Clone, Default)]
struct Queries(Rc<RefCell<BTreeMap<i32, KeyBitmap>>>);
impl KeyQueries for Queries {
    fn query(&self, fd: BorrowedFd<'_>, _: Capabilities) -> io::Result<(KeyBitmap, Option<bool>)> {
        self.0
            .borrow()
            .get(&fd.as_raw_fd())
            .cloned()
            .map(|b| (b, None))
            .ok_or_else(|| io::Error::from_raw_os_error(19))
    }
}
fn pair() -> (OwnedFd, OwnedFd) {
    let (r, w) = UnixStream::pair().unwrap();
    r.set_nonblocking(true).unwrap();
    w.set_nonblocking(true).unwrap();
    (r.into(), w.into())
}
fn event(kind: u16, code: u16, value: i32) -> Vec<u8> {
    let mut b = vec![0; touchbard::input::EVENT_SIZE];
    let p = b.len() - 8;
    b[p..p + 2].copy_from_slice(&kind.to_ne_bytes());
    b[p + 2..p + 4].copy_from_slice(&code.to_ne_bytes());
    b[p + 4..].copy_from_slice(&value.to_ne_bytes());
    b
}
#[test]
fn virtual_pipe_wakes_reader_but_only_physical_bitmap_controls_fn() {
    futures_lite::future::block_on(async {
        let q = Queries::default();
        let mut reader = KeyReader::new(q.clone()).unwrap();
        let (physical, _pw) = pair();
        let pfd = physical.as_raw_fd();
        q.0.borrow_mut().insert(pfd, KeyBitmap::default());
        reader
            .insert(
                1,
                physical,
                InputKind::Physical,
                Capabilities {
                    keys: true,
                    lid: false,
                },
            )
            .unwrap();
        let (virtual_fd, vw) = pair();
        reader
            .insert(
                2,
                virtual_fd,
                InputKind::Virtual,
                Capabilities {
                    keys: true,
                    lid: false,
                },
            )
            .unwrap();
        assert!(reader.next().await.unwrap().changes.keys.is_empty());
        rustix::io::write(&vw, &event(1, 464, 1)).unwrap();
        let out = reader.next().await.unwrap();
        assert!(out.changes.activity);
        assert!(out.changes.keys.is_empty());
        q.0.borrow_mut()
            .get_mut(&pfd)
            .unwrap()
            .set(watch_code(WatchKey::Fn), true)
            .unwrap();
        rustix::io::write(&vw, &event(1, 30, 1)).unwrap();
        let out = reader.next().await.unwrap();
        assert_eq!(out.changes.keys.len(), 1);
        assert_eq!(out.changes.keys[0].phase, KeyPhase::Pressed);
        q.0.borrow_mut().remove(&pfd);
        rustix::io::write(&vw, &event(1, 464, 0)).unwrap();
        let out = reader.next().await.unwrap();
        assert_eq!(out.faults.len(), 1);
        assert_eq!(out.changes.keys[0].phase, KeyPhase::Released);
        // A vanished node is removed once, while other devices continue to work.
        rustix::io::write(&vw, &event(2, 0, 1)).unwrap();
        let out = reader.next().await.unwrap();
        assert!(out.faults.is_empty());
        assert!(out.changes.activity);
    });
}
#[test]
fn eof_releases_and_own_keyboard_cannot_cause_activity() {
    futures_lite::future::block_on(async {
        let q = Queries::default();
        let mut reader = KeyReader::new(q.clone()).unwrap();
        let (fd, writer) = pair();
        let mut bits = KeyBitmap::default();
        bits.set(464, true).unwrap();
        q.0.borrow_mut().insert(fd.as_raw_fd(), bits);
        reader
            .insert(
                1,
                fd,
                InputKind::Physical,
                Capabilities {
                    keys: true,
                    lid: false,
                },
            )
            .unwrap();
        assert_eq!(reader.next().await.unwrap().changes.keys.len(), 1);
        drop(writer);
        let out = reader.next().await.unwrap();
        assert_eq!(out.changes.keys[0].phase, KeyPhase::Released);
        assert_eq!(out.faults.len(), 1);
        let (fd, writer) = pair();
        reader
            .insert(
                2,
                fd,
                InputKind::Own,
                Capabilities {
                    keys: true,
                    lid: false,
                },
            )
            .unwrap();
        reader.next().await.unwrap();
        rustix::io::write(&writer, &event(1, 63, 1)).unwrap();
        assert!(!reader.next().await.unwrap().changes.activity);
    });
}
#[test]
fn syn_dropped_recovers_from_bitmap_and_idle_reader_has_no_tick() {
    futures_lite::future::block_on(async {
        let q = Queries::default();
        let mut reader = KeyReader::new(q.clone()).unwrap();
        let (fd, w) = pair();
        let raw = fd.as_raw_fd();
        q.0.borrow_mut().insert(raw, KeyBitmap::default());
        reader
            .insert(
                1,
                fd,
                InputKind::Physical,
                Capabilities {
                    keys: true,
                    lid: false,
                },
            )
            .unwrap();
        reader.next().await.unwrap();
        // The wire claims release; the physical ioctl bitmap is authoritative, including
        // after a lost kernel event batch.
        q.0.borrow_mut()
            .get_mut(&raw)
            .unwrap()
            .set(464, true)
            .unwrap();
        rustix::io::write(&w, &event(0, 3, 0)).unwrap();
        let out = reader.next().await.unwrap();
        assert_eq!(out.changes.keys[0].phase, KeyPhase::Pressed);
        let ready = futures_lite::future::or(
            async {
                reader.next().await.unwrap();
                true
            },
            async {
                async_io::Timer::after(std::time::Duration::from_millis(25)).await;
                false
            },
        )
        .await;
        assert!(!ready, "idle input collection woke without an event");
    });
}
