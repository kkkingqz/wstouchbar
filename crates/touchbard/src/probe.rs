//! Explicit root-only stand diagnostic. Ordinary test runs never call `run`.
use crate::{
    device::{Device, ExclusiveDevice, KernelMonitor, Roots, Snapshot, SwitchPlan, discover},
    drm::{
        self, CommitMode, Completion, DeviceWorker, Frame, Operation, Reply, Submission,
        dimensions::PhysicalSize,
    },
};
use async_io::{Async, Timer};
use async_signal::{Signal, Signals};
use futures_lite::StreamExt;
use serde::Serialize;
use std::{
    future::Future,
    io,
    os::fd::OwnedFd,
    time::{Duration, Instant},
};
use touchbar_protocol::{DamageRect, Mode};
#[derive(Serialize)]
pub struct CommitReport {
    pub requested: &'static str,
    pub actual: &'static str,
    pub requested_damage: bool,
    pub actual_damage: bool,
    pub commit_us: u32,
}
#[derive(Serialize)]
pub struct Report {
    pub model: Option<String>,
    pub logical_pixels: (u32, u32),
    pub physical_pixels: (u32, u32),
    pub physical_size: PhysicalSize,
    pub format: u32,
    pub buffer_count: u16,
    pub pitch: u32,
    pub buffer_size: u64,
    pub prime_cloexec: bool,
    pub damage_supported: bool,
    pub commits: Vec<CommitReport>,
    pub native_safe: bool,
    pub native_hid_ready: bool,
}
async fn timeout<T>(duration: Duration, f: impl Future<Output = io::Result<T>>) -> io::Result<T> {
    futures_lite::future::or(f, async {
        Timer::after(duration).await;
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "device operation timed out",
        ))
    })
    .await
}
async fn interrupted<T>(
    signals: &mut Signals,
    work: impl Future<Output = io::Result<T>>,
) -> io::Result<T> {
    futures_lite::future::or(work, async {
        signals.next().await.transpose()?;
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "probe interrupted; restoring native",
        ))
    })
    .await
}
async fn request(
    worker: &mut DeviceWorker,
    generation: u64,
    operation: Operation,
) -> io::Result<Reply> {
    worker
        .try_submit(generation, operation)
        .map_err(|e| io::Error::other(format!("device queue: {e:?}")))?;
    worker
        .next()
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "device worker stopped"))?
        .value
}
async fn wait_snapshot(
    roots: &Roots,
    device: &Device,
    monitor: &KernelMonitor,
    predicate: impl Fn(&Snapshot) -> bool,
) -> io::Result<Snapshot> {
    loop {
        let s = discover(roots)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Touch Bar disconnected"))?;
        if s.device != *device {
            return Err(io::Error::other("Touch Bar was replaced"));
        }
        if predicate(&s) {
            return Ok(s);
        }
        monitor.next().await?;
    }
}
async fn wait_stable(roots: &Roots, device: &Device, monitor: &KernelMonitor) -> io::Result<()> {
    // Relevant events wake a rescan, but unrelated USB traffic does not reset stability.
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if discover(roots)?.is_none_or(|s| s.device != *device) {
            return Err(io::Error::other(
                "Touch Bar changed while settling; retry probe",
            ));
        }
        let done = futures_lite::future::or(async { monitor.next().await.map(|_| false) }, async {
            Timer::at(deadline).await;
            Ok(true)
        })
        .await?;
        if done {
            return Ok(());
        }
    }
}
async fn completed(
    worker: &mut DeviceWorker,
    events: &Async<OwnedFd>,
    frame: Frame,
    damage: Vec<DamageRect>,
    mode: CommitMode,
) -> io::Result<Completion> {
    match request(
        worker,
        1,
        Operation::Present {
            frame,
            damage,
            mode,
        },
    )
    .await?
    {
        Reply::Submitted(Submission::Complete(c)) => Ok(c),
        Reply::Submitted(Submission::Pending) => loop {
            let mut bytes = [0u8; 4096];
            let n = events
                .read_with(|fd| rustix::io::read(fd, &mut bytes).map_err(io::Error::from))
                .await?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "DRM event descriptor closed",
                ));
            }
            for event in drm::events::decode_events(&bytes[..n])? {
                if let Reply::Completed(Some(c)) =
                    request(worker, 1, Operation::PageFlip(event)).await?
                {
                    return Ok(c);
                }
            }
        },
        _ => Err(io::Error::other("unexpected device reply to commit")),
    }
}
async fn display(
    worker: &mut DeviceWorker,
    roots: &Roots,
    device: &Device,
    monitor: &KernelMonitor,
) -> io::Result<Report> {
    let model = std::fs::read_to_string(roots.sys.join("class/dmi/id/product_name"))
        .ok()
        .map(|s| s.trim().to_string());
    let exports = timeout(Duration::from_secs(5), async {
        request(
            worker,
            1,
            Operation::Switch(SwitchPlan::new(device.clone(), Mode::Display)),
        )
        .await?;
        let nodes = wait_snapshot(roots, device, monitor, |s| s.display.is_some())
            .await?
            .display
            .ok_or_else(|| io::Error::other("display disappeared"))?;
        match request(
            worker,
            1,
            Operation::Open {
                card: nodes.card,
                dmi_model: model.clone(),
            },
        )
        .await?
        {
            Reply::Opened(exports) => Ok(exports),
            _ => Err(io::Error::other("unexpected device reply to open")),
        }
    })
    .await?;
    let header = exports.header;
    let prime_cloexec = exports.buffers.iter().all(|fd| {
        rustix::io::fcntl_getfd(fd).is_ok_and(|f| f.contains(rustix::io::FdFlags::CLOEXEC))
    });
    if !prime_cloexec {
        return Err(io::Error::other("PRIME descriptor lacks CLOEXEC"));
    }
    let events = Async::new(exports.events)?;
    let mut report = Report {
        model,
        logical_pixels: (header.logical_width, header.logical_height),
        physical_pixels: (header.physical_width, header.physical_height),
        physical_size: exports.physical_size,
        format: header.format,
        buffer_count: header.buffer_count,
        pitch: header.pitch,
        buffer_size: header.buffer_size,
        prime_cloexec,
        damage_supported: header.frame_damage,
        commits: Vec::new(),
        native_safe: false,
        native_hid_ready: false,
    };
    let patch = DamageRect {
        x: 0,
        y: 0,
        width: u16::try_from((header.physical_width / 2).max(1)).expect("validated dimensions"),
        height: u16::try_from((header.physical_height / 8).max(1)).expect("validated dimensions"),
    };
    for (number, index, mode, partial) in [
        (1, 0, CommitMode::Nonblocking, false),
        (2, 1, CommitMode::Nonblocking, true),
        (3, 2, CommitMode::Blocking, false),
        (4, 0, CommitMode::Blocking, true),
    ] {
        request(
            worker,
            1,
            Operation::PaintProbe {
                index,
                patch: partial.then_some(patch),
            },
        )
        .await?;
        let c = timeout(
            Duration::from_secs(5),
            completed(
                worker,
                &events,
                Frame {
                    index,
                    frame: number,
                },
                if partial { vec![patch] } else { vec![] },
                mode,
            ),
        )
        .await?;
        report.commits.push(CommitReport {
            requested: if mode == CommitMode::Blocking {
                "blocking"
            } else {
                "nonblocking"
            },
            actual: if c.mode == CommitMode::Blocking {
                "blocking"
            } else {
                "nonblocking"
            },
            requested_damage: partial,
            actual_damage: partial && header.frame_damage,
            commit_us: c.commit_us,
        });
    }
    // The local event fd and all PRIME exports close before the native worker operation.
    Ok(report)
}
pub async fn run() -> io::Result<Report> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "touchbard probe requires root",
        ));
    }
    let mut signals = Signals::new([Signal::Int, Signal::Term, Signal::Hup])?;
    let roots = Roots::default();
    let exclusive = ExclusiveDevice::acquire(&roots)?;
    let monitor = KernelMonitor::new()?; // Subscribe before the first discovery/write.
    let snapshot = discover(&roots)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "USB 05ac:8302 not found"))?;
    let device = snapshot.device.clone();
    interrupted(&mut signals, wait_stable(&roots, &device, &monitor)).await?;
    let snapshot = discover(&roots)?
        .filter(|s| s.device == device)
        .ok_or_else(|| io::Error::other("Touch Bar changed before probe"))?;
    let mut worker = drm::device_worker(exclusive)?;
    // Refuse another DRM owner before cycling USB. The daemon lock is checked above.
    if let Some(display) = snapshot.display {
        timeout(
            Duration::from_secs(5),
            request(&mut worker, 1, Operation::CheckMaster(display.card)),
        )
        .await?;
    } else if snapshot.drm_bound {
        return Err(io::Error::other(
            "DRM is bound but not ready; stop other hardware owners before probe",
        ));
    }
    let result = interrupted(
        &mut signals,
        timeout(
            Duration::from_secs(30),
            display(&mut worker, &roots, &device, &monitor),
        ),
    )
    .await;
    // Higher generation discards queued/stale display work, including after a timeout.
    let restored = timeout(Duration::from_secs(3), async {
        request(
            &mut worker,
            2,
            Operation::Switch(SwitchPlan::new(device.clone(), Mode::Native)),
        )
        .await?;
        wait_snapshot(&roots, &device, &monitor, Snapshot::native_safe).await
    })
    .await;
    match restored {
        Err(e) => Err(io::Error::other(format!(
            "native restoration failed: {e}; display result: {}",
            result
                .as_ref()
                .err()
                .map(ToString::to_string)
                .unwrap_or_else(|| "ok".into())
        ))),
        Ok(_) => {
            let hid = timeout(
                Duration::from_secs(10),
                wait_snapshot(&roots, &device, &monitor, |s| s.native.is_some()),
            )
            .await
            .is_ok();
            let mut report = result.map_err(|e| {
                io::Error::other(format!(
                    "{e}; native restored (HID {})",
                    if hid { "ready" } else { "missing" }
                ))
            })?;
            report.native_safe = true;
            report.native_hid_ready = hid;
            Ok(report)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupt_stops_display_work_so_native_restoration_can_run() {
        futures_lite::future::block_on(async {
            let mut signals = async_signal::Signals::new([async_signal::Signal::Int]).unwrap();
            rustix::process::kill_process(rustix::process::getpid(), rustix::process::Signal::INT)
                .unwrap();
            let result = timeout(
                Duration::from_secs(1),
                interrupted(&mut signals, std::future::pending::<io::Result<()>>()),
            )
            .await;
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
        });
    }
}
