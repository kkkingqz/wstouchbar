use super::*;
use futures_lite::{
    future::{or, poll_fn},
    io::AsyncWriteExt,
};
use std::{
    fs::{self, File},
    future::Future,
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    pin::Pin,
    task::Poll,
    time::Duration,
};
type Stream = async_io::Async<UnixStream>;
type Client = Pin<Box<dyn Future<Output = io::Result<Option<Incoming>>>>>;
pub struct Incoming {
    pub request: Request,
    stream: Stream,
}
pub struct Server {
    listener: async_io::Async<UnixListener>,
    clients: Vec<Client>,
    accept_retry: Option<async_io::Timer>,
    issues: Vec<String>,
    path: PathBuf,
    identity: (u64, u64),
    _lock: File,
}
pub fn check_peer(stream: &UnixStream, uid: u32) -> io::Result<()> {
    if rustix::net::sockopt::socket_peercred(stream)?.uid.as_raw() != uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "control peer has another UID",
        ));
    }
    Ok(())
}
fn private_dir(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != rustix::process::getuid().as_raw()
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{} must be an owned directory with mode 0700",
                path.display()
            ),
        ));
    }
    Ok(())
}
fn ensure_dir(path: &Path) -> io::Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    private_dir(path)
}
fn timed(client: impl Future<Output = io::Result<Option<Incoming>>> + 'static) -> Client {
    Box::pin(or(client, async {
        async_io::Timer::after(Duration::from_secs(5)).await;
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "control client timed out",
        ))
    }))
}
impl Server {
    pub fn bind(runtime: &Path, simulator: bool) -> io::Result<Self> {
        if !runtime.is_absolute() {
            return Err(invalid("runtime directory must be absolute"));
        }
        private_dir(runtime)?;
        ensure_dir(&runtime.join("touchbar"))?;
        if simulator {
            ensure_dir(&runtime.join("touchbar/simulator"))?;
        }
        let path = socket_path(runtime, simulator);
        let lock = File::open(path.parent().expect("socket parent"))?;
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if !metadata.file_type().is_socket()
                    || metadata.uid() != rustix::process::getuid().as_raw()
                {
                    return Err(invalid(
                        "refusing to replace a non-socket or foreign control path",
                    ));
                }
                let fd = rustix::net::socket_with(
                    rustix::net::AddressFamily::UNIX,
                    rustix::net::SocketType::STREAM,
                    rustix::net::SocketFlags::NONBLOCK | rustix::net::SocketFlags::CLOEXEC,
                    None,
                )?;
                match rustix::net::connect(&fd, &rustix::net::SocketAddrUnix::new(&path)?) {
                    Err(rustix::io::Errno::CONNREFUSED) => fs::remove_file(&path)?,
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::AddrInUse,
                            "session control socket already active",
                        ));
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        let metadata = fs::symlink_metadata(&path)?;
        Ok(Self {
            listener: async_io::Async::new(listener)?,
            clients: Vec::new(),
            accept_retry: None,
            issues: Vec::new(),
            path,
            identity: (metadata.dev(), metadata.ino()),
            _lock: lock,
        })
    }
    pub fn take_issues(&mut self) -> Vec<String> {
        std::mem::take(&mut self.issues)
    }
    fn retry_accept(&mut self, error: &io::Error) {
        if self.issues.len() < 8 {
            self.issues.push(format!("control accept: {error}"));
        }
        self.accept_retry = Some(async_io::Timer::after(Duration::from_millis(100)));
    }
    pub async fn next(&mut self) -> io::Result<Incoming> {
        enum Ready {
            Client(io::Result<Option<Incoming>>),
            Accept(io::Result<UnixStream>),
        }
        loop {
            let ready = poll_fn(|cx| {
                for i in 0..self.clients.len() {
                    if let Poll::Ready(result) = self.clients[i].as_mut().poll(cx) {
                        drop(self.clients.swap_remove(i));
                        return Poll::Ready(Ready::Client(result));
                    }
                }
                if let Some(timer) = &mut self.accept_retry {
                    if Pin::new(timer).poll(cx).is_pending() {
                        return Poll::Pending;
                    }
                    self.accept_retry = None;
                }
                if self.clients.len() < 8 {
                    loop {
                        match self.listener.get_ref().accept() {
                            Ok((stream, _)) => return Poll::Ready(Ready::Accept(Ok(stream))),
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                // poll_readable keeps the registration in Async itself.
                                // A temporary read_with future would unregister on drop.
                                match self.listener.poll_readable(cx) {
                                    Poll::Ready(Ok(())) => continue,
                                    Poll::Ready(Err(error)) => {
                                        return Poll::Ready(Ready::Accept(Err(error)));
                                    }
                                    Poll::Pending => break,
                                }
                            }
                            Err(error) => return Poll::Ready(Ready::Accept(Err(error))),
                        }
                    }
                }
                Poll::Pending
            })
            .await;
            match ready {
                Ready::Accept(Ok(stream)) => {
                    if check_peer(&stream, rustix::process::getuid().as_raw()).is_err() {
                        continue;
                    }
                    let mut stream = match Stream::new(stream) {
                        Ok(stream) => stream,
                        Err(error) => {
                            self.retry_accept(&error);
                            continue;
                        }
                    };
                    self.clients.push(timed(async move {
                        let request: Request = match read_async(&mut stream).await {
                            Ok(r) => r,
                            Err(e) => {
                                let response = Response {
                                    version: VERSION,
                                    reply: Reply::Error(e.to_string()),
                                };
                                stream.write_all(&encode(&response)?).await?;
                                return Ok(None);
                            }
                        };
                        if let Err(e) = request.validate() {
                            stream
                                .write_all(&encode(&Response {
                                    version: VERSION,
                                    reply: Reply::Error(e.to_string()),
                                })?)
                                .await?;
                            return Ok(None);
                        }
                        Ok(Some(Incoming { request, stream }))
                    }));
                }
                Ready::Accept(Err(e)) if transient_accept(&e) => self.retry_accept(&e),
                Ready::Accept(Err(e)) => return Err(e),
                Ready::Client(Ok(Some(request))) => return Ok(request),
                Ready::Client(Ok(None) | Err(_)) => {} // Malformed/slow clients are isolated.
            }
        }
    }
    pub fn respond(&mut self, mut incoming: Incoming, reply: Reply) -> io::Result<()> {
        let bytes = encode(&Response {
            version: VERSION,
            reply,
        })
        .or_else(|error| {
            encode(&Response {
                version: VERSION,
                reply: Reply::Error(error.to_string()),
            })
        })?;
        self.clients.push(timed(async move {
            incoming.stream.write_all(&bytes).await?;
            Ok(None)
        }));
        Ok(())
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path).is_ok_and(|m| (m.dev(), m.ino()) == self.identity) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn transient_accept(error: &io::Error) -> bool {
    use rustix::io::Errno;
    error.kind() == io::ErrorKind::Interrupted
        || error.kind() == io::ErrorKind::ConnectionAborted
        || error.raw_os_error().is_some_and(|code| {
            [
                Errno::MFILE,
                Errno::NFILE,
                Errno::NOBUFS,
                Errno::NOMEM,
                Errno::PROTO,
            ]
            .iter()
            .any(|errno| errno.raw_os_error() == code)
        })
}
