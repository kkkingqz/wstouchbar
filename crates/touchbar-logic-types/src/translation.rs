//! Instantiates the same model translation against host or guest generated types.
//! No bindings or external ABI dependencies are compiled in this crate.
#[macro_export]
macro_rules! define_wit_translations {
    ($api:path) => {
        use touchbar_model as m;
        use $api as w;
        impl From<m::ScalarKind> for w::ScalarKind {
            fn from(v: m::ScalarKind) -> Self {
                match v {
                    m::ScalarKind::Boolean => Self::Boolean,
                    m::ScalarKind::Number => Self::Number,
                    m::ScalarKind::Text => Self::Text,
                }
            }
        }
        impl From<w::ScalarKind> for m::ScalarKind {
            fn from(v: w::ScalarKind) -> Self {
                match v {
                    w::ScalarKind::Boolean => Self::Boolean,
                    w::ScalarKind::Number => Self::Number,
                    w::ScalarKind::Text => Self::Text,
                }
            }
        }
        impl From<m::ColorRole> for w::ColorRole {
            fn from(v: m::ColorRole) -> Self {
                match v {
                    m::ColorRole::Background => Self::Background,
                    m::ColorRole::Control => Self::Control,
                    m::ColorRole::ControlPressed => Self::ControlPressed,
                    m::ColorRole::Track => Self::Track,
                    m::ColorRole::Foreground => Self::Foreground,
                    m::ColorRole::Muted => Self::Muted,
                    m::ColorRole::Accent => Self::Accent,
                    m::ColorRole::Destructive => Self::Destructive,
                    m::ColorRole::OnAccent => Self::OnAccent,
                    m::ColorRole::OnDestructive => Self::OnDestructive,
                }
            }
        }
        impl From<w::ColorRole> for m::ColorRole {
            fn from(v: w::ColorRole) -> Self {
                match v {
                    w::ColorRole::Background => Self::Background,
                    w::ColorRole::Control => Self::Control,
                    w::ColorRole::ControlPressed => Self::ControlPressed,
                    w::ColorRole::Track => Self::Track,
                    w::ColorRole::Foreground => Self::Foreground,
                    w::ColorRole::Muted => Self::Muted,
                    w::ColorRole::Accent => Self::Accent,
                    w::ColorRole::Destructive => Self::Destructive,
                    w::ColorRole::OnAccent => Self::OnAccent,
                    w::ColorRole::OnDestructive => Self::OnDestructive,
                }
            }
        }
        impl From<m::TextAlign> for w::TextAlign {
            fn from(v: m::TextAlign) -> Self {
                match v {
                    m::TextAlign::Leading => Self::Leading,
                    m::TextAlign::Center => Self::Center,
                    m::TextAlign::Trailing => Self::Trailing,
                }
            }
        }
        impl From<w::TextAlign> for m::TextAlign {
            fn from(v: w::TextAlign) -> Self {
                match v {
                    w::TextAlign::Leading => Self::Leading,
                    w::TextAlign::Center => Self::Center,
                    w::TextAlign::Trailing => Self::Trailing,
                }
            }
        }
        impl From<m::ActionKind> for w::ActionKind {
            fn from(v: m::ActionKind) -> Self {
                match v {
                    m::ActionKind::Pressed => Self::Pressed,
                    m::ActionKind::LongPressed => Self::LongPressed,
                    m::ActionKind::Activated => Self::Activated,
                    m::ActionKind::Released => Self::Released,
                    m::ActionKind::Cancelled => Self::Cancelled,
                    m::ActionKind::ValueChanged => Self::ValueChanged,
                    m::ActionKind::Selected => Self::Selected,
                }
            }
        }
        impl From<w::ActionKind> for m::ActionKind {
            fn from(v: w::ActionKind) -> Self {
                match v {
                    w::ActionKind::Pressed => Self::Pressed,
                    w::ActionKind::LongPressed => Self::LongPressed,
                    w::ActionKind::Activated => Self::Activated,
                    w::ActionKind::Released => Self::Released,
                    w::ActionKind::Cancelled => Self::Cancelled,
                    w::ActionKind::ValueChanged => Self::ValueChanged,
                    w::ActionKind::Selected => Self::Selected,
                }
            }
        }
        impl From<m::Origin> for w::Origin {
            fn from(v: m::Origin) -> Self {
                match v {
                    m::Origin::Physical => Self::Physical,
                    m::Origin::Synthetic => Self::Synthetic,
                }
            }
        }
        impl From<w::Origin> for m::Origin {
            fn from(v: w::Origin) -> Self {
                match v {
                    w::Origin::Physical => Self::Physical,
                    w::Origin::Synthetic => Self::Synthetic,
                }
            }
        }
        impl From<m::CancelReason> for w::CancelReason {
            fn from(v: m::CancelReason) -> Self {
                match v {
                    m::CancelReason::Hidden => Self::Hidden,
                    m::CancelReason::Layer => Self::Layer,
                    m::CancelReason::Presentation => Self::Presentation,
                    m::CancelReason::Profile => Self::Profile,
                    m::CancelReason::Layout => Self::Layout,
                    m::CancelReason::Mode => Self::Mode,
                    m::CancelReason::Revoked => Self::Revoked,
                    m::CancelReason::Fault => Self::Fault,
                    m::CancelReason::Hardware => Self::Hardware,
                    m::CancelReason::SurfaceRemoved => Self::SurfaceRemoved,
                }
            }
        }
        impl From<w::CancelReason> for m::CancelReason {
            fn from(v: w::CancelReason) -> Self {
                match v {
                    w::CancelReason::Hidden => Self::Hidden,
                    w::CancelReason::Layer => Self::Layer,
                    w::CancelReason::Presentation => Self::Presentation,
                    w::CancelReason::Profile => Self::Profile,
                    w::CancelReason::Layout => Self::Layout,
                    w::CancelReason::Mode => Self::Mode,
                    w::CancelReason::Revoked => Self::Revoked,
                    w::CancelReason::Fault => Self::Fault,
                    w::CancelReason::Hardware => Self::Hardware,
                    w::CancelReason::SurfaceRemoved => Self::SurfaceRemoved,
                }
            }
        }
        impl From<m::AudioTarget> for w::AudioTarget {
            fn from(v: m::AudioTarget) -> Self {
                match v {
                    m::AudioTarget::DefaultSink => Self::DefaultSink,
                    m::AudioTarget::DefaultSource => Self::DefaultSource,
                }
            }
        }
        impl From<w::AudioTarget> for m::AudioTarget {
            fn from(v: w::AudioTarget) -> Self {
                match v {
                    w::AudioTarget::DefaultSink => Self::DefaultSink,
                    w::AudioTarget::DefaultSource => Self::DefaultSource,
                }
            }
        }
        impl From<m::MuteValue> for w::MuteValue {
            fn from(v: m::MuteValue) -> Self {
                match v {
                    m::MuteValue::On => Self::On,
                    m::MuteValue::Off => Self::Off,
                    m::MuteValue::Toggle => Self::Toggle,
                }
            }
        }
        impl From<w::MuteValue> for m::MuteValue {
            fn from(v: w::MuteValue) -> Self {
                match v {
                    w::MuteValue::On => Self::On,
                    w::MuteValue::Off => Self::Off,
                    w::MuteValue::Toggle => Self::Toggle,
                }
            }
        }
        impl From<m::Bus> for w::Bus {
            fn from(v: m::Bus) -> Self {
                match v {
                    m::Bus::Session => Self::Session,
                    m::Bus::System => Self::System,
                }
            }
        }
        impl From<w::Bus> for m::Bus {
            fn from(v: w::Bus) -> Self {
                match v {
                    w::Bus::Session => Self::Session,
                    w::Bus::System => Self::System,
                }
            }
        }
        impl From<m::SystemKey> for w::SystemKey {
            fn from(v: m::SystemKey) -> Self {
                match v {
                    m::SystemKey::Esc => Self::Esc,
                    m::SystemKey::F1 => Self::F1,
                    m::SystemKey::F2 => Self::F2,
                    m::SystemKey::F3 => Self::F3,
                    m::SystemKey::F4 => Self::F4,
                    m::SystemKey::F5 => Self::F5,
                    m::SystemKey::F6 => Self::F6,
                    m::SystemKey::F7 => Self::F7,
                    m::SystemKey::F8 => Self::F8,
                    m::SystemKey::F9 => Self::F9,
                    m::SystemKey::F10 => Self::F10,
                    m::SystemKey::F11 => Self::F11,
                    m::SystemKey::F12 => Self::F12,
                    m::SystemKey::BrightnessDown => Self::BrightnessDown,
                    m::SystemKey::BrightnessUp => Self::BrightnessUp,
                    m::SystemKey::KeyboardIlluminationDown => Self::KeyboardIlluminationDown,
                    m::SystemKey::KeyboardIlluminationUp => Self::KeyboardIlluminationUp,
                    m::SystemKey::PreviousSong => Self::PreviousSong,
                    m::SystemKey::PlayPause => Self::PlayPause,
                    m::SystemKey::NextSong => Self::NextSong,
                    m::SystemKey::Mute => Self::Mute,
                    m::SystemKey::VolumeDown => Self::VolumeDown,
                    m::SystemKey::VolumeUp => Self::VolumeUp,
                    m::SystemKey::MicrophoneMute => Self::MicrophoneMute,
                    m::SystemKey::Search => Self::Search,
                }
            }
        }
        impl From<w::SystemKey> for m::SystemKey {
            fn from(v: w::SystemKey) -> Self {
                match v {
                    w::SystemKey::Esc => Self::Esc,
                    w::SystemKey::F1 => Self::F1,
                    w::SystemKey::F2 => Self::F2,
                    w::SystemKey::F3 => Self::F3,
                    w::SystemKey::F4 => Self::F4,
                    w::SystemKey::F5 => Self::F5,
                    w::SystemKey::F6 => Self::F6,
                    w::SystemKey::F7 => Self::F7,
                    w::SystemKey::F8 => Self::F8,
                    w::SystemKey::F9 => Self::F9,
                    w::SystemKey::F10 => Self::F10,
                    w::SystemKey::F11 => Self::F11,
                    w::SystemKey::F12 => Self::F12,
                    w::SystemKey::BrightnessDown => Self::BrightnessDown,
                    w::SystemKey::BrightnessUp => Self::BrightnessUp,
                    w::SystemKey::KeyboardIlluminationDown => Self::KeyboardIlluminationDown,
                    w::SystemKey::KeyboardIlluminationUp => Self::KeyboardIlluminationUp,
                    w::SystemKey::PreviousSong => Self::PreviousSong,
                    w::SystemKey::PlayPause => Self::PlayPause,
                    w::SystemKey::NextSong => Self::NextSong,
                    w::SystemKey::Mute => Self::Mute,
                    w::SystemKey::VolumeDown => Self::VolumeDown,
                    w::SystemKey::VolumeUp => Self::VolumeUp,
                    w::SystemKey::MicrophoneMute => Self::MicrophoneMute,
                    w::SystemKey::Search => Self::Search,
                }
            }
        }
        impl From<m::Lifetime> for w::Lifetime {
            fn from(v: m::Lifetime) -> Self {
                match v {
                    m::Lifetime::UntilHidden => Self::UntilHidden,
                    m::Lifetime::Committed => Self::Committed,
                }
            }
        }
        impl From<w::Lifetime> for m::Lifetime {
            fn from(v: w::Lifetime) -> Self {
                match v {
                    w::Lifetime::UntilHidden => Self::UntilHidden,
                    w::Lifetime::Committed => Self::Committed,
                }
            }
        }
        impl From<m::PlaybackStatus> for w::PlaybackStatus {
            fn from(v: m::PlaybackStatus) -> Self {
                match v {
                    m::PlaybackStatus::Playing => Self::Playing,
                    m::PlaybackStatus::Paused => Self::Paused,
                    m::PlaybackStatus::Stopped => Self::Stopped,
                }
            }
        }
        impl From<w::PlaybackStatus> for m::PlaybackStatus {
            fn from(v: w::PlaybackStatus) -> Self {
                match v {
                    w::PlaybackStatus::Playing => Self::Playing,
                    w::PlaybackStatus::Paused => Self::Paused,
                    w::PlaybackStatus::Stopped => Self::Stopped,
                }
            }
        }
        impl From<m::Placement> for w::Placement {
            fn from(v: m::Placement) -> Self {
                match v {
                    m::Placement::Anchored => Self::Anchored,
                    m::Placement::Region => Self::Region,
                    m::Placement::Full => Self::Full,
                }
            }
        }
        impl From<w::Placement> for m::Placement {
            fn from(v: w::Placement) -> Self {
                match v {
                    w::Placement::Anchored => Self::Anchored,
                    w::Placement::Region => Self::Region,
                    w::Placement::Full => Self::Full,
                }
            }
        }
        impl From<m::PresentationLifecycle> for w::Lifecycle {
            fn from(v: m::PresentationLifecycle) -> Self {
                match v {
                    m::PresentationLifecycle::Transient => Self::Transient,
                    m::PresentationLifecycle::Persistent => Self::Persistent,
                }
            }
        }
        impl From<w::Lifecycle> for m::PresentationLifecycle {
            fn from(v: w::Lifecycle) -> Self {
                match v {
                    w::Lifecycle::Transient => Self::Transient,
                    w::Lifecycle::Persistent => Self::Persistent,
                }
            }
        }
        impl From<m::Point> for w::Point {
            fn from(v: m::Point) -> Self {
                Self { x: v.x, y: v.y }
            }
        }
        impl From<w::Point> for m::Point {
            fn from(v: w::Point) -> Self {
                Self { x: v.x, y: v.y }
            }
        }
        impl From<m::Rect> for w::Rect {
            fn from(v: m::Rect) -> Self {
                Self {
                    x: v.x,
                    y: v.y,
                    width: v.width,
                    height: v.height,
                }
            }
        }
        impl From<w::Rect> for m::Rect {
            fn from(v: w::Rect) -> Self {
                Self {
                    x: v.x,
                    y: v.y,
                    width: v.width,
                    height: v.height,
                }
            }
        }
        impl From<m::CanvasPaint> for w::CanvasPaint {
            fn from(v: m::CanvasPaint) -> Self {
                Self {
                    color: v.color.into(),
                    opacity: v.opacity,
                }
            }
        }
        impl From<w::CanvasPaint> for m::CanvasPaint {
            fn from(v: w::CanvasPaint) -> Self {
                Self {
                    color: v.color.into(),
                    opacity: v.opacity,
                }
            }
        }
        impl From<m::CanvasLinearGradient> for w::CanvasLinearGradient {
            fn from(v: m::CanvasLinearGradient) -> Self {
                Self {
                    start: v.start.into(),
                    end: v.end.into(),
                    start_color: v.start_color.into(),
                    end_color: v.end_color.into(),
                }
            }
        }
        impl From<w::CanvasLinearGradient> for m::CanvasLinearGradient {
            fn from(v: w::CanvasLinearGradient) -> Self {
                Self {
                    start: v.start.into(),
                    end: v.end.into(),
                    start_color: v.start_color.into(),
                    end_color: v.end_color.into(),
                }
            }
        }
        impl From<m::StateField> for w::Field {
            fn from(v: m::StateField) -> Self {
                Self {
                    name: v.name,
                    kind: v.kind.into(),
                    persist: v.persist,
                }
            }
        }
        impl From<w::Field> for m::StateField {
            fn from(v: w::Field) -> Self {
                Self {
                    name: v.name,
                    kind: v.kind.into(),
                    persist: v.persist,
                }
            }
        }
        impl From<m::ActionDescription> for w::ActionDescription {
            fn from(v: m::ActionDescription) -> Self {
                Self {
                    kinds: v.kinds.into_iter().map(Into::into).collect(),
                    value: v.value.map(Into::into),
                }
            }
        }
        impl From<w::ActionDescription> for m::ActionDescription {
            fn from(v: w::ActionDescription) -> Self {
                Self {
                    kinds: v.kinds.into_iter().map(Into::into).collect(),
                    value: v.value.map(Into::into),
                }
            }
        }
        impl From<m::ActionEvent> for w::ActionEvent {
            fn from(v: m::ActionEvent) -> Self {
                Self {
                    action: v.action,
                    kind: v.kind.into(),
                    value: v.value.map(Into::into),
                    record_key: v.record_key,
                    origin: v.origin.into(),
                    cancel_reason: v.cancel_reason.map(Into::into),
                }
            }
        }
        impl From<w::ActionEvent> for m::ActionEvent {
            fn from(v: w::ActionEvent) -> Self {
                Self {
                    action: v.action,
                    kind: v.kind.into(),
                    value: v.value.map(Into::into),
                    record_key: v.record_key,
                    origin: v.origin.into(),
                    cancel_reason: v.cancel_reason.map(Into::into),
                }
            }
        }
        impl From<m::LogicContext> for w::Context {
            fn from(v: m::LogicContext) -> Self {
                Self {
                    surface_kind: v.surface_kind,
                    slot_width: v.slot_width,
                    representation: v.representation,
                    color_scheme: v.color_scheme,
                    accent: v.accent,
                    locale: v.locale,
                }
            }
        }
        impl From<w::Context> for m::LogicContext {
            fn from(v: w::Context) -> Self {
                Self {
                    surface_kind: v.surface_kind,
                    slot_width: v.slot_width,
                    representation: v.representation,
                    color_scheme: v.color_scheme,
                    accent: v.accent,
                    locale: v.locale,
                }
            }
        }
        impl From<m::DbusValue> for w::DbusValue {
            fn from(v: m::DbusValue) -> Self {
                Self {
                    signature: v.signature,
                    json: v.json,
                }
            }
        }
        impl From<w::DbusValue> for m::DbusValue {
            fn from(v: w::DbusValue) -> Self {
                Self {
                    signature: v.signature,
                    json: v.json,
                }
            }
        }
        impl From<m::DbusCall> for w::DbusCall {
            fn from(v: m::DbusCall) -> Self {
                Self {
                    bus: v.bus.into(),
                    destination: v.destination,
                    path: v.path,
                    interface: v.interface,
                    method: v.method,
                    args: v.args.into(),
                }
            }
        }
        impl From<w::DbusCall> for m::DbusCall {
            fn from(v: w::DbusCall) -> Self {
                Self {
                    bus: v.bus.into(),
                    destination: v.destination,
                    path: v.path,
                    interface: v.interface,
                    method: v.method,
                    args: v.args.into(),
                }
            }
        }
        impl From<m::DbusMatch> for w::DbusMatch {
            fn from(v: m::DbusMatch) -> Self {
                Self {
                    bus: v.bus.into(),
                    sender: v.sender,
                    path: v.path,
                    interface: v.interface,
                    member: v.member,
                }
            }
        }
        impl From<w::DbusMatch> for m::DbusMatch {
            fn from(v: w::DbusMatch) -> Self {
                Self {
                    bus: v.bus.into(),
                    sender: v.sender,
                    path: v.path,
                    interface: v.interface,
                    member: v.member,
                }
            }
        }
        impl From<m::DbusProperty> for w::DbusProperty {
            fn from(v: m::DbusProperty) -> Self {
                Self {
                    bus: v.bus.into(),
                    destination: v.destination,
                    path: v.path,
                    interface: v.interface,
                    property: v.property,
                }
            }
        }
        impl From<w::DbusProperty> for m::DbusProperty {
            fn from(v: w::DbusProperty) -> Self {
                Self {
                    bus: v.bus.into(),
                    destination: v.destination,
                    path: v.path,
                    interface: v.interface,
                    property: v.property,
                }
            }
        }
        impl From<m::SpawnResult> for w::SpawnResult {
            fn from(v: m::SpawnResult) -> Self {
                Self {
                    status: v.status,
                    stdout: v.stdout,
                    stderr: v.stderr,
                    truncated: v.truncated,
                }
            }
        }
        impl From<w::SpawnResult> for m::SpawnResult {
            fn from(v: w::SpawnResult) -> Self {
                Self {
                    status: v.status,
                    stdout: v.stdout,
                    stderr: v.stderr,
                    truncated: v.truncated,
                }
            }
        }
        impl From<m::AudioState> for w::AudioState {
            fn from(v: m::AudioState) -> Self {
                Self {
                    available: v.available,
                    volume: v.volume,
                    muted: v.muted,
                    name: v.name,
                }
            }
        }
        impl From<w::AudioState> for m::AudioState {
            fn from(v: w::AudioState) -> Self {
                Self {
                    available: v.available,
                    volume: v.volume,
                    muted: v.muted,
                    name: v.name,
                }
            }
        }
        impl From<m::MediaState> for w::MediaState {
            fn from(v: m::MediaState) -> Self {
                Self {
                    player: v.player,
                    status: v.status.into(),
                    title: v.title,
                    artist: v.artist,
                    album: v.album,
                    art_uri: v.art_uri,
                    position_ms: v.position_ms,
                    length_ms: v.length_ms,
                    can_play: v.can_play,
                    can_pause: v.can_pause,
                    can_next: v.can_next,
                    can_previous: v.can_previous,
                    can_seek: v.can_seek,
                }
            }
        }
        impl From<w::MediaState> for m::MediaState {
            fn from(v: w::MediaState) -> Self {
                Self {
                    player: v.player,
                    status: v.status.into(),
                    title: v.title,
                    artist: v.artist,
                    album: v.album,
                    art_uri: v.art_uri,
                    position_ms: v.position_ms,
                    length_ms: v.length_ms,
                    can_play: v.can_play,
                    can_pause: v.can_pause,
                    can_next: v.can_next,
                    can_previous: v.can_previous,
                    can_seek: v.can_seek,
                }
            }
        }
        impl From<m::Scalar> for w::Scalar {
            fn from(v: m::Scalar) -> Self {
                match v {
                    m::Scalar::Boolean(v) => Self::Boolean(v),
                    m::Scalar::Number(v) => Self::Number(v),
                    m::Scalar::Text(v) => Self::Text(v),
                }
            }
        }
        impl From<w::Scalar> for m::Scalar {
            fn from(v: w::Scalar) -> Self {
                match v {
                    w::Scalar::Boolean(v) => Self::Boolean(v),
                    w::Scalar::Number(v) => Self::Number(v),
                    w::Scalar::Text(v) => Self::Text(v),
                }
            }
        }
        impl From<m::FieldKind> for w::FieldKind {
            fn from(v: m::FieldKind) -> Self {
                match v {
                    m::FieldKind::Scalar(v) => Self::Scalar(v.into()),
                    m::FieldKind::Records(v) => Self::Records(
                        v.into_iter()
                            .map(|(key, value)| (key, value.into()))
                            .collect(),
                    ),
                    m::FieldKind::Canvas => Self::Canvas,
                }
            }
        }
        impl From<w::FieldKind> for m::FieldKind {
            fn from(v: w::FieldKind) -> Self {
                match v {
                    w::FieldKind::Scalar(v) => Self::Scalar(v.into()),
                    w::FieldKind::Records(v) => Self::Records(
                        v.into_iter()
                            .map(|(key, value)| (key, value.into()))
                            .collect(),
                    ),
                    w::FieldKind::Canvas => Self::Canvas,
                }
            }
        }
        impl From<m::EffectError> for w::EffectError {
            fn from(v: m::EffectError) -> Self {
                match v {
                    m::EffectError::Unsupported => Self::Unsupported,
                    m::EffectError::Unavailable => Self::Unavailable,
                    m::EffectError::Failed(v) => Self::Failed(v),
                    m::EffectError::Timeout => Self::Timeout,
                    m::EffectError::Cancelled => Self::Cancelled,
                }
            }
        }
        impl From<w::EffectError> for m::EffectError {
            fn from(v: w::EffectError) -> Self {
                match v {
                    w::EffectError::Unsupported => Self::Unsupported,
                    w::EffectError::Unavailable => Self::Unavailable,
                    w::EffectError::Failed(v) => Self::Failed(v),
                    w::EffectError::Timeout => Self::Timeout,
                    w::EffectError::Cancelled => Self::Cancelled,
                }
            }
        }
        impl From<m::EffectResult> for w::EffectResult {
            fn from(v: m::EffectResult) -> Self {
                match v {
                    m::EffectResult::None => Self::None,
                    m::EffectResult::Mute(v) => Self::Mute(v),
                    m::EffectResult::Spawn(v) => Self::Spawn(v.into()),
                    m::EffectResult::Dbus(v) => Self::Dbus(v.into()),
                }
            }
        }
        impl From<w::EffectResult> for m::EffectResult {
            fn from(v: w::EffectResult) -> Self {
                match v {
                    w::EffectResult::None => Self::None,
                    w::EffectResult::Mute(v) => Self::Mute(v),
                    w::EffectResult::Spawn(v) => Self::Spawn(v.into()),
                    w::EffectResult::Dbus(v) => Self::Dbus(v.into()),
                }
            }
        }
        impl From<m::CanvasColor> for w::CanvasColor {
            fn from(v: m::CanvasColor) -> Self {
                match v {
                    m::CanvasColor::Role(v) => Self::Role(v.into()),
                    m::CanvasColor::Rgba {
                        red,
                        green,
                        blue,
                        alpha,
                    } => Self::Rgba(w::Rgba {
                        red,
                        green,
                        blue,
                        alpha,
                    }),
                }
            }
        }
        impl From<w::CanvasColor> for m::CanvasColor {
            fn from(v: w::CanvasColor) -> Self {
                match v {
                    w::CanvasColor::Role(v) => Self::Role(v.into()),
                    w::CanvasColor::Rgba(v) => Self::Rgba {
                        red: v.red,
                        green: v.green,
                        blue: v.blue,
                        alpha: v.alpha,
                    },
                }
            }
        }
        impl From<$crate::CallContext> for w::CallContext {
            fn from(v: $crate::CallContext) -> Self {
                Self { now_ms: v.now_ms }
            }
        }
        impl From<w::CallContext> for $crate::CallContext {
            fn from(v: w::CallContext) -> Self {
                Self { now_ms: v.now_ms }
            }
        }

        impl From<m::CanvasCommand> for w::CanvasCommand {
            fn from(v: m::CanvasCommand) -> Self {
                match v {
                    m::CanvasCommand::FillRect {
                        rect,
                        radius,
                        paint,
                    } => Self::FillRect(w::CanvasRect {
                        rect: rect.into(),
                        radius: radius,
                        paint: paint.into(),
                    }),
                    m::CanvasCommand::Line {
                        start,
                        end,
                        width,
                        paint,
                    } => Self::Line(w::CanvasLine {
                        start: start.into(),
                        end: end.into(),
                        width: width,
                        paint: paint.into(),
                    }),
                    m::CanvasCommand::Polyline {
                        points,
                        width,
                        paint,
                    } => Self::Polyline(w::CanvasPolyline {
                        points: points.into_iter().map(Into::into).collect(),
                        width: width,
                        paint: paint.into(),
                    }),
                    m::CanvasCommand::FillCircle {
                        center,
                        radius,
                        paint,
                    } => Self::FillCircle(w::CanvasCircle {
                        center: center.into(),
                        radius: radius,
                        paint: paint.into(),
                    }),
                    m::CanvasCommand::Text {
                        rect,
                        text,
                        size,
                        paint,
                        align,
                    } => Self::Text(w::CanvasText {
                        rect: rect.into(),
                        text: text,
                        size: size,
                        paint: paint.into(),
                        align: align.into(),
                    }),
                    m::CanvasCommand::FillLinearGradientRect {
                        rect,
                        radius,
                        gradient,
                    } => Self::FillLinearGradientRect(w::CanvasGradientRect {
                        rect: rect.into(),
                        radius: radius,
                        gradient: gradient.into(),
                    }),
                    m::CanvasCommand::StrokePath {
                        segments,
                        width,
                        paint,
                    } => Self::StrokePath(w::CanvasPath {
                        segments: segments.into_iter().map(Into::into).collect(),
                        width: width,
                        paint: paint.into(),
                    }),
                    m::CanvasCommand::FillPath { segments, paint } => {
                        Self::FillPath(w::CanvasFillPath {
                            segments: segments.into_iter().map(Into::into).collect(),
                            paint: paint.into(),
                        })
                    }
                }
            }
        }
        impl From<w::CanvasCommand> for m::CanvasCommand {
            fn from(v: w::CanvasCommand) -> Self {
                match v {
                    w::CanvasCommand::FillRect(v) => Self::FillRect {
                        rect: v.rect.into(),
                        radius: v.radius,
                        paint: v.paint.into(),
                    },
                    w::CanvasCommand::Line(v) => Self::Line {
                        start: v.start.into(),
                        end: v.end.into(),
                        width: v.width,
                        paint: v.paint.into(),
                    },
                    w::CanvasCommand::Polyline(v) => Self::Polyline {
                        points: v.points.into_iter().map(Into::into).collect(),
                        width: v.width,
                        paint: v.paint.into(),
                    },
                    w::CanvasCommand::FillCircle(v) => Self::FillCircle {
                        center: v.center.into(),
                        radius: v.radius,
                        paint: v.paint.into(),
                    },
                    w::CanvasCommand::Text(v) => Self::Text {
                        rect: v.rect.into(),
                        text: v.text,
                        size: v.size,
                        paint: v.paint.into(),
                        align: v.align.into(),
                    },
                    w::CanvasCommand::FillLinearGradientRect(v) => Self::FillLinearGradientRect {
                        rect: v.rect.into(),
                        radius: v.radius,
                        gradient: v.gradient.into(),
                    },
                    w::CanvasCommand::StrokePath(v) => Self::StrokePath {
                        segments: v.segments.into_iter().map(Into::into).collect(),
                        width: v.width,
                        paint: v.paint.into(),
                    },
                    w::CanvasCommand::FillPath(v) => Self::FillPath {
                        segments: v.segments.into_iter().map(Into::into).collect(),
                        paint: v.paint.into(),
                    },
                }
            }
        }
        impl From<m::CanvasPathSegment> for w::CanvasPathSegment {
            fn from(v: m::CanvasPathSegment) -> Self {
                match v {
                    m::CanvasPathSegment::QuadraticTo { control, endpoint } => {
                        Self::QuadraticTo(w::CanvasQuadraticSegment {
                            control: control.into(),
                            endpoint: endpoint.into(),
                        })
                    }
                    m::CanvasPathSegment::CubicTo {
                        control_one,
                        control_two,
                        endpoint,
                    } => Self::CubicTo(w::CanvasCubicSegment {
                        control_one: control_one.into(),
                        control_two: control_two.into(),
                        endpoint: endpoint.into(),
                    }),
                    m::CanvasPathSegment::MoveTo(v) => Self::MoveTo(v.into()),
                    m::CanvasPathSegment::LineTo(v) => Self::LineTo(v.into()),
                    m::CanvasPathSegment::Close => Self::Close,
                }
            }
        }
        impl From<w::CanvasPathSegment> for m::CanvasPathSegment {
            fn from(v: w::CanvasPathSegment) -> Self {
                match v {
                    w::CanvasPathSegment::QuadraticTo(v) => Self::QuadraticTo {
                        control: v.control.into(),
                        endpoint: v.endpoint.into(),
                    },
                    w::CanvasPathSegment::CubicTo(v) => Self::CubicTo {
                        control_one: v.control_one.into(),
                        control_two: v.control_two.into(),
                        endpoint: v.endpoint.into(),
                    },
                    w::CanvasPathSegment::MoveTo(v) => Self::MoveTo(v.into()),
                    w::CanvasPathSegment::LineTo(v) => Self::LineTo(v.into()),
                    w::CanvasPathSegment::Close => Self::Close,
                }
            }
        }
        fn unique_map<V>(
            values: Vec<(String, V)>,
        ) -> Result<std::collections::BTreeMap<String, V>, String> {
            let mut out = std::collections::BTreeMap::new();
            for (key, value) in values {
                if out.insert(key.clone(), value).is_some() {
                    return Err(format!("duplicate record/map key: {key}"));
                }
            }
            Ok(out)
        }
        fn record_from(v: w::RecordValue) -> Result<m::RecordValue, String> {
            unique_map(
                v.into_iter()
                    .map(|(key, value)| (key, value.into()))
                    .collect(),
            )
        }
        impl From<m::SpawnSpec> for w::SpawnSpec {
            fn from(v: m::SpawnSpec) -> Self {
                Self {
                    argv: v.argv,
                    env: v.env.into_iter().collect(),
                    stdin: v.stdin,
                    max_output: v.max_output,
                }
            }
        }
        impl TryFrom<w::SpawnSpec> for m::SpawnSpec {
            type Error = String;
            fn try_from(v: w::SpawnSpec) -> Result<Self, String> {
                Ok(Self {
                    argv: v.argv,
                    env: unique_map(v.env)?,
                    stdin: v.stdin,
                    max_output: v.max_output,
                })
            }
        }

        impl From<m::Effect> for w::Effect {
            fn from(v: m::Effect) -> Self {
                match v {
                    m::Effect::AudioSetVolume { target, level } => {
                        Self::AudioSetVolume(w::AudioVolume {
                            target: target.into(),
                            level: level,
                        })
                    }
                    m::Effect::AudioSetMute { target, value } => Self::AudioSetMute(w::AudioMute {
                        target: target.into(),
                        value: value.into(),
                    }),
                    m::Effect::MediaPlayPause => Self::MediaPlayPause,
                    m::Effect::MediaNext => Self::MediaNext,
                    m::Effect::MediaPrevious => Self::MediaPrevious,
                    m::Effect::MediaSeek { position_ms } => Self::MediaSeek(position_ms),
                    m::Effect::DesktopShortcut { keys, app } => {
                        Self::DesktopShortcut(w::DesktopShortcut {
                            keys: keys,
                            app: app,
                        })
                    }
                    m::Effect::DesktopBrightness { level } => Self::DesktopBrightness(level),
                    m::Effect::DesktopWorkspace { id } => Self::DesktopWorkspace(id),
                    m::Effect::DesktopAppLaunch { id } => Self::DesktopAppLaunch(id),
                    m::Effect::KeyTap { key } => Self::KeyTap(key.into()),
                    m::Effect::KeyDown { key } => Self::KeyDown(key.into()),
                    m::Effect::KeyUp { key } => Self::KeyUp(key.into()),
                    m::Effect::OpenUri { uri } => Self::OpenUri(uri),
                    m::Effect::TimerAfter { ms } => Self::TimerAfter(ms),
                    m::Effect::SpawnRun(v) => Self::SpawnRun(v.into()),
                    m::Effect::DbusCall(v) => Self::DbusCall(v.into()),
                }
            }
        }
        impl TryFrom<w::Effect> for m::Effect {
            type Error = String;
            fn try_from(v: w::Effect) -> Result<Self, String> {
                Ok(match v {
                    w::Effect::AudioSetVolume(v) => Self::AudioSetVolume {
                        target: v.target.into(),
                        level: v.level,
                    },
                    w::Effect::AudioSetMute(v) => Self::AudioSetMute {
                        target: v.target.into(),
                        value: v.value.into(),
                    },
                    w::Effect::MediaPlayPause => Self::MediaPlayPause,
                    w::Effect::MediaNext => Self::MediaNext,
                    w::Effect::MediaPrevious => Self::MediaPrevious,
                    w::Effect::MediaSeek(v) => Self::MediaSeek { position_ms: v },
                    w::Effect::DesktopShortcut(v) => Self::DesktopShortcut {
                        keys: v.keys,
                        app: v.app,
                    },
                    w::Effect::DesktopBrightness(v) => Self::DesktopBrightness { level: v },
                    w::Effect::DesktopWorkspace(v) => Self::DesktopWorkspace { id: v },
                    w::Effect::DesktopAppLaunch(v) => Self::DesktopAppLaunch { id: v },
                    w::Effect::KeyTap(v) => Self::KeyTap { key: v.into() },
                    w::Effect::KeyDown(v) => Self::KeyDown { key: v.into() },
                    w::Effect::KeyUp(v) => Self::KeyUp { key: v.into() },
                    w::Effect::OpenUri(v) => Self::OpenUri { uri: v },
                    w::Effect::TimerAfter(v) => Self::TimerAfter { ms: v },
                    w::Effect::SpawnRun(v) => Self::SpawnRun(v.try_into()?),
                    w::Effect::DbusCall(v) => Self::DbusCall(v.into()),
                })
            }
        }
        impl From<m::EffectRequest> for w::Request {
            fn from(v: m::EffectRequest) -> Self {
                Self {
                    tag: v.tag,
                    effect: v.effect.into(),
                    lifetime: v.lifetime.map(Into::into),
                    coalesce: v.coalesce,
                    timeout_ms: v.timeout_ms,
                }
            }
        }
        impl TryFrom<w::Request> for m::EffectRequest {
            type Error = String;
            fn try_from(v: w::Request) -> Result<Self, String> {
                Ok(Self {
                    tag: v.tag,
                    effect: v.effect.try_into()?,
                    lifetime: v.lifetime.map(Into::into),
                    coalesce: v.coalesce,
                    timeout_ms: v.timeout_ms,
                })
            }
        }
        impl From<m::SubscriptionKind> for w::SubscriptionKind {
            fn from(v: m::SubscriptionKind) -> Self {
                match v {
                    m::SubscriptionKind::AudioWatch { target } => Self::AudioWatch(target.into()),
                    m::SubscriptionKind::MediaWatch => Self::MediaWatch,
                    m::SubscriptionKind::DesktopFact { name } => Self::DesktopFact(name.0),
                    m::SubscriptionKind::TimerInterval { ms } => Self::TimerInterval(ms),
                    m::SubscriptionKind::SpawnLines(v) => Self::SpawnLines(v.into()),
                    m::SubscriptionKind::DbusSignal(v) => Self::DbusSignal(v.into()),
                    m::SubscriptionKind::DbusProperty(v) => Self::DbusProperty(v.into()),
                }
            }
        }
        impl TryFrom<w::SubscriptionKind> for m::SubscriptionKind {
            type Error = String;
            fn try_from(v: w::SubscriptionKind) -> Result<Self, String> {
                Ok(match v {
                    w::SubscriptionKind::AudioWatch(v) => Self::AudioWatch { target: v.into() },
                    w::SubscriptionKind::MediaWatch => Self::MediaWatch,
                    w::SubscriptionKind::DesktopFact(v) => Self::DesktopFact { name: v.into() },
                    w::SubscriptionKind::TimerInterval(v) => Self::TimerInterval { ms: v },
                    w::SubscriptionKind::SpawnLines(v) => Self::SpawnLines(v.try_into()?),
                    w::SubscriptionKind::DbusSignal(v) => Self::DbusSignal(v.into()),
                    w::SubscriptionKind::DbusProperty(v) => Self::DbusProperty(v.into()),
                })
            }
        }
        impl From<m::SubscriptionSpec> for w::Subscription {
            fn from(v: m::SubscriptionSpec) -> Self {
                Self {
                    tag: v.tag,
                    kind: v.kind.into(),
                    background: v.background,
                }
            }
        }
        impl TryFrom<w::Subscription> for m::SubscriptionSpec {
            type Error = String;
            fn try_from(v: w::Subscription) -> Result<Self, String> {
                Ok(Self {
                    tag: v.tag,
                    kind: v.kind.try_into()?,
                    background: v.background,
                })
            }
        }
        impl From<m::FactValue> for w::FactValue {
            fn from(v: m::FactValue) -> Self {
                match v {
                    m::FactValue::Boolean(v) => Self::Boolean(v),
                    m::FactValue::Number(v) => Self::Number(v),
                    m::FactValue::Text(v) => Self::Text(v),
                    m::FactValue::List(v) => Self::List(v),
                    m::FactValue::Records(v) => Self::Records(
                        v.into_iter()
                            .map(|r| {
                                r.into_iter()
                                    .map(|(key, value)| (key, value.into()))
                                    .collect()
                            })
                            .collect(),
                    ),
                }
            }
        }
        impl TryFrom<w::FactValue> for m::FactValue {
            type Error = String;
            fn try_from(v: w::FactValue) -> Result<Self, String> {
                Ok(match v {
                    w::FactValue::Boolean(v) => Self::Boolean(v),
                    w::FactValue::Number(v) => Self::Number(v),
                    w::FactValue::Text(v) => Self::Text(v),
                    w::FactValue::List(v) => Self::List(v),
                    w::FactValue::Records(v) => {
                        Self::Records(v.into_iter().map(record_from).collect::<Result<_, _>>()?)
                    }
                })
            }
        }
        impl From<m::SubscriptionData> for w::SubscriptionData {
            fn from(v: m::SubscriptionData) -> Self {
                match v {
                    m::SubscriptionData::Audio(v) => Self::Audio(v.into()),
                    m::SubscriptionData::Media(v) => Self::Media(v.into()),
                    m::SubscriptionData::Fact(v) => Self::Fact(v.map(Into::into)),
                    m::SubscriptionData::Tick => Self::Tick,
                    m::SubscriptionData::Line(v) => Self::Line(v),
                    m::SubscriptionData::Dbus(v) => Self::Dbus(v.into()),
                }
            }
        }
        impl TryFrom<w::SubscriptionData> for m::SubscriptionData {
            type Error = String;
            fn try_from(v: w::SubscriptionData) -> Result<Self, String> {
                Ok(match v {
                    w::SubscriptionData::Audio(v) => Self::Audio(v.into()),
                    w::SubscriptionData::Media(v) => Self::Media(v.into()),
                    w::SubscriptionData::Fact(v) => {
                        Self::Fact(v.map(TryInto::try_into).transpose()?)
                    }
                    w::SubscriptionData::Tick => Self::Tick,
                    w::SubscriptionData::Line(v) => Self::Line(v),
                    w::SubscriptionData::Dbus(v) => Self::Dbus(v.into()),
                })
            }
        }
        pub fn description_to(v: m::LogicDescription) -> w::Description {
            w::Description {
                actions: v
                    .actions
                    .into_iter()
                    .map(|(key, value)| (key, value.into()))
                    .collect(),
                state: v.state.into_iter().map(Into::into).collect(),
            }
        }
        pub fn description_from(v: w::Description) -> m::LogicDescription {
            m::LogicDescription {
                actions: v
                    .actions
                    .into_iter()
                    .map(|(key, value)| (key, value.into()))
                    .collect(),
                state: v.state.into_iter().map(Into::into).collect(),
            }
        }
        pub fn state_to(v: m::StatePatch) -> w::StatePatch {
            v.into_iter()
                .map(|(key, value)| {
                    (
                        key,
                        match value {
                            m::StateValue::Scalar(v) => w::Value::Scalar(v.into()),
                            m::StateValue::Records(v) => w::Value::Records(
                                v.into_iter()
                                    .map(|r| {
                                        r.into_iter()
                                            .map(|(key, value)| (key, value.into()))
                                            .collect()
                                    })
                                    .collect(),
                            ),
                            m::StateValue::Canvas(v) => {
                                w::Value::Canvas(v.into_iter().map(Into::into).collect())
                            }
                        },
                    )
                })
                .collect()
        }
        pub fn state_from(v: w::StatePatch) -> $crate::Translation<m::StatePatch> {
            let mut warnings = vec![];
            let value = v
                .into_iter()
                .map(|(key, value)| {
                    (
                        key,
                        match value {
                            w::Value::Scalar(v) => m::StateValue::Scalar(v.into()),
                            w::Value::Records(v) => m::StateValue::Records(
                                v.into_iter()
                                    .filter_map(|r| match record_from(r) {
                                        Ok(r) => Some(r),
                                        Err(e) => {
                                            warnings.push(e);
                                            None
                                        }
                                    })
                                    .collect(),
                            ),
                            w::Value::Canvas(v) => {
                                m::StateValue::Canvas(v.into_iter().map(Into::into).collect())
                            }
                        },
                    )
                })
                .collect();
            $crate::Translation { value, warnings }
        }
        pub fn update_to(v: m::LogicUpdate) -> w::Update {
            w::Update {
                state: state_to(v.state),
                requests: v.requests.into_iter().map(Into::into).collect(),
                cancel: v.cancel,
                subscriptions: v
                    .subscriptions
                    .map(|v| v.into_iter().map(Into::into).collect()),
                presentation: v.presentation.map(|v| match v {
                    m::PresentationCommand::Close => w::Command::Close,
                    m::PresentationCommand::Open {
                        panel,
                        placement,
                        lifecycle,
                    } => w::Command::Open(w::OpenPanel {
                        panel: panel.into(),
                        placement: placement.into(),
                        lifecycle: lifecycle.into(),
                    }),
                }),
            }
        }
        pub fn update_from(v: w::Update) -> $crate::Translation<m::LogicUpdate> {
            let state = state_from(v.state);
            let mut warnings = state.warnings;
            let requests = v
                .requests
                .into_iter()
                .filter_map(|r| match r.try_into() {
                    Ok(r) => Some(r),
                    Err(e) => {
                        warnings.push(e);
                        None
                    }
                })
                .collect();
            let subscriptions = v.subscriptions.map(|v| {
                v.into_iter()
                    .filter_map(|s| match s.try_into() {
                        Ok(s) => Some(s),
                        Err(e) => {
                            warnings.push(e);
                            None
                        }
                    })
                    .collect()
            });
            let presentation = v.presentation.and_then(|v| match v {
                w::Command::Close => Some(m::PresentationCommand::Close),
                w::Command::Open(v) => match v.panel.parse() {
                    Ok(panel) => Some(m::PresentationCommand::Open {
                        panel,
                        placement: v.placement.into(),
                        lifecycle: v.lifecycle.into(),
                    }),
                    Err(e) => {
                        warnings.push(format!("{e}"));
                        None
                    }
                },
            });
            $crate::Translation {
                value: m::LogicUpdate {
                    state: state.value,
                    requests,
                    cancel: v.cancel,
                    subscriptions,
                    presentation,
                },
                warnings,
            }
        }
        pub fn event_to(v: m::LogicEvent) -> w::Event {
            match v {
                m::LogicEvent::Action(v) => w::Event::Action(v.into()),
                m::LogicEvent::Shown => w::Event::Shown,
                m::LogicEvent::Hidden => w::Event::Hidden,
                m::LogicEvent::ContextChanged(v) => w::Event::ContextChanged(v.into()),
                m::LogicEvent::PresentationClosed(v) => w::Event::PresentationClosed(v.into()),
                m::LogicEvent::EffectDone { tag, result } => w::Event::EffectDone(w::EffectDone {
                    tag,
                    result: result.map(Into::into).map_err(Into::into),
                }),
                m::LogicEvent::SubscriptionData { tag, data } => {
                    w::Event::SubscriptionData(w::SubscriptionMessage {
                        tag,
                        data: data.into(),
                    })
                }
                m::LogicEvent::SubscriptionLost { tag, error } => {
                    w::Event::SubscriptionLost(w::SubscriptionLost {
                        tag,
                        error: error.into(),
                    })
                }
            }
        }
        pub fn event_from(v: w::Event) -> Result<m::LogicEvent, String> {
            Ok(match v {
                w::Event::Action(v) => m::LogicEvent::Action(v.into()),
                w::Event::Shown => m::LogicEvent::Shown,
                w::Event::Hidden => m::LogicEvent::Hidden,
                w::Event::ContextChanged(v) => m::LogicEvent::ContextChanged(v.into()),
                w::Event::PresentationClosed(v) => m::LogicEvent::PresentationClosed(
                    v.parse().map_err(|e: m::ModelError| e.to_string())?,
                ),
                w::Event::EffectDone(v) => m::LogicEvent::EffectDone {
                    tag: v.tag,
                    result: v.result.map(Into::into).map_err(Into::into),
                },
                w::Event::SubscriptionData(v) => m::LogicEvent::SubscriptionData {
                    tag: v.tag,
                    data: v.data.try_into()?,
                },
                w::Event::SubscriptionLost(v) => m::LogicEvent::SubscriptionLost {
                    tag: v.tag,
                    error: v.error.into(),
                },
            })
        }
    };
}
