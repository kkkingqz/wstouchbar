use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Write},
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
}
#[derive(Default)]
struct PoolState {
    next: u64,
    jobs: BTreeMap<u64, BTreeSet<InstanceId>>,
}
struct Lease {
    counts: Arc<Mutex<PoolState>>,
    id: u64,
}
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
impl Lease {
    fn release(&self) {
        lock(&self.counts).jobs.remove(&self.id);
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
        }
    }

    pub fn cancel(&self) {
        let mut c = lock(&self.control);
        if !c.done && c.cancel.is_none() {
            c.cancel = Some(EffectError::Cancelled);
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
        }
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
                id,
            })
        };
        let mut command = Command::new(&self.helper);
        command
            .arg(std::process::id().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let status = native::configure(&mut command).map_err(failed)?;
        let mut child = command.spawn().map_err(failed)?;
        let pid = child.id();
        let setup = (|| {
            native::nonblocking(&status)?;
            native::nonblocking(child.stdin.as_ref().expect("stdin"))?;
            native::nonblocking(child.stdout.as_ref().expect("stdout"))?;
            native::nonblocking(child.stderr.as_ref().expect("stderr"))?;
            Ok::<_, std::io::Error>(())
        })();
        if let Err(e) = setup {
            native::signal_group(pid, libc::SIGKILL);
            let _ = child.wait();
            return Err(failed(e));
        }
        let control = Arc::new(Mutex::new(Control {
            pid,
            done: false,
            reaped: false,
            cancel: None,
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
    let mut status = native::helper_setup(parent).map_err(|e| e.to_string())?;
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
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        if let Some(input) = spec.stdin {
            let mut pipe = child.stdin.take().expect("stdin");
            std::thread::spawn(move || {
                let _ = pipe.write_all(input.as_bytes());
            });
        }
        let mut terminating = None;
        loop {
            if native::terminating() && terminating.is_none() {
                terminating = Some(Instant::now());
            }
            if terminating.is_some_and(|t| t.elapsed() >= Duration::from_secs(1)) {
                native::signal_group(std::process::id(), libc::SIGKILL);
            }
            if let Some(exit) = child.try_wait().map_err(|e| e.to_string())? {
                use std::os::unix::process::ExitStatusExt;
                break Ok(exit.code().unwrap_or_else(|| -exit.signal().unwrap_or(1)));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    })();
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
    // If the target forked children, supervise and reap them instead of letting them escape the session.
    if native::reap_descendants().map_err(|e| e.to_string())? {
        native::signal_group(std::process::id(), libc::SIGTERM);
        let end = Instant::now() + Duration::from_secs(1);
        while Instant::now() < end {
            if !native::reap_descendants().map_err(|e| e.to_string())? {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        native::signal_group(std::process::id(), libc::SIGKILL);
    }
    Ok(())
}
struct Reader {
    file: Box<dyn Read + Send>,
    bytes: Vec<u8>,
    eof: bool,
    truncated: bool,
}
impl Reader {
    fn new(file: impl Read + Send + 'static) -> Self {
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
    mut event: ProcessEvent,
    control: &Mutex<Control>,
) -> bool {
    loop {
        match events.try_send(event) {
            Ok(()) => return true,
            Err(async_channel::TrySendError::Closed(_)) => return false,
            Err(async_channel::TrySendError::Full(e)) => {
                event = e;
                if lock(control).cancel.is_some() {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(5));
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
) {
    let start = Instant::now();
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
            if !c.reaped && termination.is_some_and(|t| t.elapsed() >= Duration::from_secs(1)) {
                native::signal_group(c.pid, libc::SIGKILL);
            }
            if !c.reaped {
                match native::exited_without_reaping(c.pid) {
                    Ok(true) => {
                        // The zombie reserves this group id until cleanup has been signalled.
                        native::signal_group(c.pid, libc::SIGKILL);
                        match child.wait() {
                            Ok(_) => {
                                exited = true;
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
                    if overflow || !send(&events, ProcessEvent::Line(value), &control) {
                        if lock(&control).cancel.is_none() {
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
        std::thread::sleep(Duration::from_millis(5));
    }
    if lines && !line.is_empty() && lock(&control).cancel.is_none() {
        let (value, _) = text(line, limit);
        send(&events, ProcessEvent::Line(value), &control);
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
        let control = Arc::new(Mutex::new(Control {
            pid,
            done: false,
            reaped: false,
            cancel: None,
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
}
