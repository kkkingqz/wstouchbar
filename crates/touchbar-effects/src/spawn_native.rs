//! Narrow Linux boundary: owned helper status pipe, process groups and signal setup.
#![allow(unsafe_code)]
use std::{
    fs::File,
    io,
    os::fd::{AsRawFd, FromRawFd},
    os::unix::process::CommandExt,
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
};
static TERM: AtomicBool = AtomicBool::new(false);
extern "C" fn parent_dead(_: libc::c_int) {
    // SAFETY: kill is async-signal-safe; setup placed this helper in its own group before installing the handler.
    unsafe {
        libc::kill(0, libc::SIGKILL);
    }
}
extern "C" fn terminate(_: libc::c_int) {
    TERM.store(true, Ordering::Relaxed);
}
pub(crate) fn terminating() -> bool {
    TERM.load(Ordering::Relaxed)
}
pub(crate) fn helper_setup(parent: u32) -> io::Result<File> {
    // SAFETY: called once by the single-threaded helper, before spawning or running user code.
    unsafe {
        if libc::setpgid(0, 0) < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut action: libc::sigaction = std::mem::zeroed();
        libc::sigemptyset(&mut action.sa_mask);
        action.sa_sigaction = parent_dead as *const () as usize;
        if libc::sigaction(libc::SIGUSR1, &action, std::ptr::null_mut()) < 0 {
            return Err(io::Error::last_os_error());
        }
        action.sa_sigaction = terminate as *const () as usize;
        if libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut()) < 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) < 0
            || libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGUSR1, 0, 0, 0) < 0
        {
            return Err(io::Error::last_os_error());
        }
        if libc::getppid() != i32::try_from(parent).unwrap_or(-1) {
            parent_dead(0);
        }
        // FD 3 was installed by configure(). It belongs only to the helper, never the target executable.
        if libc::fcntl(3, libc::F_SETFD, libc::FD_CLOEXEC) < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(File::from_raw_fd(3))
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
