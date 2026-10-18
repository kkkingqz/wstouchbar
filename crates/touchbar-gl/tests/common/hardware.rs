//! Explicit stand gate only. The parent owns USB/DRM; GPU work runs in a bounded child.
#![allow(unsafe_code)]
use async_signal::{Signal, Signals};
use futures_lite::{StreamExt, future::or};
use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    os::unix::process::CommandExt,
    process::{Child, Command},
    time::{Duration, Instant},
};
use touchbar_protocol::{self as wire, DaemonPacket, Mode, SwapchainHeader};
use touchbard::{
    device::{self, Device, ExclusiveDevice, KernelMonitor, Roots, Snapshot, SwitchPlan},
    drm::{self, DeviceWorker, Operation, Reply},
};
const CHILD_FD: &str = "WSTOUCHBAR_HARDWARE_IMPORT_CHILD_FD";
async fn bounded<T>(
    seconds: u64,
    work: impl std::future::Future<Output = io::Result<T>>,
) -> io::Result<T> {
    or(work, async {
        async_io::Timer::after(Duration::from_secs(seconds)).await;
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "hardware import deadline expired",
        ))
    })
    .await
}
async fn request(worker: &mut DeviceWorker, generation: u64, op: Operation) -> io::Result<Reply> {
    worker
        .try_submit(generation, op)
        .map_err(|e| io::Error::other(format!("device queue: {e:?}")))?;
    worker
        .next()
        .await
        .map_err(|_| io::Error::other("device worker closed"))?
        .value
}
async fn snapshot(
    roots: &Roots,
    selected: &Device,
    monitor: &KernelMonitor,
    ready: impl Fn(&Snapshot) -> bool,
) -> io::Result<Snapshot> {
    loop {
        let s = device::discover(roots)?
            .filter(|s| s.device == *selected)
            .ok_or_else(|| io::Error::other("selected Touch Bar disappeared"))?;
        if ready(&s) {
            return Ok(s);
        }
        monitor.next().await?;
    }
}
struct GpuChild(Option<Child>);
impl Drop for GpuChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            // Never block native restoration on a stuck GPU child; reap once it exits.
            let _ = std::thread::Builder::new()
                .name("gpu-test-reaper".into())
                .spawn(move || {
                    let _ = child.wait();
                });
        }
    }
}
async fn gpu_child(name: &str, exports: drm::Exports) -> io::Result<()> {
    use rustix::net::{self, AddressFamily, SocketFlags, SocketType};
    let (parent, end) = net::socketpair(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC,
        None,
    )?;
    let source = end.as_raw_fd();
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["--exact", name, "--ignored", "--nocapture"])
        .env(CHILD_FD, "3")
        .env_remove("LIBGL_ALWAYS_SOFTWARE");
    // SAFETY: post-fork hook uses only async-signal-safe dup2/fcntl. All unrelated
    // daemon descriptors are CLOEXEC, including DRM master/event descriptors.
    unsafe {
        command.pre_exec(move || {
            if source != 3 && libc::dup2(source, 3) < 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::fcntl(3, libc::F_SETFD, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = GpuChild(Some(command.spawn()?));
    drop(end);
    let pid = child.0.as_ref().expect("spawned child").id();
    // SAFETY: pidfd_open creates an owned CLOEXEC fd for this still-unreaped child.
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0u32) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(i32::try_from(raw).map_err(io::Error::other)?) };
    let exited = async_io::Async::new(fd)?;
    // Only the three shared PRIME buffers leave this parent, never DRM master.
    wire::send_swapchain(&parent, exports.header, &exports.buffers)?;
    exited.readable().await?;
    let status = child
        .0
        .as_mut()
        .expect("live child")
        .try_wait()?
        .ok_or_else(|| io::Error::other("pidfd ready before child exit"))?;
    child.0.take();
    if !status.success() {
        return Err(io::Error::other(format!(
            "GPU import child failed: {status}"
        )));
    }
    Ok(())
}
pub fn child_buffers() -> io::Result<Option<(SwapchainHeader, Vec<OwnedFd>)>> {
    let Some(value) = std::env::var_os(CHILD_FD) else {
        return Ok(None);
    };
    if value != "3" {
        return Err(io::Error::other("invalid private GPU child descriptor"));
    }
    // SAFETY: our parent's pre-exec hook uniquely passes its socket as fd 3.
    let fd = unsafe { OwnedFd::from_raw_fd(3) };
    let DaemonPacket::Swapchain { header, buffers } = wire::recv_daemon(&fd)? else {
        return Err(io::Error::other("expected private swapchain packet"));
    };
    Ok(Some((header, buffers)))
}
pub fn with_buffers(child_name: &str) -> io::Result<()> {
    if !rustix::process::geteuid().is_root() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "hardware import test requires root and explicit stand approval",
        ));
    }
    device::isolation::require_installed()?;
    futures_lite::future::block_on(async {
        let mut signals = Signals::new([Signal::Term, Signal::Int, Signal::Hup])?;
        let roots = Roots::default();
        let exclusive = ExclusiveDevice::acquire(&roots)?;
        let monitor = KernelMonitor::new()?;
        let selected = device::discover(&roots)?
            .ok_or_else(|| io::Error::other("Touch Bar unavailable"))?
            .device;
        let stable = Instant::now() + Duration::from_secs(2);
        or(
            async {
                loop {
                    snapshot(&roots, &selected, &monitor, |_| true).await?;
                    let done = or(async { monitor.next().await.map(|_| false) }, async {
                        async_io::Timer::at(stable).await;
                        Ok(true)
                    })
                    .await?;
                    if done {
                        break;
                    }
                }
                Ok(())
            },
            async {
                let _ = signals.next().await;
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "hardware preflight interrupted",
                ))
            },
        )
        .await?;
        let before = snapshot(&roots, &selected, &monitor, |_| true).await?;
        let mut worker = drm::device_worker(exclusive)?;
        if let Some(nodes) = before.display {
            bounded(
                5,
                request(&mut worker, 1, Operation::CheckMaster(nodes.card)),
            )
            .await?;
        } else if before.drm_bound {
            return Err(io::Error::other(
                "bound DRM not ready; stop other hardware owners",
            ));
        }
        let result = or(
            bounded(5, async {
                request(
                    &mut worker,
                    1,
                    Operation::Switch(SwitchPlan::new(selected.clone(), Mode::Display)),
                )
                .await?;
                let ready = snapshot(&roots, &selected, &monitor, |s| {
                    s.config == Some(2) && s.display.is_some()
                })
                .await?;
                let nodes = ready.display.expect("ready display");
                let Reply::Opened(exports) = request(
                    &mut worker,
                    1,
                    Operation::Open {
                        card: nodes.card,
                        dmi_model: None,
                    },
                )
                .await?
                else {
                    return Err(io::Error::other("expected display exports"));
                };
                gpu_child(child_name, exports).await
            }),
            async {
                let _ = signals.next().await;
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "hardware import interrupted",
                ))
            },
        )
        .await;
        let native = bounded(3, async {
            request(
                &mut worker,
                2,
                Operation::Switch(SwitchPlan::new(selected.clone(), Mode::Native)),
            )
            .await?;
            snapshot(&roots, &selected, &monitor, Snapshot::native_safe).await?;
            Ok(())
        })
        .await;
        if let Err(error) = native {
            return Err(io::Error::other(format!(
                "native restoration failed: {error}; import result: {result:?}"
            )));
        }
        bounded(
            10,
            snapshot(&roots, &selected, &monitor, |s| s.native.is_some()),
        )
        .await?;
        result
    })
}
