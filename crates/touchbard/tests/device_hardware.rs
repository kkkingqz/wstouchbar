//! Run only after explicit stand approval; ordinary tests never execute these writes.
use std::{io, time::Duration};
use touchbar_protocol::Mode;
use touchbard::device::{
    ExclusiveDevice, KernelMonitor, Roots, Snapshot, SwitchPlan, SystemOps, discover,
};
async fn wait(
    roots: &Roots,
    monitor: &KernelMonitor,
    seconds: u64,
    predicate: impl Fn(&Snapshot) -> bool,
) -> io::Result<()> {
    futures_lite::future::or(
        async {
            loop {
                if discover(roots)?.as_ref().is_some_and(&predicate) {
                    return Ok(());
                }
                monitor.next().await?;
            }
        },
        async {
            async_io::Timer::after(Duration::from_secs(seconds)).await;
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "hardware mode readiness timed out",
            ))
        },
    )
    .await
}
#[test]
#[ignore = "requires root, real USB 05ac:8302 and explicit stage-4 hardware approval"]
fn env_hardware_usb_mode_roundtrip() -> io::Result<()> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "hardware test requires root",
        ));
    }
    let roots = Roots::default();
    let _exclusive = ExclusiveDevice::acquire(&roots)?;
    let device = discover(&roots)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "USB 05ac:8302 not found"))?
        .device;
    // All potentially blocking module/USB operations run off the main test thread.
    std::thread::spawn(move || {
        let monitor = KernelMonitor::new()?;
        let mut ops = SystemOps { release: || Ok(()) };
        let result = SwitchPlan::new(device.clone(), Mode::Display)
            .execute(&mut ops)
            .and_then(|()| {
                futures_lite::future::block_on(wait(&roots, &monitor, 5, |s| s.display.is_some()))
            });
        // Always attempt native restoration, including when display failed.
        let restore = SwitchPlan::new(device, Mode::Native)
            .execute(&mut ops)
            .and_then(|()| {
                futures_lite::future::block_on(wait(&roots, &monitor, 10, |s| {
                    s.native_safe() && s.native.is_some()
                }))
            });
        result.and(restore)
    })
    .join()
    .map_err(|_| io::Error::other("hardware test worker panicked"))?
}
