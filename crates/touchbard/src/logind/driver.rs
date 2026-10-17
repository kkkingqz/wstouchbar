use super::{Properties, SessionRef, invalid, session_fact, session_ref, user_ref};
use crate::{Event, Session};
use async_channel::{Receiver, Sender};
use async_io::{Async, Timer};
use futures_lite::{StreamExt, future};
use std::{
    cell::RefCell,
    future::Future,
    io,
    os::fd::OwnedFd,
    pin::Pin,
    rc::Rc,
    time::{Duration, Instant},
};
use zbus::{
    Connection, MatchRule, Message, MessageStream,
    message::{Sequence, Type},
    zvariant::OwnedValue,
};
const DEST: &str = "org.freedesktop.login1";
const ROOT: &str = "/org/freedesktop/login1";
const SEAT: &str = "/org/freedesktop/login1/seat/seat0";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const SEAT_IF: &str = "org.freedesktop.login1.Seat";
const SESSION_IF: &str = "org.freedesktop.login1.Session";
const USER_IF: &str = "org.freedesktop.login1.User";
const PROPS: &str = "org.freedesktop.DBus.Properties";
const RPC: Duration = Duration::from_millis(500);
#[derive(Clone, Debug)]
pub enum Notice {
    Fact(Event),
    Unavailable(String),
}
type Task<T> = Pin<Box<dyn Future<Output = T>>>;
/// Stored actor futures survive cancellation of `next` by the shell's select.
/// Coalesced inhibitor control cannot fill a queue and lose a required release.
pub struct Monitor {
    work: Task<()>,
    events: Receiver<Notice>,
    desired: Rc<RefCell<Option<u64>>>,
    wake: Sender<()>,
}
impl Default for Monitor {
    fn default() -> Self {
        Self::new()
    }
}
impl Monitor {
    pub fn new() -> Self {
        Self::build(None)
    }
    pub fn with_address(address: String) -> Self {
        Self::build(Some(address))
    }
    fn build(address: Option<String>) -> Self {
        let (send, events) = async_channel::bounded(64);
        let (wake, receive) = async_channel::bounded(1);
        let desired = Rc::new(RefCell::new(None));
        Self {
            work: Box::pin(run(address, send, receive, desired.clone())),
            events,
            desired,
            wake,
        }
    }
    pub fn acquire_inhibitor(&self, generation: u64) {
        *self.desired.borrow_mut() = Some(generation);
        let _ = self.wake.try_send(());
    }
    pub fn release_inhibitor(&self, generation: u64) {
        if *self.desired.borrow() == Some(generation) {
            *self.desired.borrow_mut() = None;
            let _ = self.wake.try_send(());
        }
    }
    pub async fn next(&mut self) -> io::Result<Notice> {
        future::or(
            async {
                self.events
                    .recv()
                    .await
                    .map_err(|_| io::Error::other("logind monitor stopped"))
            },
            async {
                self.work.as_mut().await;
                Err(io::Error::other("logind actor stopped"))
            },
        )
        .await
    }
}
async fn bounded<T>(work: impl Future<Output = io::Result<T>>) -> io::Result<T> {
    future::or(work, async {
        Timer::after(RPC).await;
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "logind request timed out",
        ))
    })
    .await
}
fn body(message: &Message) -> io::Result<()> {
    if message.body().len() > 256 * 1024 {
        Err(invalid("logind message too large"))
    } else {
        Ok(())
    }
}
async fn signal(
    conn: &Connection,
    owner: &str,
    path: &str,
    interface: &str,
    member: &str,
    arg: Option<&str>,
) -> io::Result<MessageStream> {
    let mut rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(owner)
        .map_err(invalid)?
        .path(path)
        .map_err(invalid)?
        .interface(interface)
        .map_err(invalid)?
        .member(member)
        .map_err(invalid)?;
    if let Some(arg) = arg {
        rule = rule.arg(0, arg).map_err(invalid)?;
    }
    MessageStream::for_match_rule(rule.build(), conn, Some(64))
        .await
        .map_err(invalid)
}
async fn get(
    conn: &Connection,
    owner: &str,
    path: &str,
    interface: &str,
    key: &str,
) -> io::Result<(OwnedValue, Sequence)> {
    let reply = conn
        .call_method(Some(owner), path, Some(PROPS), "Get", &(interface, key))
        .await
        .map_err(invalid)?;
    body(&reply)?;
    Ok((
        reply.body().deserialize().map_err(invalid)?,
        reply.recv_position(),
    ))
}
async fn all(
    conn: &Connection,
    owner: &str,
    path: &str,
    interface: &str,
) -> io::Result<(Properties, Sequence)> {
    let reply = conn
        .call_method(Some(owner), path, Some(PROPS), "GetAll", &(interface,))
        .await
        .map_err(invalid)?;
    body(&reply)?;
    Ok((
        reply.body().deserialize().map_err(invalid)?,
        reply.recv_position(),
    ))
}
struct Watch {
    stream: MessageStream,
    position: Option<Sequence>,
}
impl Watch {
    async fn next(&mut self) -> io::Result<Message> {
        loop {
            let m = self
                .stream
                .next()
                .await
                .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "logind disconnected"))?
                .map_err(invalid)?;
            body(&m)?;
            if self.position.is_none_or(|p| m.recv_position() > p) {
                return Ok(m);
            }
        }
    }
}
#[derive(Clone, Copy)]
enum Source {
    Owner,
    Sleep,
    Seat,
    Session,
    User,
}
fn relevant(source: Source, message: &Message) -> io::Result<bool> {
    let (_, changed, invalidated) = message
        .body()
        .deserialize::<(String, Properties, Vec<String>)>()
        .map_err(invalid)?;
    let keys: &[&str] = match source {
        Source::Seat => &["ActiveSession"],
        Source::Session => &[
            "User",
            "Id",
            "Active",
            "State",
            "Class",
            "Type",
            "LockedHint",
        ],
        Source::User => &["UID", "Display"],
        _ => &[],
    };
    Ok(keys
        .iter()
        .any(|key| changed.contains_key(*key) || invalidated.iter().any(|name| name == key)))
}
async fn snapshot_changed(watch: &mut Watch, source: Source) -> io::Result<bool> {
    for _ in 0..64 {
        let Some(message) = future::poll_once(watch.next()).await.transpose()? else {
            return Ok(false);
        };
        if relevant(source, &message)? {
            return Ok(true);
        }
    }
    Err(invalid("excessive logind property traffic during snapshot"))
}
struct Connected {
    conn: Connection,
    owner: String,
    owners: Watch,
    sleep: Watch,
    seat: Watch,
    session: Option<Watch>,
    user: Option<Watch>,
    active: Option<SessionRef>,
    fact: Option<Session>,
    preparing: bool,
    sleep_edges: Vec<bool>,
}
impl Connected {
    async fn connect(address: Option<String>) -> io::Result<Box<Self>> {
        let builder = if let Some(address) = address.as_deref() {
            zbus::connection::Builder::address(address).map_err(invalid)?
        } else {
            zbus::connection::Builder::system().map_err(invalid)?
        };
        let conn = builder
            .method_timeout(RPC)
            .max_queued(64)
            .build()
            .await
            .map_err(invalid)?;
        let owners = signal(
            &conn,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameOwnerChanged",
            Some(DEST),
        )
        .await?;
        let reply = conn
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "GetNameOwner",
                &(DEST,),
            )
            .await
            .map_err(invalid)?;
        let owner = reply.body().deserialize::<String>().map_err(invalid)?;
        let sleep = signal(&conn, &owner, ROOT, MANAGER, "PrepareForSleep", None).await?;
        let seat = signal(
            &conn,
            &owner,
            SEAT,
            PROPS,
            "PropertiesChanged",
            Some(SEAT_IF),
        )
        .await?;
        let (preparing, position) = get(&conn, &owner, ROOT, MANAGER, "PreparingForSleep").await?;
        let mut result = Self {
            conn,
            owner,
            owners: Watch {
                stream: owners,
                position: Some(reply.recv_position()),
            },
            sleep: Watch {
                stream: sleep,
                position: Some(position),
            },
            seat: Watch {
                stream: seat,
                position: None,
            },
            session: None,
            user: None,
            active: None,
            fact: None,
            preparing: bool::try_from(&preparing).map_err(invalid)?,
            sleep_edges: Vec::new(),
        };
        result.refresh().await?;
        Ok(Box::new(result))
    }
    async fn property_watch(&self, path: &str, interface: &str) -> io::Result<Watch> {
        Ok(Watch {
            stream: signal(
                &self.conn,
                &self.owner,
                path,
                PROPS,
                "PropertiesChanged",
                Some(interface),
            )
            .await?,
            position: None,
        })
    }
    async fn refresh(&mut self) -> io::Result<()> {
        // Subscribe before the GetAll snapshot. If a newer notification is already
        // queued, repeat within the caller's single total RPC deadline, never publish it stale.
        loop {
            future::yield_now().await;
            let (value, _) = get(&self.conn, &self.owner, SEAT, SEAT_IF, "ActiveSession").await?;
            let active = session_ref(&value)?;
            let (mut session, mut user, fact) =
                if active.id.is_empty() || active.path.as_str() == "/" {
                    (None, None, None)
                } else {
                    let mut session = self
                        .property_watch(active.path.as_str(), SESSION_IF)
                        .await?;
                    let (properties, position) =
                        all(&self.conn, &self.owner, active.path.as_str(), SESSION_IF).await?;
                    session.position = Some(position);
                    let (_, user_path) = user_ref(&properties)?;
                    let mut user = self.property_watch(user_path.as_str(), USER_IF).await?;
                    let (user_properties, position) =
                        all(&self.conn, &self.owner, user_path.as_str(), USER_IF).await?;
                    user.position = Some(position);
                    (
                        Some(session),
                        Some(user),
                        session_fact(&active, &properties, &user_properties)?,
                    )
                };
            let (value, seat_position) =
                get(&self.conn, &self.owner, SEAT, SEAT_IF, "ActiveSession").await?;
            if session_ref(&value)? != active {
                continue;
            }
            self.seat.position = Some(seat_position);
            let mut changed = false;
            if let Some(watch) = session.as_mut() {
                changed |= snapshot_changed(watch, Source::Session).await?;
            }
            if let Some(watch) = user.as_mut() {
                changed |= snapshot_changed(watch, Source::User).await?;
            }
            changed |= snapshot_changed(&mut self.seat, Source::Seat).await?;
            if let Some(message) = future::poll_once(self.owners.next()).await.transpose()?
                && self.owner_changed(&message)?
            {
                return Err(io::Error::other("logind owner changed during snapshot"));
            }
            // Sleep signals are never coalesced while running; first startup uses their
            // final known value before emitting Start, so it cannot inhibit during sleep.
            while let Some(message) = future::poll_once(self.sleep.next()).await.transpose()? {
                self.sleep_message(&message)?;
            }
            if changed {
                continue;
            }
            self.active = Some(active);
            self.fact = fact;
            self.session = session;
            self.user = user;
            return Ok(());
        }
    }
    fn owner_changed(&self, message: &Message) -> io::Result<bool> {
        let (_, _, new) = message
            .body()
            .deserialize::<(String, String, String)>()
            .map_err(invalid)?;
        Ok(new != self.owner)
    }
    fn sleep_message(&mut self, message: &Message) -> io::Result<()> {
        let value = message.body().deserialize::<bool>().map_err(invalid)?;
        self.preparing = value;
        if self.sleep_edges.len() >= 64 {
            return Err(invalid("excessive logind sleep traffic"));
        }
        self.sleep_edges.push(value);
        Ok(())
    }
    async fn next(&mut self) -> io::Result<(Source, Message)> {
        async fn optional(
            watch: &mut Option<Watch>,
            source: Source,
        ) -> io::Result<(Source, Message)> {
            match watch {
                Some(w) => w.next().await.map(|m| (source, m)),
                None => std::future::pending().await,
            }
        }
        future::or(
            async { self.owners.next().await.map(|m| (Source::Owner, m)) },
            future::or(
                async { self.sleep.next().await.map(|m| (Source::Sleep, m)) },
                future::or(
                    async { self.seat.next().await.map(|m| (Source::Seat, m)) },
                    future::or(
                        optional(&mut self.session, Source::Session),
                        optional(&mut self.user, Source::User),
                    ),
                ),
            ),
        )
        .await
    }
    /// Apply fail-closed security facts before awaiting a fresh snapshot. Locking
    /// preserves the owner; other invalidated identity/eligibility fields revoke it.
    fn invalidation(&mut self, source: Source, message: &Message) -> io::Result<Option<Event>> {
        let (_, changed, invalidated) = message
            .body()
            .deserialize::<(String, Properties, Vec<String>)>()
            .map_err(invalid)?;
        let has = |key: &str| changed.contains_key(key) || invalidated.iter().any(|s| s == key);
        match source {
            Source::Seat if has("ActiveSession") => Ok(Some(Event::Session(None))),
            Source::User if has("Display") || has("UID") => Ok(Some(Event::Session(None))),
            Source::Session
                if ["User", "Id", "Active", "State", "Class", "Type"]
                    .iter()
                    .any(|key| has(key)) =>
            {
                Ok(Some(Event::Session(None)))
            }
            Source::Session if has("LockedHint") => {
                let must_lock = changed
                    .get("LockedHint")
                    .and_then(|v| bool::try_from(v).ok())
                    .unwrap_or(true);
                if must_lock && let Some(fact) = &mut self.fact {
                    fact.locked = true;
                    return Ok(Some(Event::Session(Some(fact.clone()))));
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}
struct Lease {
    generation: u64,
    fd: Async<OwnedFd>,
}
async fn lost(lease: &Option<Lease>) -> io::Result<()> {
    let Some(lease) = lease else {
        return std::future::pending().await;
    };
    lease
        .fd
        .read_with(|fd| {
            let mut fds = [rustix::event::PollFd::new(
                fd,
                rustix::event::PollFlags::empty(),
            )];
            rustix::event::poll(
                &mut fds,
                Some(&rustix::event::Timespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                }),
            )?;
            if fds[0].revents().intersects(
                rustix::event::PollFlags::HUP
                    | rustix::event::PollFlags::ERR
                    | rustix::event::PollFlags::NVAL,
            ) {
                Ok(())
            } else {
                Err(io::ErrorKind::WouldBlock.into())
            }
        })
        .await
}
async fn inhibit(conn: Connection, owner: String) -> io::Result<OwnedFd> {
    let reply = conn
        .call_method(
            Some(owner.as_str()),
            ROOT,
            Some(MANAGER),
            "Inhibit",
            &(
                "sleep",
                "touchbard",
                "Return Touch Bar to native before sleep",
                "delay",
            ),
        )
        .await
        .map_err(invalid)?;
    body(&reply)?;
    reply
        .body()
        .deserialize::<zbus::zvariant::OwnedFd>()
        .map(Into::into)
        .map_err(invalid)
}
enum Pick {
    Wake,
    Query(io::Result<Box<Connected>>),
    Signal(io::Result<(Source, Message)>),
    Lost(io::Result<()>),
    Inhibited(u64, io::Result<OwnedFd>),
    Retry,
}
async fn run(
    address: Option<String>,
    send: Sender<Notice>,
    wake: Receiver<()>,
    desired: Rc<RefCell<Option<u64>>>,
) {
    let mut connected: Option<Box<Connected>> = None;
    let mut query: Option<Task<io::Result<Box<Connected>>>> = None;
    let mut reconnect = true;
    let mut started = false;
    let mut held: Option<Lease> = None;
    let mut acquiring: Option<(u64, Task<io::Result<OwnedFd>>)> = None;
    let mut retry = Some(Instant::now());
    let mut backoff = 100u64;
    loop {
        let wanted = *desired.borrow();
        if held.as_ref().is_some_and(|h| Some(h.generation) != wanted) {
            held = None;
        }
        if acquiring.as_ref().is_some_and(|(g, _)| Some(*g) != wanted) {
            acquiring = None;
        }
        if held.is_none()
            && acquiring.is_none()
            && let (Some(generation), Some(c)) = (wanted, connected.as_ref())
        {
            let conn = c.conn.clone();
            let owner = c.owner.clone();
            acquiring = Some((generation, Box::pin(bounded(inhibit(conn, owner)))));
        }
        let pick = {
            let control = async {
                let _ = wake.recv().await;
                Pick::Wake
            };
            let waiting = async {
                match query.as_mut() {
                    Some(q) => Pick::Query(q.as_mut().await),
                    None => std::future::pending().await,
                }
            };
            let signal = async {
                match connected.as_mut() {
                    Some(c) => Pick::Signal(c.next().await),
                    None => std::future::pending().await,
                }
            };
            let inhibitor = async {
                match acquiring.as_mut() {
                    Some((g, work)) => Pick::Inhibited(*g, work.as_mut().await),
                    None => std::future::pending().await,
                }
            };
            let timer = async {
                match retry {
                    Some(at) => {
                        Timer::at(at).await;
                        Pick::Retry
                    }
                    None => std::future::pending().await,
                }
            };
            future::or(
                control,
                future::or(
                    async { Pick::Lost(lost(&held).await) },
                    future::or(waiting, future::or(signal, future::or(inhibitor, timer))),
                ),
            )
            .await
        };
        let mut failure = None;
        match pick {
            Pick::Wake => {}
            Pick::Retry => {
                retry = None;
                let address = address.clone();
                query = Some(Box::pin(bounded(Connected::connect(address))));
                reconnect = true;
            }
            Pick::Query(result) => {
                query = None;
                match result {
                    Ok(mut c) => {
                        if reconnect {
                            let event = if !started {
                                started = true;
                                Event::Start {
                                    preparing_sleep: c.preparing,
                                }
                            } else {
                                Event::PrepareForSleep(c.preparing)
                            };
                            if send.send(Notice::Fact(event)).await.is_err() {
                                return;
                            }
                            c.sleep_edges.clear();
                        } else {
                            for value in c.sleep_edges.drain(..) {
                                if send
                                    .send(Notice::Fact(Event::PrepareForSleep(value)))
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                            }
                        }
                        if send
                            .send(Notice::Fact(Event::Session(c.fact.clone())))
                            .await
                            .is_err()
                        {
                            return;
                        }
                        connected = Some(c);
                        backoff = 100;
                    }
                    Err(error) => failure = Some(error),
                }
            }
            Pick::Signal(result) => match result {
                Err(error) => failure = Some(error),
                Ok((source, message)) => {
                    let c = connected.as_mut().expect("signal connection");
                    match source {
                        Source::Owner => match c.owner_changed(&message) {
                            Ok(false) => {}
                            Ok(true) => failure = Some(io::Error::other("logind owner changed")),
                            Err(e) => failure = Some(e),
                        },
                        Source::Sleep => match c.sleep_message(&message) {
                            Ok(()) => {
                                c.sleep_edges.clear();
                                if send
                                    .send(Notice::Fact(Event::PrepareForSleep(c.preparing)))
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            Err(e) => failure = Some(e),
                        },
                        source => {
                            let refresh = match relevant(source, &message) {
                                Ok(value) => value,
                                Err(error) => {
                                    failure = Some(error);
                                    false
                                }
                            };
                            if !refresh && failure.is_none() {
                                continue;
                            }
                            match c.invalidation(source, &message) {
                                Ok(Some(event)) => {
                                    if send.send(Notice::Fact(event)).await.is_err() {
                                        return;
                                    }
                                }
                                Ok(None) => {}
                                Err(e) => {
                                    failure = Some(e);
                                }
                            }
                            if failure.is_none() {
                                let mut c = connected.take().expect("refresh connection");
                                query = Some(Box::pin(bounded(async move {
                                    c.refresh().await?;
                                    Ok(c)
                                })));
                                reconnect = false;
                            }
                        }
                    }
                }
            },
            Pick::Lost(result) => {
                if let Some(lease) = held.take() {
                    let generation = lease.generation;
                    if *desired.borrow() == Some(generation) {
                        *desired.borrow_mut() = None;
                    }
                    let message = result
                        .err()
                        .map_or_else(|| "logind inhibitor FD closed".into(), |e| e.to_string());
                    if send
                        .send(Notice::Fact(Event::InhibitorFailed {
                            generation,
                            message,
                        }))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
            Pick::Inhibited(generation, result) => {
                acquiring = None;
                match result.and_then(Async::new) {
                    Ok(fd) => {
                        if *desired.borrow() == Some(generation) {
                            held = Some(Lease { generation, fd });
                            if send
                                .send(Notice::Fact(Event::InhibitorAcquired { generation }))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                    Err(error) => {
                        if *desired.borrow() == Some(generation) {
                            *desired.borrow_mut() = None;
                        }
                        if send
                            .send(Notice::Fact(Event::InhibitorFailed {
                                generation,
                                message: error.to_string(),
                            }))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
        }
        if let Some(error) = failure {
            connected = None;
            query = None;
            if let Some((generation, _)) = acquiring.take() {
                if *desired.borrow() == Some(generation) {
                    *desired.borrow_mut() = None;
                }
                if send
                    .send(Notice::Fact(Event::InhibitorFailed {
                        generation,
                        message: error.to_string(),
                    }))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            // SCM_RIGHTS inhibitor FDs outlive D-Bus. Keep a live lease until H4 releases it
            // after NativeSafe/its sleep deadline; FD HUP/ERR reports actual lease loss.
            if send.send(Notice::Fact(Event::Session(None))).await.is_err() {
                return;
            }
            if send
                .send(Notice::Unavailable(error.to_string()))
                .await
                .is_err()
            {
                return;
            }
            retry = Some(Instant::now() + Duration::from_millis(backoff));
            backoff = (backoff * 2).min(5000);
        }
    }
}
