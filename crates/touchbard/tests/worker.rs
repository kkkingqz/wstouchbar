use std::{
    sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use touchbar_model::{Timestamp, TouchPhase};
use touchbar_protocol::{Mode, SwapchainHeader, XRGB8888};
use touchbard::{
    Command, Daemon, DisplayInfo, Event, Session,
    config::Config,
    worker::{SubmitError, Worker},
};
fn displaying() -> Daemon {
    let mut d = Daemon::new(Config::default(), false, Timestamp(0)).unwrap();
    d.handle(
        Timestamp(0),
        Event::Session(Some(Session {
            uid: 1000,
            id: "s".into(),
            user_display: Some("s".into()),
            active: true,
            state: "active".into(),
            class: "user".into(),
            kind: "wayland".into(),
            locked: false,
        })),
    );
    d.handle(Timestamp(0), Event::Device(Some(1)));
    let generation = d.mode().generation();
    d.handle(Timestamp(0), Event::NativeSafe { generation });
    d.handle(
        Timestamp(0),
        Event::Hello {
            connection: 1,
            uid: 1000,
            version: 2,
        },
    );
    d.advance(Timestamp(2000));
    let generation = d.mode().generation();
    d.handle(
        Timestamp(2001),
        Event::DisplayReady {
            generation,
            info: DisplayInfo {
                header: SwapchainHeader {
                    logical_width: 20,
                    logical_height: 4,
                    physical_width: 20,
                    physical_height: 4,
                    format: XRGB8888,
                    pitch: 80,
                    buffer_size: 320,
                    buffer_count: 3,
                    frame_damage: true,
                },
                caps: 1,
                width_mm100: 2000,
                height_mm100: 400,
            },
        },
    );
    while d.pending(1).is_some() {
        d.handle(Timestamp(2001), Event::Sent { connection: 1 });
    }
    d
}
#[test]
fn two_second_operation_does_not_delay_touch_recovery_or_sleep() {
    futures_lite::future::block_on(async {
        let (tx, rx) = async_channel::bounded(1);
        let mut worker = Worker::new(move |value| {
            tx.send_blocking(()).unwrap();
            std::thread::sleep(Duration::from_secs(2));
            value
        })
        .unwrap();
        worker.try_submit(1, 42).unwrap();
        rx.recv().await.unwrap();
        let mut d = displaying();
        let start = Instant::now();
        d.handle(
            Timestamp(2100),
            Event::Touch {
                tracking: 0,
                phase: TouchPhase::Down,
                x: 1,
                y: 1,
            },
        );
        assert!(
            d.handle(Timestamp(2101), Event::SetMode(Mode::Native))
                .contains(&Command::WriteRecovery(true))
        );
        d.handle(Timestamp(2102), Event::PrepareForSleep(true));
        assert!(d.sleeping());
        assert!(start.elapsed() < Duration::from_millis(100));
        let completion = worker.next().await.unwrap();
        assert_eq!(completion.generation, 1);
        assert_eq!(completion.value, 42);
    });
}
#[test]
fn queue_is_four_and_new_generation_discards_queued_stale_work() {
    futures_lite::future::block_on(async {
        let barrier = Arc::new(Barrier::new(2));
        let work = barrier.clone();
        let (tx, rx) = async_channel::bounded(1);
        let mut worker = Worker::new(move |value| {
            if value == 0 {
                tx.send_blocking(()).unwrap();
                work.wait();
            }
            value
        })
        .unwrap();
        worker.try_submit(1, 0).unwrap();
        rx.recv().await.unwrap();
        for value in 1..=4 {
            worker.try_submit(1, value).unwrap();
        }
        assert_eq!(worker.try_submit(1, 5), Err(SubmitError::Full));
        worker.try_submit(2, 99).unwrap();
        assert_eq!(worker.try_submit(1, 6), Err(SubmitError::Stale));
        barrier.wait();
        let reply = worker.next().await.unwrap();
        assert_eq!((reply.generation, reply.value), (2, 99));
    });
}
#[test]
fn dropping_worker_never_runs_queued_operations_after_an_active_one() {
    let barrier = Arc::new(Barrier::new(2));
    let work = barrier.clone();
    let count = Arc::new(AtomicUsize::new(0));
    let executed = count.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let (done, finished) = std::sync::mpsc::channel();
    struct Finished(std::sync::mpsc::Sender<()>);
    impl Drop for Finished {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    let finished_guard = Finished(done);
    let mut worker = Worker::new(move |value| {
        let _ = &finished_guard;
        if value == 0 {
            tx.send(()).unwrap();
            work.wait();
        } else {
            executed.fetch_add(1, Ordering::SeqCst);
        }
        value
    })
    .unwrap();
    worker.try_submit(1, 0).unwrap();
    rx.recv_timeout(Duration::from_secs(1)).unwrap();
    worker.try_submit(1, 1).unwrap();
    drop(worker);
    barrier.wait();
    finished.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

// Cancelling mode work must never lose or reorder ownership/persistence operations.
#[test]
fn committed_jobs_and_replies_survive_generation_changes_in_fifo_order() {
    futures_lite::future::block_on(async {
        let barrier = Arc::new(Barrier::new(2));
        let work = barrier.clone();
        let (tx, rx) = async_channel::bounded(8);
        let mut worker = Worker::new(move |value| {
            tx.send_blocking(value).unwrap();
            if value == 10 {
                work.wait();
            }
            value
        })
        .unwrap();
        worker.try_submit_committed(100, 10).unwrap();
        assert_eq!(rx.recv().await.unwrap(), 10);
        worker.try_submit(1, 11).unwrap();
        worker.try_submit_committed(101, 12).unwrap();
        worker.try_submit_committed(102, 13).unwrap();
        worker.try_submit(1, 14).unwrap();
        assert_eq!(worker.try_submit_committed(103, 15), Err(SubmitError::Full));
        worker.try_submit(2, 20).unwrap();
        barrier.wait();
        // Read execution notices first: completions already queued when generation changes.
        for value in [12, 13, 20] {
            assert_eq!(rx.recv().await.unwrap(), value);
        }
        worker.try_submit(3, 30).unwrap();
        let mut results = Vec::new();
        for _ in 0..4 {
            let c = worker.next().await.unwrap();
            results.push((c.generation, c.value));
        }
        assert_eq!(results, [(100, 10), (101, 12), (102, 13), (3, 30)]);
    });
}
