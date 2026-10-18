use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Write},
    os::{fd::AsFd, unix::net::UnixStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};
use touchbar_model::*;
#[path = "spawn_native.rs"]
mod native;
const MAX_SPEC: usize = 1024 * 1024;
#[derive(Clone, Debug)]
pub enum ProcessEvent {
    Line(String),
    Exited(Result<SpawnResult, EffectError>),
}
#[derive(Clone)]
pub struct SpawnPool {
    helper: PathBuf,
    per_instance: usize,
    session: usize,
    counts: Arc<Mutex<PoolState>>,
    changed: Arc<event_listener::Event>,
}
#[derive(Default)]
struct PoolState {
    next: u64,
    jobs: BTreeMap<u64, BTreeSet<InstanceId>>,
}
struct Lease {
    counts: Arc<Mutex<PoolState>>,
    changed: Arc<event_listener::Event>,
    id: u64,
}
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
impl Lease {
    fn release(&self) {
        if lock(&self.counts).jobs.remove(&self.id).is_some() {
            self.changed.notify(usize::MAX);
        }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.release();
    }
}
struct Control {
    pid: u32,
    done: bool,
    reaped: bool,
    cancel: Option<EffectError>,
    wake: UnixStream,
}
pub struct Process {
    pub events: async_channel::Receiver<ProcessEvent>,
    control: Arc<Mutex<Control>>,
    lease: Arc<Lease>,
}
impl Process {
    /// Existing shared sources transfer their quota attribution without starting another process.
    pub fn set_owners(&self, owners: BTreeSet<InstanceId>) {
        if !owners.is_empty()
            && let Some(job) = lock(&self.lease.counts).jobs.get_mut(&self.lease.id)
        {
            *job = owners;
            self.lease.changed.notify(usize::MAX);
        }
    }

    pub fn cancel(&self) {
        let mut c = lock(&self.control);
        if !c.done && c.cancel.is_none() {
            c.cancel = Some(EffectError::Cancelled);
            let _ = c.wake.write(&[1]);
            if !c.reaped {
                native::signal_group(c.pid, libc::SIGTERM);
            }
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl SpawnPool {
    pub fn new(helper: PathBuf, per_instance: usize, session: usize) -> Self {
        Self {
            helper,
            per_instance,
            session,
            counts: Arc::new(Mutex::new(PoolState::default())),
            changed: Arc::new(event_listener::Event::new()),
        }
    }
    pub(crate) fn notify_change(&self) {
        self.changed.notify(usize::MAX);
    }
    pub(crate) fn changed(&self) -> event_listener::EventListener {
        self.changed.listen()
    }
    pub fn active(&self) -> usize {
        lock(&self.counts).jobs.len()
    }
    pub fn start(
        &self,
        owner: InstanceId,
        spec: SpawnSpec,
        lines: bool,
        timeout_ms: Option<u64>,
    ) -> Result<Process, EffectError> {
        self.start_shared(&[owner].into(), spec, lines, timeout_ms)
    }
    pub fn start_shared(
        &self,
        owners: &BTreeSet<InstanceId>,
        spec: SpawnSpec,
        lines: bool,
        timeout_ms: Option<u64>,
    ) -> Result<Process, EffectError> {
        if owners.is_empty() {
            return Err(failed("spawn requires an owner"));
        }
        spec.validate()
            .map_err(|e| EffectError::Failed(e.to_string()))?;
        if spec.max_output > 1024 * 1024 || timeout_ms == Some(0) {
            return Err(EffectError::Failed("invalid spawn limits".into()));
        }
        let payload = serde_json::to_vec(&spec).map_err(|e| EffectError::Failed(e.to_string()))?;
        if payload.len() > MAX_SPEC {
            return Err(EffectError::Failed(
                "spawn specification exceeds 1 MiB".into(),
            ));
        }
        let lease = {
            let mut state = lock(&self.counts);
            if state.jobs.len() >= self.session
                || owners.iter().any(|owner| {
                    state
                        .jobs
                        .values()
                        .filter(|set| set.contains(owner))
                        .count()
                        >= self.per_instance
                })
            {
                return Err(EffectError::Unavailable);
            }
            state.next = state
                .next
                .checked_add(1)
                .ok_or_else(|| failed("process id counter exhausted"))?;
            let id = state.next;
            state.jobs.insert(id, owners.clone());
            Arc::new(Lease {
                counts: self.counts.clone(),
                changed: self.changed.clone(),
                id,
            })
        };
        let mut command = Command::new(&self.helper);
        command
            .arg(std::process::id().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let (wake_read, wake_write) = UnixStream::pair().map_err(failed)?;
        native::nonblocking(&wake_write).map_err(failed)?;
        let wake_read = async_io::Async::new(wake_read).map_err(failed)?;
        let status = native::configure(&mut command).map_err(failed)?;
        let mut child = command.spawn().map_err(failed)?;
        let pid = child.id();
        let setup = (|| {
            let pidfd = async_io::Async::new(native::pidfd(pid)?)?;
            native::nonblocking(&status)?;
            native::nonblocking(child.stdin.as_ref().expect("stdin"))?;
            native::nonblocking(child.stdout.as_ref().expect("stdout"))?;
            native::nonblocking(child.stderr.as_ref().expect("stderr"))?;
            Ok::<_, std::io::Error>(pidfd)
        })();
        let pidfd = match setup {
            Ok(fd) => fd,
            Err(e) => {
                native::signal_group(pid, libc::SIGKILL);
                let _ = child.wait();
                return Err(failed(e));
            }
        };
        let control = Arc::new(Mutex::new(Control {
            pid,
            done: false,
            reaped: false,
            cancel: None,
            wake: wake_write,
        }));
        let worker_control = control.clone();
        let (events, receiver) = async_channel::bounded(64);
        let worker_lease = lease.clone();
        let worker = std::thread::Builder::new()
            .name("touchbar-spawn".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                run(
                    child,
                    status,
                    payload,
                    spec.max_output as usize,
                    lines,
                    timeout_ms,
                    worker_control,
                    events,
                    worker_lease,
                    wake_read,
                    pidfd,
                )
            });
        if let Err(e) = worker {
            native::kill_and_reap(pid);
            return Err(failed(e));
        }
        Ok(Process {
            events: receiver,
            control,
            lease,
        })
    }
}
fn failed(e: impl std::fmt::Display) -> EffectError {
    EffectError::Failed(e.to_string())
}
#[derive(serde::Serialize, serde::Deserialize)]
struct HelperResult {
    status: Option<i32>,
    error: Option<String>,
}
/// Internal entry point for the sibling supervisor binary.
#[doc(hidden)]
pub fn helper_main() -> Result<(), String> {
    let parent = std::env::args()
        .nth(1)
        .ok_or("missing parent pid")?
        .parse::<u32>()
        .map_err(|e| e.to_string())?;
    let (mut status, mut wake) = native::helper_setup(parent).map_err(|e| e.to_string())?;
    let mut terminating = None;
    let outcome = (|| {
        let mut payload = vec![];
        std::io::stdin()
            .take(MAX_SPEC as u64 + 1)
            .read_to_end(&mut payload)
            .map_err(|e| e.to_string())?;
        if payload.len() > MAX_SPEC {
            return Err("spawn specification too large".into());
        }
        let spec: SpawnSpec = serde_json::from_slice(&payload).map_err(|e| e.to_string())?;
        spec.validate().map_err(|e| e.to_string())?;
        let mut command = Command::new(&spec.argv[0]);
        command
            .args(&spec.argv[1..])
            .envs(&spec.env)
            .stdin(if spec.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        if native::parent_dead() || native::terminating() {
            return Err("supervisor cancelled before launch".into());
        }
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        if let Some(input) = spec.stdin {
            let mut pipe = child.stdin.take().expect("stdin");
            std::thread::spawn(move || {
                let _ = pipe.write_all(input.as_bytes());
            });
        }
        loop {
            wake.drain();
            if native::parent_dead() || native::terminating() {
                let at = *terminating.get_or_insert_with(Instant::now);
                native::signal_children(
                    if native::parent_dead() || at.elapsed() >= Duration::from_secs(1) {
                        libc::SIGKILL
                    } else {
                        libc::SIGTERM
                    },
                )
                .map_err(|e| e.to_string())?;
            }
            if let Some(exit) = child.try_wait().map_err(|e| e.to_string())? {
                use std::os::unix::process::ExitStatusExt;
                break Ok(exit.code().unwrap_or_else(|| -exit.signal().unwrap_or(1)));
            }
            let deadline = terminating
                .map(|at| at + Duration::from_secs(1))
                .filter(|at| *at > Instant::now());
            wake.wait(deadline).map_err(|e| e.to_string())?;
        }
    })();
    // Remain alive as subreaper until all adopted descendants are collected.
    // A process that calls setsid is still our child after its parent exits.
    let end = terminating.unwrap_or_else(Instant::now) + Duration::from_secs(1);
    loop {
        wake.drain();
        if !native::reap_descendants().map_err(|e| e.to_string())? {
            break;
        }
        let kill = native::parent_dead() || Instant::now() >= end;
        native::signal_children(if kill { libc::SIGKILL } else { libc::SIGTERM })
            .map_err(|e| e.to_string())?;
        wake.wait((!kill).then_some(end))
            .map_err(|e| e.to_string())?;
    }
    let result = match outcome {
        Ok(code) => HelperResult {
            status: Some(code),
            error: None,
        },
        Err(error) => HelperResult {
            status: None,
            error: Some(error),
        },
    };
    serde_json::to_writer(&mut status, &result).map_err(|e| e.to_string())?;
    drop(status);
    Ok(())
}
trait Pipe: Read + AsFd + Send {}
impl<T: Read + AsFd + Send> Pipe for T {}
struct Reader {
    file: Box<dyn Pipe>,
    bytes: Vec<u8>,
    eof: bool,
    truncated: bool,
}
impl Reader {
    fn new(file: impl Pipe + 'static) -> Self {
        Self {
            file: Box::new(file),
            bytes: vec![],
            eof: false,
            truncated: false,
        }
    }
    fn read(&mut self, limit: usize) -> std::io::Result<Vec<u8>> {
        let mut fresh = vec![];
        let mut buffer = [0; 8192];
        for _ in 0..16 {
            match self.file.read(&mut buffer) {
                Ok(0) => {
                    self.eof = true;
                    break;
                }
                Ok(n) => {
                    let keep = n.min(limit.saturating_sub(self.bytes.len()));
                    self.bytes.extend_from_slice(&buffer[..keep]);
                    self.truncated |= keep < n;
                    fresh.extend_from_slice(&buffer[..n]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(fresh)
    }
}
fn send(
    events: &async_channel::Sender<ProcessEvent>,
    event: ProcessEvent,
    control: &Mutex<Control>,
    wake: &async_io::Async<UnixStream>,
    deadline: Option<Instant>,
    pidfd: &async_io::Async<File>,
    drain_deadline: &mut Option<Instant>,
) -> bool {
    enum Ready {
        Sent(bool),
        Cancel,
        Deadline,
        HelperExited,
    }
    let command_deadline = deadline;
    let timed_out = || {
        let mut c = lock(control);
        if c.cancel.is_none() {
            c.cancel = Some(EffectError::Timeout);
            if !c.reaped {
                native::signal_group(c.pid, libc::SIGTERM);
            }
        }
    };
    let mut delivery = std::pin::pin!(events.send(event));
    loop {
        if lock(control).cancel.is_some() {
            return false;
        }
        if command_deadline.is_some_and(|at| Instant::now() >= at) {
            timed_out();
            return false;
        }
        if drain_deadline.is_some_and(|at| Instant::now() >= at) {
            return false;
        }
        let deadline = command_deadline.into_iter().chain(*drain_deadline).min();
        let waiting_for_helper = drain_deadline.is_none();
        let ready = futures_lite::future::block_on(futures_lite::future::or(
            async { Ready::Sent(delivery.as_mut().await.is_ok()) },
            futures_lite::future::or(
                async {
                    let _ = wake.readable().await;
                    Ready::Cancel
                },
                futures_lite::future::or(
                    async {
                        if let Some(at) = deadline {
                            async_io::Timer::at(at).await;
                        } else {
                            std::future::pending::<()>().await;
                        }
                        Ready::Deadline
                    },
                    async {
                        if waiting_for_helper {
                            let _ = pidfd.readable().await;
                        } else {
                            std::future::pending::<()>().await;
                        }
                        Ready::HelperExited
                    },
                ),
            ),
        ));
        match ready {
            Ready::Sent(sent) => return sent,
            Ready::Cancel => return false,
            Ready::Deadline => {
                if command_deadline.is_some_and(|at| Instant::now() >= at) {
                    timed_out();
                }
                return false;
            }
            Ready::HelperExited => {
                // Delivery backpressure must not postpone cleanup. The unreaped helper
                // still reserves its process-group id, just as in the outer wait loop.
                let c = lock(control);
                if !c.reaped {
                    native::signal_group(c.pid, libc::SIGKILL);
                }
                *drain_deadline = Some(Instant::now() + Duration::from_secs(1));
            }
        }
    }
}
fn text(bytes: Vec<u8>, limit: usize) -> (String, bool) {
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    let truncated = text.len() > limit;
    if truncated {
        let mut end = limit;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    (text, truncated)
}
#[allow(clippy::too_many_arguments)]
fn run(
    mut child: Child,
    status: File,
    payload: Vec<u8>,
    limit: usize,
    lines: bool,
    timeout: Option<u64>,
    control: Arc<Mutex<Control>>,
    events: async_channel::Sender<ProcessEvent>,
    lease: Arc<Lease>,
    wake: async_io::Async<UnixStream>,
    pidfd: async_io::Async<File>,
) {
    let start = Instant::now();
    let deadline = timeout.map(|ms| start + Duration::from_millis(ms));
    let mut drain_deadline = None;
    let mut killed = false;
    let mut input = child.stdin.take();
    let mut offset = 0;
    let mut stdout = Reader::new(child.stdout.take().expect("stdout"));
    let mut stderr = Reader::new(child.stderr.take().expect("stderr"));
    let mut status = Reader::new(status);
    let mut exited = false;
    let mut termination = None;
    let mut line = vec![];
    let mut failure = None;
    loop {
        let mut bytes = [0; 64];
        let mut wake_pipe = wake.get_ref();
        while wake_pipe.read(&mut bytes).is_ok_and(|n| n > 0) {}
        {
            let mut c = lock(&control);
            if !c.done
                && timeout.is_some_and(|ms| start.elapsed() >= Duration::from_millis(ms))
                && c.cancel.is_none()
            {
                c.cancel = Some(EffectError::Timeout);
                if !c.reaped {
                    native::signal_group(c.pid, libc::SIGTERM);
                }
            }
            if c.cancel.is_some() && termination.is_none() {
                termination = Some(Instant::now());
            }
            if !c.reaped
                && !killed
                && termination.is_some_and(|t| t.elapsed() >= Duration::from_secs(2))
            {
                native::signal_group(c.pid, libc::SIGKILL);
                killed = true;
            }
            if !c.reaped {
                match native::exited_without_reaping(c.pid) {
                    Ok(true) => {
                        // The zombie reserves this group id until cleanup has been signalled.
                        native::signal_group(c.pid, libc::SIGKILL);
                        match child.wait() {
                            Ok(_) => {
                                exited = true;
                                drain_deadline
                                    .get_or_insert_with(|| Instant::now() + Duration::from_secs(1));
                                c.reaped = true;
                            }
                            Err(e) => failure = Some(failed(e)),
                        }
                    }
                    Ok(false) => {}
                    Err(e) => {
                        failure = Some(failed(e));
                        c.cancel.get_or_insert(EffectError::Cancelled);
                        native::signal_group(c.pid, libc::SIGTERM);
                    }
                }
            }
        }
        if let Some(pipe) = &mut input {
            match pipe.write(&payload[offset..]) {
                Ok(n) => {
                    offset += n;
                    if offset == payload.len() {
                        input = None;
                    }
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => {
                    input = None;
                }
            }
        }
        let fresh = match stdout.read(limit) {
            Ok(b) => b,
            Err(e) => {
                failure = Some(failed(e));
                vec![]
            }
        };
        if let Err(e) = stderr.read(limit) {
            failure = Some(failed(e));
        }
        if let Err(e) = status.read(8192) {
            failure = Some(failed(e));
        }
        if lines && lock(&control).cancel.is_none() {
            for byte in fresh {
                if byte == b'\n' {
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                    let (value, overflow) = text(std::mem::take(&mut line), limit);
                    if overflow
                        || !send(
                            &events,
                            ProcessEvent::Line(value),
                            &control,
                            &wake,
                            deadline,
                            &pidfd,
                            &mut drain_deadline,
                        )
                    {
                        if lock(&control).cancel.is_none()
                            && !deadline.is_some_and(|at| Instant::now() >= at)
                        {
                            failure = Some(failed("line delivery failed"));
                        }
                        break;
                    }
                } else {
                    line.push(byte);
                    if line.len() > limit {
                        failure = Some(failed("stdout line exceeds max-output"));
                        break;
                    }
                }
            }
        }
        if failure.is_some() {
            let mut c = lock(&control);
            if !c.done && c.cancel.is_none() {
                c.cancel = Some(EffectError::Cancelled);
                if !c.reaped {
                    native::signal_group(c.pid, libc::SIGTERM);
                }
            }
        }
        if exited && (lock(&control).cancel.is_some() || (stdout.eof && stderr.eof && status.eof)) {
            break;
        }
        if drain_deadline.is_some_and(|at| Instant::now() >= at) {
            failure
                .get_or_insert_with(|| failed("supervisor exited with inherited pipes still open"));
            // send() may have observed helper death while the output channel was full.
            // Reap that helper at the top before completing or releasing its quota.
            if !exited {
                continue;
            }
            break;
        }
        let next = deadline
            .filter(|_| lock(&control).cancel.is_none())
            .into_iter()
            .chain(
                termination
                    .filter(|_| !killed && !exited)
                    .map(|at| at + Duration::from_secs(2)),
            )
            .chain(drain_deadline)
            .min();
        let mut fds = vec![(wake.as_fd(), false)];
        if !exited {
            fds.push((pidfd.as_fd(), false));
        }
        if !stdout.eof {
            fds.push((stdout.file.as_fd(), false));
        }
        if !stderr.eof {
            fds.push((stderr.file.as_fd(), false));
        }
        if !status.eof {
            fds.push((status.file.as_fd(), false));
        }
        if let Some(input) = &input {
            fds.push((input.as_fd(), true));
        }
        if let Err(e) = native::wait(&fds, next) {
            failure = Some(failed(e));
        }
    }
    if lines && !line.is_empty() && lock(&control).cancel.is_none() {
        let (value, _) = text(line, limit);
        if !send(
            &events,
            ProcessEvent::Line(value),
            &control,
            &wake,
            deadline,
            &pidfd,
            &mut drain_deadline,
        ) && lock(&control).cancel.is_none()
        {
            failure.get_or_insert_with(|| failed("final line delivery failed"));
        }
    }
    let cancel = lock(&control).cancel.clone();
    let result = if let Some(error) = failure {
        Err(error)
    } else if let Some(error) = cancel {
        Err(error)
    } else {
        match serde_json::from_slice::<HelperResult>(&status.bytes) {
            Ok(HelperResult {
                status: Some(code),
                error: None,
            }) => {
                let (out, ot) = text(stdout.bytes, limit);
                let (err, et) = text(stderr.bytes, limit);
                Ok(SpawnResult {
                    status: code,
                    stdout: out,
                    stderr: err,
                    truncated: stdout.truncated || stderr.truncated || ot || et,
                })
            }
            Ok(HelperResult { error: Some(e), .. }) => Err(failed(e)),
            _ => Err(failed("supervisor exited without a valid result")),
        }
    };
    lock(&control).done = true;
    lease.release();
    drop(lease);
    let _ = events.send_blocking(ProcessEvent::Exited(result));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_stops_buffered_lines_after_helper_was_reaped() {
        let target = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let mut command = Command::new(target.join("touchbar-spawn-helper"));
        command
            .arg(std::process::id().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let status = native::configure(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        let pid = child.id();
        let spec = SpawnSpec::new(vec![
            target
                .join("touchbar-effects-test-helper")
                .to_str()
                .unwrap()
                .into(),
            "many-lines".into(),
        ]);
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&spec).unwrap())
            .unwrap();
        drop(command);
        // Complete the producer first, so delivery starts with an already reaped helper.
        let deadline = Instant::now() + Duration::from_secs(3);
        while !native::exited_without_reaping(pid).unwrap() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        native::nonblocking(&status).unwrap();
        native::nonblocking(child.stdout.as_ref().unwrap()).unwrap();
        native::nonblocking(child.stderr.as_ref().unwrap()).unwrap();
        let (wake_read, wake_write) = UnixStream::pair().unwrap();
        native::nonblocking(&wake_write).unwrap();
        let wake_read = async_io::Async::new(wake_read).unwrap();
        let pidfd = async_io::Async::new(native::pidfd(pid).unwrap()).unwrap();
        let control = Arc::new(Mutex::new(Control {
            pid,
            done: false,
            reaped: false,
            cancel: None,
            wake: wake_write,
        }));
        let (events, rx) = async_channel::bounded(64);
        let owner = InstanceId {
            number: 1,
            generation: Generation(1),
        };
        let pool = Arc::new(Mutex::new(PoolState {
            next: 1,
            jobs: BTreeMap::from([(1, [owner].into())]),
        }));
        let lease = Arc::new(Lease {
            counts: pool,
            changed: Arc::new(event_listener::Event::new()),
            id: 1,
        });
        let worker_lease = lease.clone();
        let cloned = control.clone();
        let worker = std::thread::spawn(move || {
            run(
                child,
                status,
                vec![],
                65536,
                true,
                None,
                cloned,
                events,
                worker_lease,
                wake_read,
                pidfd,
            )
        });
        let process = Process {
            events: rx,
            control,
            lease,
        };
        while process.events.len() < 64 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        process.cancel();
        loop {
            if let ProcessEvent::Exited(result) = process.events.recv_blocking().unwrap() {
                assert_eq!(result, Err(EffectError::Cancelled));
                break;
            }
        }
        worker.join().unwrap();
    }
    #[test]
    fn full_line_delivery_kills_owned_group_when_helper_exits() {
        use std::os::unix::process::CommandExt;
        let path =
            std::env::temp_dir().join(format!("wstouchbar-dead-helper-{}", std::process::id()));
        let mut child = Command::new("sh")
            .args(["-c", "sleep 60 & echo $! > \"$1\"; wait", "fixture"])
            .arg(&path)
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = child.id();
        let deadline = Instant::now() + Duration::from_secs(3);
        let descendant = loop {
            if let Ok(text) = std::fs::read_to_string(&path)
                && let Ok(pid) = text.trim().parse::<u32>()
            {
                break pid;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        };
        rustix::process::kill_process(
            rustix::process::Pid::from_raw(i32::try_from(pid).unwrap()).unwrap(),
            rustix::process::Signal::KILL,
        )
        .unwrap();
        while !native::exited_without_reaping(pid).unwrap() {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        let (read, write) = UnixStream::pair().unwrap();
        let wake = async_io::Async::new(read).unwrap();
        let pidfd = async_io::Async::new(native::pidfd(pid).unwrap()).unwrap();
        let control = Mutex::new(Control {
            pid,
            done: false,
            reaped: false,
            cancel: None,
            wake: write,
        });
        let (events, _receiver) = async_channel::bounded(1);
        events
            .send_blocking(ProcessEvent::Line("occupied".into()))
            .unwrap();
        let delivered = send(
            &events,
            ProcessEvent::Line("blocked".into()),
            &control,
            &wake,
            None,
            &pidfd,
            &mut None,
        );
        let running = std::fs::read_to_string(format!("/proc/{descendant}/stat")).is_ok_and(|s| {
            s.rsplit_once(") ")
                .is_some_and(|(_, rest)| !rest.starts_with('Z'))
        });
        native::signal_group(pid, libc::SIGKILL);
        child.wait().unwrap();
        let _ = std::fs::remove_file(path);
        assert!(!delivered);
        assert!(
            !running,
            "full output channel postponed descendant cleanup until after completion"
        );
    }
}
