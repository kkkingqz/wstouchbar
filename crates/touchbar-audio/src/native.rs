use crate::{model::*, *};
use pipewire as pw;
use pw::{
    proxy::ProxyT,
    spa::{
        self,
        param::ParamType,
        pod::{Object, Pod, Property, Value, ValueArray},
    },
};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    io::{Cursor, Read, Write},
    os::unix::net::UnixStream,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Default)]
pub struct NativeOptions {
    pub remote: Option<String>,
}
pub struct NativeAudio {
    inner: Arc<Inner>,
}
struct Inner {
    commands: mpsc::SyncSender<Request>,
    wake: UnixStream,
    hub: Arc<Mutex<Hub>>,
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<thread::JoinHandle<()>>>,
}
struct Request {
    command: AudioCommand,
    reply: async_channel::Sender<EffectOutcome>,
}
#[derive(Default)]
struct Hub {
    states: [Option<AudioState>; 2],
    next: u64,
    watchers: BTreeMap<
        u64,
        (
            AudioTarget,
            async_channel::Sender<Result<AudioState, EffectError>>,
        ),
    >,
}
impl Hub {
    fn publish(&mut self, graph: &Graph) {
        for target in [AudioTarget::DefaultSink, AudioTarget::DefaultSource] {
            let state = graph.state(target);
            let index = target_index(target);
            if self.states[index].as_ref() == Some(&state) {
                continue;
            }
            self.states[index] = Some(state.clone());
            self.watchers
                .retain(|_, (t, tx)| *t != target || tx.try_send(Ok(state.clone())).is_ok());
        }
    }
    fn lost(&mut self) {
        for (_, tx) in self.watchers.values() {
            let _ = tx.try_send(Err(EffectError::Unavailable));
        }
        self.watchers.clear();
        self.states = [None, None];
    }
}
impl NativeAudio {
    pub fn connect(options: NativeOptions) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::sync_channel(128);
        let (read, write) = UnixStream::pair()?;
        read.set_nonblocking(true)?;
        write.set_nonblocking(true)?;
        let hub = Arc::new(Mutex::new(Hub::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let h = hub.clone();
        let s = stop.clone();
        let thread = thread::Builder::new()
            .name("touchbar-pipewire".into())
            .spawn(move || worker(options, rx, h, s, read))?;
        Ok(Self {
            inner: Arc::new(Inner {
                commands: tx,
                wake: write,
                hub,
                stop,
                thread: Mutex::new(Some(thread)),
            }),
        })
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = (&self.wake).write(&[1]);
        if let Some(thread) = self.thread.lock().expect("audio thread lock").take() {
            let _ = thread.join();
        }
    }
}
impl AudioBackend for NativeAudio {
    fn command(&self, command: AudioCommand) -> AudioFuture {
        let (tx, rx) = async_channel::bounded(1);
        let sent = self
            .inner
            .commands
            .try_send(Request { command, reply: tx })
            .is_ok();
        if sent {
            let _ = (&self.inner.wake).write(&[1]);
        }
        Box::pin(async move {
            if !sent {
                return Err(EffectError::Unavailable);
            }
            rx.recv().await.unwrap_or(Err(EffectError::Unavailable))
        })
    }
    fn watch(&self, target: AudioTarget) -> Result<AudioWatch, EffectError> {
        let (tx, rx) = async_channel::bounded(64);
        let mut hub = self.inner.hub.lock().expect("audio hub lock");
        if hub.watchers.len() >= 256 {
            return Err(EffectError::Unavailable);
        }
        let id = hub.next;
        hub.next = hub.next.checked_add(1).ok_or(EffectError::Unavailable)?;
        tx.try_send(Ok(hub.states[target_index(target)]
            .clone()
            .unwrap_or_else(unavailable)))
            .map_err(|_| EffectError::Unavailable)?;
        hub.watchers.insert(id, (target, tx));
        let inner = self.inner.clone();
        Ok(AudioWatch::new(rx, move || {
            inner
                .hub
                .lock()
                .expect("audio hub lock")
                .watchers
                .remove(&id);
        }))
    }
}
fn worker(
    options: NativeOptions,
    rx: mpsc::Receiver<Request>,
    hub: Arc<Mutex<Hub>>,
    stop: Arc<AtomicBool>,
    wake: UnixStream,
) {
    pw::init();
    let mut backoff = touchbar_desktop::ReconnectBackoff::default();
    while !stop.load(Ordering::Acquire) {
        let connected = connection(&options, &rx, &hub, &stop, &wake).unwrap_or(false);
        if connected {
            hub.lock().expect("audio hub lock").lost();
            backoff.reset();
        }
        if stop.load(Ordering::Acquire) {
            break;
        }
        let wait = backoff.failed(Timestamp(0)).0;
        let deadline = Instant::now() + Duration::from_millis(wait);
        while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
            if let Ok(request) = rx.recv_timeout(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(50)),
            ) {
                let _ = request.reply.try_send(Err(EffectError::Unavailable));
            }
        }
    }
}
struct Objects {
    nodes: BTreeMap<u32, pw::node::Node>,
    devices: BTreeMap<u32, pw::device::Device>,
    metadata: BTreeMap<u32, pw::metadata::Metadata>,
    listeners: BTreeMap<u32, Box<dyn pw::proxy::Listener>>,
}
impl Objects {
    fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            devices: BTreeMap::new(),
            metadata: BTreeMap::new(),
            listeners: BTreeMap::new(),
        }
    }
}
struct Pending {
    seq: spa::utils::result::AsyncSeq,
    object: u32,
    reply: async_channel::Sender<EffectOutcome>,
    result: EffectOutcome,
    deadline: Instant,
}
fn connection(
    options: &NativeOptions,
    rx: &mpsc::Receiver<Request>,
    hub: &Arc<Mutex<Hub>>,
    stop: &AtomicBool,
    wake: &UnixStream,
) -> Result<bool, pw::Error> {
    let main = pw::main_loop::MainLoopRc::new(None)?;
    let wake_fd = wake.try_clone().map_err(|_| pw::Error::CreationFailed)?;
    let _wake = main
        .loop_()
        .add_io(wake_fd, spa::support::system::IoFlags::IN, move |stream| {
            let mut bytes = [0u8; 256];
            while stream.read(&mut bytes).is_ok_and(|n| n > 0) {}
        });
    let context = pw::context::ContextRc::new(&main, None)?;
    let props=options.remote.as_ref().map(|r|pw::properties::properties!{"remote.name"=>r.as_str(),"application.name"=>"wstouchbar"});
    let core = context.connect_rc(props)?;
    let registry = core.get_registry_rc()?;
    let graph = Rc::new(RefCell::new(Graph::default()));
    let objects = Rc::new(RefCell::new(Objects::new()));
    let pending: Rc<RefCell<Option<Pending>>> = Rc::new(RefCell::new(None));
    let ready = Rc::new(Cell::new(false));
    let lost = Rc::new(Cell::new(false));
    let r = ready.clone();
    let l = lost.clone();
    let p = pending.clone();
    let p2 = pending.clone();
    let _core_listener = core
        .add_listener_local()
        .info(move |_| r.set(true))
        .error(move |id, _, _, _| {
            if id == 0 {
                l.set(true);
            }
            if p.borrow()
                .as_ref()
                .is_some_and(|p| p.object == id || id == 0)
                && let Some(p) = p.borrow_mut().take()
            {
                let _ = p.reply.try_send(Err(EffectError::Unavailable));
            }
        })
        .done(move |_, seq| {
            if p2.borrow().as_ref().is_some_and(|p| p.seq == seq)
                && let Some(p) = p2.borrow_mut().take()
            {
                let _ = p.reply.try_send(p.result);
            }
        })
        .register();
    let reg = registry.downgrade();
    let g = graph.clone();
    let o = objects.clone();
    let h = hub.clone();
    let g2 = graph.clone();
    let o2 = objects.clone();
    let h2 = hub.clone();
    let _registry_listener = registry
        .add_listener_local()
        .global(move |global| {
            let Some(reg) = reg.upgrade() else { return };
            let Some(props) = global.props else { return };
            if o.borrow().listeners.len() >= 1024 {
                return;
            }
            let id = global.id;
            match global.type_ {
                pw::types::ObjectType::Node
                    if props.get("media.class").is_some_and(|c| {
                        c == "Audio/Sink" || c == "Audio/Source" || c == "Audio/Source/Virtual"
                    }) =>
                {
                    let Ok(node) = reg.bind::<pw::node::Node, _>(global) else {
                        return;
                    };
                    let Some(name) = props.get("node.name").filter(|n| n.len() <= 1024) else {
                        return;
                    };
                    let description = props
                        .get("node.description")
                        .or_else(|| props.get("node.nick"))
                        .unwrap_or(name)
                        .chars()
                        .take(256)
                        .collect();
                    let device = props.get("device.id").and_then(|s| s.parse().ok()).zip(
                        props
                            .get("card.profile.device")
                            .and_then(|s| s.parse().ok()),
                    );
                    g.borrow_mut().nodes.insert(
                        id,
                        Node {
                            name: name.into(),
                            description,
                            device,
                            volume: None,
                        },
                    );
                    let g1 = g.clone();
                    let h1 = h.clone();
                    let gi = g.clone();
                    let hi = h.clone();
                    let listener = node
                        .add_listener_local()
                        .info(move |info| {
                            if let Some(props) = info.props() {
                                let mut graph = gi.borrow_mut();
                                if let Some(node) = graph.nodes.get_mut(&id) {
                                    if let Some(name) = props.get("node.description") {
                                        node.description = name.chars().take(256).collect();
                                    }
                                    node.device =
                                        props.get("device.id").and_then(|s| s.parse().ok()).zip(
                                            props
                                                .get("card.profile.device")
                                                .and_then(|s| s.parse().ok()),
                                        );
                                }
                                hi.lock().expect("audio hub lock").publish(&graph);
                            }
                        })
                        .param(move |_, kind, _, _, pod| {
                            if kind == ParamType::Props
                                && let Some(v) = pod.and_then(parse_volume)
                            {
                                if let Some(node) = g1.borrow_mut().nodes.get_mut(&id) {
                                    node.volume = Some(v);
                                }
                                h1.lock().expect("audio hub lock").publish(&g1.borrow());
                            }
                        })
                        .register();
                    node.subscribe_params(&[ParamType::Props]);
                    node.enum_params(0, Some(ParamType::Props), 0, u32::MAX);
                    let mut objects = o.borrow_mut();
                    objects.listeners.insert(id, Box::new(listener));
                    objects.nodes.insert(id, node);
                }
                pw::types::ObjectType::Device => {
                    let Ok(device) = reg.bind::<pw::device::Device, _>(global) else {
                        return;
                    };
                    let g1 = g.clone();
                    let h1 = h.clone();
                    let listener = device
                        .add_listener_local()
                        .param(move |_, kind, index, _, pod| {
                            if kind == ParamType::Route {
                                let mut graph = g1.borrow_mut();
                                if index == 0 {
                                    graph.routes.retain(|(d, _), _| *d != id);
                                }
                                if let Some((index, route)) = pod.and_then(parse_route)
                                    && graph.routes.len() < 1024
                                {
                                    graph.routes.insert((id, index), route);
                                }
                                h1.lock().expect("audio hub lock").publish(&graph);
                            }
                        })
                        .register();
                    device.subscribe_params(&[ParamType::Route]);
                    device.enum_params(0, Some(ParamType::Route), 0, u32::MAX);
                    let mut objects = o.borrow_mut();
                    objects.listeners.insert(id, Box::new(listener));
                    objects.devices.insert(id, device);
                }
                pw::types::ObjectType::Metadata
                    if props.get("metadata.name") == Some("default") =>
                {
                    let Ok(metadata) = reg.bind::<pw::metadata::Metadata, _>(global) else {
                        return;
                    };
                    let g1 = g.clone();
                    let h1 = h.clone();
                    let listener = metadata
                        .add_listener_local()
                        .property(move |subject, key, _, value| {
                            if subject == 0 {
                                let mut graph = g1.borrow_mut();
                                if let Some(key) = key {
                                    graph.metadata(key, value);
                                } else {
                                    graph.metadata("default.audio.sink", None);
                                    graph.metadata("default.audio.source", None);
                                }
                                h1.lock().expect("audio hub lock").publish(&graph);
                            }
                            0
                        })
                        .register();
                    let mut objects = o.borrow_mut();
                    objects.listeners.insert(id, Box::new(listener));
                    objects.metadata.insert(id, metadata);
                }
                _ => {}
            }
        })
        .global_remove(move |id| {
            let mut objects = o2.borrow_mut();
            objects.listeners.remove(&id);
            objects.nodes.remove(&id);
            objects.devices.remove(&id);
            if objects.metadata.remove(&id).is_some() {
                g2.borrow_mut().metadata("default.audio.sink", None);
                g2.borrow_mut().metadata("default.audio.source", None);
            }
            g2.borrow_mut().remove(id);
            h2.lock().expect("audio hub lock").publish(&g2.borrow());
        })
        .register();
    while !stop.load(Ordering::Acquire) && !lost.get() {
        if pending
            .borrow()
            .as_ref()
            .is_some_and(|p| p.reply.is_closed() || Instant::now() >= p.deadline)
            && let Some(p) = pending.borrow_mut().take()
        {
            let _ = p.reply.try_send(Err(EffectError::Timeout));
        }
        let mut consumed = false;
        if pending.borrow().is_none()
            && let Ok(request) = rx.try_recv()
        {
            consumed = true;
            if request.reply.is_closed() {
                continue;
            }
            match write_command(&request.command, &graph.borrow(), &objects.borrow()) {
                Ok((object, result)) => match core.sync(0) {
                    Ok(seq) => {
                        *pending.borrow_mut() = Some(Pending {
                            seq,
                            object,
                            reply: request.reply,
                            result,
                            deadline: Instant::now() + Duration::from_secs(1),
                        })
                    }
                    Err(_) => {
                        let _ = request.reply.try_send(Err(EffectError::Unavailable));
                    }
                },
                Err(error) => {
                    let _ = request.reply.try_send(Err(error));
                }
            }
        }
        // Drain queued commands without sleeping; otherwise the native loop waits on
        // PipeWire or the wake socket, with only an in-flight request's deadline.
        let timeout = if pending.borrow().is_none() && consumed {
            pw::loop_::Timeout::None
        } else {
            pending
                .borrow()
                .as_ref()
                .map_or(pw::loop_::Timeout::Infinite, |p| {
                    pw::loop_::Timeout::Finite(p.deadline.saturating_duration_since(Instant::now()))
                })
        };
        main.loop_().iterate(timeout);
    }
    if let Some(p) = pending.borrow_mut().take() {
        let _ = p.reply.try_send(Err(EffectError::Unavailable));
    }
    Ok(ready.get())
}
fn object(pod: &Pod) -> Option<Object> {
    if pod.as_bytes().len() > 64 * 1024 {
        return None;
    }
    match spa::pod::deserialize::PodDeserializer::deserialize_any_from(pod.as_bytes())
        .ok()?
        .1
    {
        Value::Object(o) => Some(o),
        _ => None,
    }
}
fn field(object: &Object, key: u32) -> Option<&Value> {
    object
        .properties
        .iter()
        .find(|p| p.key == key)
        .map(|p| &p.value)
}
fn int(object: &Object, key: u32) -> Option<i32> {
    if let Value::Int(i) = field(object, key)? {
        Some(*i)
    } else {
        None
    }
}
fn volume(object: &Object) -> Option<Volume> {
    let channels = match field(object, spa::sys::SPA_PROP_channelVolumes) {
        Some(Value::ValueArray(ValueArray::Float(v))) => v.clone(),
        _ => match field(object, spa::sys::SPA_PROP_volume) {
            Some(Value::Float(v)) => vec![*v],
            _ => return None,
        },
    };
    if channels.is_empty()
        || channels.len() > 64
        || channels.iter().any(|v| !v.is_finite() || *v < 0.0)
    {
        return None;
    }
    let muted = matches!(
        field(object, spa::sys::SPA_PROP_mute),
        Some(Value::Bool(true))
    );
    Some(Volume { channels, muted })
}
fn parse_volume(pod: &Pod) -> Option<Volume> {
    volume(&object(pod)?)
}
fn parse_route(pod: &Pod) -> Option<(i32, Route)> {
    let o = object(pod)?;
    let index = int(&o, spa::sys::SPA_PARAM_ROUTE_index)?;
    let profile_device = int(&o, spa::sys::SPA_PARAM_ROUTE_device)?;
    let Value::Object(props) = field(&o, spa::sys::SPA_PARAM_ROUTE_props)? else {
        return None;
    };
    Some((
        index,
        Route {
            profile_device,
            volume: volume(props)?,
        },
    ))
}
fn write_command(
    command: &AudioCommand,
    graph: &Graph,
    objects: &Objects,
) -> Result<(u32, EffectOutcome), EffectError> {
    let target = match command {
        AudioCommand::SetVolume { target, .. } | AudioCommand::SetMute { target, .. } => *target,
    };
    let (control, current) = graph.control(target).ok_or(EffectError::Unavailable)?;
    let (mut properties, result) = (Vec::new(), Ok(EffectResult::None));
    let result = match command {
        AudioCommand::SetVolume { level, .. } => {
            let linear = to_linear(*level)?;
            properties.push(Property::new(
                spa::sys::SPA_PROP_channelVolumes,
                Value::ValueArray(ValueArray::Float(vec![linear; current.channels.len()])),
            ));
            result
        }
        AudioCommand::SetMute { value, .. } => {
            let muted = match value {
                MuteValue::On => true,
                MuteValue::Off => false,
                MuteValue::Toggle => !current.muted,
            };
            properties.push(Property::new(spa::sys::SPA_PROP_mute, Value::Bool(muted)));
            Ok(EffectResult::Mute(muted))
        }
    };
    let props = Object {
        type_: spa::sys::SPA_TYPE_OBJECT_Props,
        id: ParamType::Props.as_raw(),
        properties,
    };
    let value = match control {
        Control::Node(_) => Value::Object(props),
        Control::Route {
            device: _,
            index,
            profile_device,
        } => Value::Object(Object {
            type_: spa::sys::SPA_TYPE_OBJECT_ParamRoute,
            id: ParamType::Route.as_raw(),
            properties: vec![
                Property::new(spa::sys::SPA_PARAM_ROUTE_index, Value::Int(index)),
                Property::new(spa::sys::SPA_PARAM_ROUTE_device, Value::Int(profile_device)),
                Property::new(spa::sys::SPA_PARAM_ROUTE_props, Value::Object(props)),
                Property::new(spa::sys::SPA_PARAM_ROUTE_save, Value::Bool(true)),
            ],
        }),
    };
    let (bytes, _) = spa::pod::serialize::PodSerializer::serialize(Cursor::new(Vec::new()), &value)
        .map_err(|e| EffectError::Failed(e.to_string()))?;
    let bytes = bytes.into_inner();
    let pod = Pod::from_bytes(&bytes).ok_or(EffectError::Unavailable)?;
    let proxy = match control {
        Control::Node(id) => {
            let node = objects.nodes.get(&id).ok_or(EffectError::Unavailable)?;
            node.set_param(ParamType::Props, 0, pod);
            node.upcast_ref().id()
        }
        Control::Route { device, .. } => {
            let device = objects
                .devices
                .get(&device)
                .ok_or(EffectError::Unavailable)?;
            device.set_param(ParamType::Route, 0, pod);
            device.upcast_ref().id()
        }
    };
    Ok((proxy, result))
}
