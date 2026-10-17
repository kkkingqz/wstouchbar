//! Narrow Linux boundary: owned helper status pipe, process groups and signal setup.
#![allow(unsafe_code)]
use std::{
    fs::File,
    io::{self, Read},
    os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd},
    os::unix::process::CommandExt,
    process::Command,
    sync::atomic::{AtomicBool, AtomicI32, Ordering},
    time::Instant,
};
static TERM: AtomicBool = AtomicBool::new(false);
static PARENT_DEAD: AtomicBool = AtomicBool::new(false);
static SIGNAL_WRITE: AtomicI32 = AtomicI32::new(-1);
extern "C" fn signal_wake(signal: libc::c_int) {
    if signal == libc::SIGTERM {
        TERM.store(true, Ordering::Relaxed);
    }
    if signal == libc::SIGUSR1 {
        PARENT_DEAD.store(true, Ordering::Relaxed);
    }
    // SAFETY: write is async-signal-safe. The nonblocking CLOEXEC pipe remains
    // owned by SignalWake for the helper's lifetime. A full pipe is already ready.
    unsafe {
        let errno = *libc::__errno_location();
        libc::write(
            SIGNAL_WRITE.load(Ordering::Relaxed),
            [1u8].as_ptr().cast(),
            1,
        );
        *libc::__errno_location() = errno;
    }
}
pub(crate) fn terminating() -> bool {
    TERM.load(Ordering::Relaxed)
}
pub(crate) fn parent_dead() -> bool {
    PARENT_DEAD.load(Ordering::Relaxed)
}
pub(crate) struct SignalWake {
    pub read: File,
    _write: File,
}
impl SignalWake {
    pub fn drain(&mut self) {
        let mut bytes = [0u8; 128];
        while self.read.read(&mut bytes).is_ok_and(|n| n > 0) {}
    }
    pub fn wait(&self, deadline: Option<Instant>) -> io::Result<()> {
        wait(&[(self.read.as_fd(), false)], deadline)
    }
}
pub(crate) fn helper_setup(parent: u32) -> io::Result<(File, SignalWake)> {
    let (read, write) = rustix::pipe::pipe_with(
        rustix::pipe::PipeFlags::CLOEXEC | rustix::pipe::PipeFlags::NONBLOCK,
    )?;
    SIGNAL_WRITE.store(write.as_raw_fd(), Ordering::Relaxed);
    let wake = SignalWake {
        read: File::from(read),
        _write: File::from(write),
    };
    // SAFETY: called once by the single-threaded helper before spawning user code.
    unsafe {
        if libc::setpgid(0, 0) < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut action: libc::sigaction = std::mem::zeroed();
        libc::sigemptyset(&mut action.sa_mask);
        action.sa_sigaction = signal_wake as *const () as usize;
        for signal in [libc::SIGCHLD, libc::SIGTERM, libc::SIGUSR1] {
            if libc::sigaction(signal, &action, std::ptr::null_mut()) < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        if libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) < 0
            || libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGUSR1, 0, 0, 0) < 0
        {
            return Err(io::Error::last_os_error());
        }
        if libc::getppid() != i32::try_from(parent).unwrap_or(-1) {
            signal_wake(libc::SIGUSR1);
        }
        if libc::fcntl(3, libc::F_SETFD, libc::FD_CLOEXEC) < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((File::from_raw_fd(3), wake))
    }
}
/// All direct children are unreaped and their PIDs cannot be reused while this
/// single wait owner signals them. Orphaned grandchildren are adopted here too.
pub(crate) fn signal_children(signal: i32) -> io::Result<()> {
    let children =
        std::fs::read_to_string(format!("/proc/self/task/{}/children", std::process::id()))?;
    for pid in children.split_whitespace() {
        let pid: i32 = pid.parse().map_err(io::Error::other)?;
        // SAFETY: PID is an unreaped direct child of this helper; no other thread waits.
        unsafe {
            libc::kill(pid, signal);
        }
    }
    Ok(())
}
pub(crate) fn pidfd(pid: u32) -> io::Result<File> {
    // SAFETY: pidfd_open returns a new owned CLOEXEC fd, or -1 and errno.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0u32) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd is a newly allocated, uniquely owned descriptor.
    Ok(unsafe { File::from_raw_fd(i32::try_from(fd).map_err(io::Error::other)?) })
}
pub(crate) fn wait(fds: &[(BorrowedFd<'_>, bool)], deadline: Option<Instant>) -> io::Result<()> {
    let mut fds: Vec<_> = fds
        .iter()
        .map(|(fd, write)| {
            rustix::event::PollFd::new(
                fd,
                if *write {
                    rustix::event::PollFlags::OUT
                } else {
                    rustix::event::PollFlags::IN
                },
            )
        })
        .collect();
    loop {
        let timeout = deadline
            .map(|at| {
                rustix::event::Timespec::try_from(at.saturating_duration_since(Instant::now()))
            })
            .transpose()
            .map_err(io::Error::other)?;
        match rustix::event::poll(&mut fds, timeout.as_ref()) {
            Ok(_) => return Ok(()),
            Err(rustix::io::Errno::INTR) => continue,
            Err(e) => return Err(e.into()),
        }
    }
}
pub(crate) fn configure(command: &mut Command) -> io::Result<File> {
    let (read, write) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)?;
    command.process_group(0);
    // SAFETY: closure performs only dup2 and returns errno; OwnedFd keeps write live through exec.
    unsafe {
        command.pre_exec(move || {
            let raw = write.as_raw_fd();
            if raw == 3 {
                if libc::fcntl(3, libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
            } else if libc::dup2(raw, 3) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(File::from(read))
}
pub(crate) fn nonblocking(fd: &impl std::os::fd::AsFd) -> io::Result<()> {
    let flags = rustix::fs::fcntl_getfl(fd)?;
    rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK)?;
    Ok(())
}
pub(crate) fn signal_group(pid: u32, signal: i32) {
    if let Ok(pid) = i32::try_from(pid) {
        // SAFETY: negative owned helper pid addresses only its private group. Caller prevents reuse by synchronizing reaping with cancellation.
        unsafe {
            libc::kill(-pid, signal);
        }
    }
}
pub(crate) fn kill_and_reap(pid: u32) {
    signal_group(pid, libc::SIGKILL);
    if let Ok(pid) = i32::try_from(pid) {
        // SAFETY: used only after worker creation fails, for its still-unreaped direct child.
        unsafe {
            libc::waitpid(pid, std::ptr::null_mut(), 0);
        }
    }
}
pub(crate) fn reap_descendants() -> io::Result<bool> {
    loop {
        // SAFETY: helper is a subreaper and this is its only wait loop after its command has been reaped.
        let result = unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) };
        if result > 0 {
            continue;
        }
        if result == 0 {
            return Ok(true);
        }
        let e = io::Error::last_os_error();
        if e.raw_os_error() == Some(libc::ECHILD) {
            return Ok(false);
        }
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

pub(crate) fn exited_without_reaping(pid: u32) -> io::Result<bool> {
    use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
    let pid = Pid::from_raw(i32::try_from(pid).map_err(|_| io::Error::other("pid out of range"))?)
        .ok_or_else(|| io::Error::other("zero pid"))?;
    Ok(waitid(
        WaitId::Pid(pid),
        WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
    )?
    .is_some())
}
