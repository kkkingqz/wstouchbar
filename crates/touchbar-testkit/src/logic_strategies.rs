//! Component boundary strategies. Every update contains every payload variant.
use proptest::prelude::*;
use touchbar_model::*;

pub fn logic_update_strategy() -> impl Strategy<Value = LogicUpdate> {
    (any::<u64>(), ".{0,40}").prop_map(|(seed, text)| logic_update_sample(seed, text))
}
pub fn logic_event_strategy() -> impl Strategy<Value = LogicEvent> {
    (any::<u64>(), ".{0,40}").prop_map(|(seed, text)| logic_event_sample(seed, text))
}

pub fn logic_update_sample(seed: u64, text: String) -> LogicUpdate {
    let n = f64::from_bits(seed);
    let f = f32::from_bits(u32::try_from(seed & 0xffff_ffff).unwrap());
    let p = Point { x: f, y: -f };
    let r = Rect {
        x: f,
        y: -f,
        width: f,
        height: -f,
    };
    let roles = [
        ColorRole::Background,
        ColorRole::Control,
        ColorRole::ControlPressed,
        ColorRole::Track,
        ColorRole::Foreground,
        ColorRole::Muted,
        ColorRole::Accent,
        ColorRole::Destructive,
        ColorRole::OnAccent,
        ColorRole::OnDestructive,
    ];
    let color = if seed & 1 == 0 {
        CanvasColor::Role(roles[usize::try_from((seed >> 8) % 10).unwrap()])
    } else {
        CanvasColor::Rgba {
            red: f,
            green: -f,
            blue: f,
            alpha: -f,
        }
    };
    let paint = CanvasPaint { color, opacity: f };
    let gradient = CanvasLinearGradient {
        start: p,
        end: p,
        start_color: paint,
        end_color: paint,
    };
    let segments = vec![
        CanvasPathSegment::MoveTo(p),
        CanvasPathSegment::LineTo(p),
        CanvasPathSegment::QuadraticTo {
            control: p,
            endpoint: p,
        },
        CanvasPathSegment::CubicTo {
            control_one: p,
            control_two: p,
            endpoint: p,
        },
        CanvasPathSegment::Close,
    ];
    let canvas = vec![
        CanvasCommand::FillRect {
            rect: r,
            radius: f,
            paint,
        },
        CanvasCommand::Line {
            start: p,
            end: p,
            width: f,
            paint,
        },
        CanvasCommand::Polyline {
            points: vec![p],
            width: f,
            paint,
        },
        CanvasCommand::FillCircle {
            center: p,
            radius: f,
            paint,
        },
        CanvasCommand::Text {
            rect: r,
            text: text.clone(),
            size: f,
            paint,
            align: [TextAlign::Leading, TextAlign::Center, TextAlign::Trailing]
                [usize::try_from((seed >> 9) % 3).unwrap()],
        },
        CanvasCommand::FillLinearGradientRect {
            rect: r,
            radius: f,
            gradient,
        },
        CanvasCommand::StrokePath {
            segments: segments.clone(),
            width: f,
            paint,
        },
        CanvasCommand::FillPath { segments, paint },
    ];
    let spawn = SpawnSpec {
        argv: vec!["cmd".into(), text.clone()],
        env: [("NAME".into(), text.clone())].into(),
        stdin: Some(text.clone()),
        max_output: u32::try_from(seed & 0xffff_ffff).unwrap(),
    };
    let dbus = DbusValue {
        signature: text.clone(),
        json: text.clone(),
    };
    let bus = if seed & 2 == 0 {
        Bus::Session
    } else {
        Bus::System
    };
    let target = if seed & 4 == 0 {
        AudioTarget::DefaultSink
    } else {
        AudioTarget::DefaultSource
    };
    let key = system_keys()[usize::try_from((seed >> 7) % 25).unwrap()];
    let effects = vec![
        Effect::AudioSetVolume { target, level: n },
        Effect::AudioSetMute {
            target,
            value: [MuteValue::On, MuteValue::Off, MuteValue::Toggle]
                [usize::try_from((seed >> 6) % 3).unwrap()],
        },
        Effect::MediaPlayPause,
        Effect::MediaNext,
        Effect::MediaPrevious,
        Effect::MediaSeek { position_ms: seed },
        Effect::DesktopShortcut {
            keys: vec![text.clone()],
            app: Some(text.clone()),
        },
        Effect::DesktopBrightness { level: n },
        Effect::DesktopWorkspace { id: text.clone() },
        Effect::DesktopAppLaunch { id: text.clone() },
        Effect::KeyTap { key },
        Effect::KeyDown { key },
        Effect::KeyUp { key },
        Effect::OpenUri { uri: text.clone() },
        Effect::TimerAfter { ms: seed },
        Effect::SpawnRun(spawn.clone()),
        Effect::DbusCall(DbusCall {
            bus,
            destination: text.clone(),
            path: text.clone(),
            interface: text.clone(),
            method: text.clone(),
            args: dbus,
        }),
    ];
    let subscriptions = vec![
        SubscriptionKind::AudioWatch { target },
        SubscriptionKind::MediaWatch,
        SubscriptionKind::DesktopFact {
            name: text.clone().into(),
        },
        SubscriptionKind::TimerInterval { ms: seed },
        SubscriptionKind::SpawnLines(spawn),
        SubscriptionKind::DbusSignal(DbusMatch {
            bus,
            sender: Some(text.clone()),
            path: Some(text.clone()),
            interface: Some(text.clone()),
            member: Some(text.clone()),
        }),
        SubscriptionKind::DbusProperty(DbusProperty {
            bus,
            destination: text.clone(),
            path: text.clone(),
            interface: text.clone(),
            property: text.clone(),
        }),
    ];
    LogicUpdate {
        state: vec![
            (
                "bool".into(),
                StateValue::Scalar(Scalar::Boolean(seed & 8 != 0)),
            ),
            ("number".into(), StateValue::Scalar(Scalar::Number(n))),
            (
                "text".into(),
                StateValue::Scalar(Scalar::Text(text.clone())),
            ),
            (
                "records".into(),
                StateValue::Records(vec![
                    ([
                        ("label".into(), Scalar::Text(text.clone())),
                        ("value".into(), Scalar::Number(n)),
                        ("flag".into(), Scalar::Boolean(seed & 8 != 0)),
                    ])
                    .into(),
                ]),
            ),
            ("canvas".into(), StateValue::Canvas(canvas)),
        ],
        requests: effects
            .into_iter()
            .enumerate()
            .map(|(i, effect)| EffectRequest {
                tag: format!("request-{i}"),
                effect,
                lifetime: if seed & 16 == 0 {
                    None
                } else {
                    Some(if seed & 32 == 0 {
                        Lifetime::Committed
                    } else {
                        Lifetime::UntilHidden
                    })
                },
                coalesce: (seed & 64 != 0).then(|| text.clone()),
                timeout_ms: (seed & 128 != 0).then_some(seed),
            })
            .collect(),
        cancel: vec![text.clone()],
        subscriptions: (seed & 256 != 0).then(|| {
            subscriptions
                .into_iter()
                .enumerate()
                .map(|(i, kind)| SubscriptionSpec {
                    tag: format!("subscription-{i}"),
                    kind,
                    background: seed & 512 != 0,
                })
                .collect()
        }),
        presentation: match (seed >> 10) % 3 {
            0 => None,
            1 => Some(PresentationCommand::Close),
            _ => Some(PresentationCommand::Open {
                panel: "panel".parse().unwrap(),
                placement: [Placement::Anchored, Placement::Region, Placement::Full]
                    [usize::try_from((seed >> 12) % 3).unwrap()],
                lifecycle: if seed & 16384 == 0 {
                    PresentationLifecycle::Transient
                } else {
                    PresentationLifecycle::Persistent
                },
            }),
        },
    }
}

pub fn logic_event_sample(seed: u64, text: String) -> LogicEvent {
    let n = f64::from_bits(seed);
    let scalar = match (seed >> 8) % 3 {
        0 => Scalar::Boolean(seed & 16 != 0),
        1 => Scalar::Number(n),
        _ => Scalar::Text(text.clone()),
    };
    let error = match (seed >> 10) % 5 {
        0 => EffectError::Unsupported,
        1 => EffectError::Unavailable,
        2 => EffectError::Failed(text.clone()),
        3 => EffectError::Timeout,
        _ => EffectError::Cancelled,
    };
    let dbus = DbusValue {
        signature: text.clone(),
        json: text.clone(),
    };
    match seed % 8 {
        0 => LogicEvent::Action(ActionEvent {
            action: text.clone(),
            kind: [
                ActionKind::Pressed,
                ActionKind::LongPressed,
                ActionKind::Activated,
                ActionKind::Released,
                ActionKind::Cancelled,
                ActionKind::ValueChanged,
                ActionKind::Selected,
            ][usize::try_from((seed >> 3) % 7).unwrap()],
            value: (seed & 8192 != 0).then_some(scalar),
            record_key: (seed & 16384 != 0).then_some(text),
            origin: if seed & 32768 == 0 {
                Origin::Physical
            } else {
                Origin::Synthetic
            },
            cancel_reason: (seed & 65536 != 0).then(|| {
                [
                    CancelReason::Hidden,
                    CancelReason::Layer,
                    CancelReason::Presentation,
                    CancelReason::Profile,
                    CancelReason::Layout,
                    CancelReason::Mode,
                    CancelReason::Revoked,
                    CancelReason::Fault,
                    CancelReason::Hardware,
                    CancelReason::SurfaceRemoved,
                ][usize::try_from((seed >> 17) % 10).unwrap()]
            }),
        }),
        1 => LogicEvent::Shown,
        2 => LogicEvent::Hidden,
        3 => LogicEvent::ContextChanged(LogicContext {
            surface_kind: text.clone(),
            slot_width: f32::from_bits(u32::try_from(seed & 0xffff_ffff).unwrap()),
            representation: text.clone(),
            color_scheme: text.clone(),
            accent: (seed & 8192 != 0).then(|| text.clone()),
            locale: text,
        }),
        4 => LogicEvent::PresentationClosed("panel".parse().unwrap()),
        5 => LogicEvent::EffectDone {
            tag: text.clone(),
            result: match (seed >> 3) % 5 {
                0 => Ok(EffectResult::None),
                1 => Ok(EffectResult::Mute(seed & 8192 != 0)),
                2 => Ok(EffectResult::Spawn(SpawnResult {
                    status: i32::from_ne_bytes(
                        u32::try_from(seed & 0xffff_ffff).unwrap().to_ne_bytes(),
                    ),
                    stdout: text.clone(),
                    stderr: text,
                    truncated: seed & 8192 != 0,
                })),
                3 => Ok(EffectResult::Dbus(dbus)),
                _ => Err(error),
            },
        },
        6 => LogicEvent::SubscriptionData {
            tag: text.clone(),
            data: match (seed >> 3) % 6 {
                0 => SubscriptionData::Audio(AudioState {
                    available: seed & 8192 != 0,
                    volume: n,
                    muted: seed & 16384 != 0,
                    name: text,
                }),
                1 => SubscriptionData::Media(MediaState {
                    player: (seed & 8192 != 0).then(|| text.clone()),
                    status: [
                        PlaybackStatus::Playing,
                        PlaybackStatus::Paused,
                        PlaybackStatus::Stopped,
                    ][usize::try_from((seed >> 6) % 3).unwrap()],
                    title: text.clone(),
                    artist: text.clone(),
                    album: text.clone(),
                    art_uri: (seed & 16384 != 0).then_some(text),
                    position_ms: seed,
                    length_ms: seed,
                    can_play: seed & 32768 != 0,
                    can_pause: seed & 65536 != 0,
                    can_next: seed & 131072 != 0,
                    can_previous: seed & 262144 != 0,
                    can_seek: seed & 524288 != 0,
                }),
                2 => SubscriptionData::Fact(match (seed >> 6) % 6 {
                    0 => None,
                    1 => Some(FactValue::Boolean(seed & 8192 != 0)),
                    2 => Some(FactValue::Number(n)),
                    3 => Some(FactValue::Text(text.clone())),
                    4 => Some(FactValue::List(vec![text.clone()])),
                    _ => Some(FactValue::Records(vec![
                        ([("value".into(), scalar)]).into(),
                    ])),
                }),
                3 => SubscriptionData::Tick,
                4 => SubscriptionData::Line(text),
                _ => SubscriptionData::Dbus(dbus),
            },
        },
        _ => LogicEvent::SubscriptionLost { tag: text, error },
    }
}
fn system_keys() -> [SystemKey; 25] {
    [
        SystemKey::Esc,
        SystemKey::F1,
        SystemKey::F2,
        SystemKey::F3,
        SystemKey::F4,
        SystemKey::F5,
        SystemKey::F6,
        SystemKey::F7,
        SystemKey::F8,
        SystemKey::F9,
        SystemKey::F10,
        SystemKey::F11,
        SystemKey::F12,
        SystemKey::BrightnessDown,
        SystemKey::BrightnessUp,
        SystemKey::KeyboardIlluminationDown,
        SystemKey::KeyboardIlluminationUp,
        SystemKey::PreviousSong,
        SystemKey::PlayPause,
        SystemKey::NextSong,
        SystemKey::Mute,
        SystemKey::VolumeDown,
        SystemKey::VolumeUp,
        SystemKey::MicrophoneMute,
        SystemKey::Search,
    ]
}
