use touchbard::drm::{CommitMode, Frame, Operation, Reply, device_worker};
#[test]
fn device_worker_rejects_frames_without_a_surface_and_release_is_idempotent() {
    futures_lite::future::block_on(async {
        let mut worker = device_worker(()).unwrap();
        worker
            .try_submit(
                1,
                Operation::Present {
                    frame: Frame { index: 0, frame: 1 },
                    damage: vec![],
                    mode: CommitMode::Nonblocking,
                },
            )
            .unwrap();
        let reply = worker.next().await.unwrap().value;
        assert!(matches!(reply, Err(e) if e.kind() == std::io::ErrorKind::NotConnected));
        for _ in 0..2 {
            worker.try_submit(1, Operation::Release).unwrap();
            assert!(matches!(
                worker.next().await.unwrap().value,
                Ok(Reply::Done)
            ));
        }
    });
}
#[test]
fn probe_preflight_rejects_a_regular_file_without_changing_it() {
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(), b"not a DRM node").unwrap();
    assert!(touchbard::drm::Surface::check_master(f.path()).is_err());
    assert_eq!(std::fs::read(f.path()).unwrap(), b"not a DRM node");
}
