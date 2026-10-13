//! Audio and brightness controls. The host owns effects and subscriptions.
use touchbar_logic_sdk::{CallContext, Description, Logic, Request, Update};
use touchbar_model::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Config {
    Volume,
    Microphone,
    Brightness,
}
#[derive(Default)]
struct Level {
    available: bool,
    system: f64,
    muted: bool,
    drag: Option<f64>,
    unmute_sent: bool,
}
impl Level {
    fn unavailable(&mut self) {
        self.available = false;
        self.drag = None;
        self.unmute_sent = false;
    }
    fn shown(&self) -> f64 {
        self.drag.unwrap_or(self.system)
    }
}
pub struct Controls {
    config: Config,
    level: Level,
    microphone_available: bool,
    microphone_system: bool,
    microphone_shown: bool,
    microphone_pending: Option<String>,
    sequence: u64,
}
impl Controls {
    fn tag(&mut self, name: &str) -> String {
        self.sequence += 1;
        format!("{name}/{}", self.sequence)
    }
    fn subscriptions(&self) -> Vec<SubscriptionSpec> {
        let watch = |tag: &str, target| SubscriptionSpec {
            tag: tag.into(),
            kind: SubscriptionKind::AudioWatch { target },
            background: false,
        };
        match self.config {
            Config::Volume => vec![
                watch("sink", AudioTarget::DefaultSink),
                watch("source", AudioTarget::DefaultSource),
            ],
            Config::Microphone => vec![watch("source", AudioTarget::DefaultSource)],
            Config::Brightness => vec![SubscriptionSpec {
                tag: "brightness".into(),
                kind: SubscriptionKind::DesktopFact {
                    name: "display.brightness".into(),
                },
                background: false,
            }],
        }
    }
    fn state(&self) -> Update {
        let available = if self.config == Config::Microphone {
            self.microphone_available
        } else {
            self.level.available
        };
        let value = self.level.shown();
        let muted = self.level.muted && !self.level.unmute_sent;
        let text = if !self.level.available {
            "–".into()
        } else if muted && self.config == Config::Volume {
            "MUTED".into()
        } else {
            format!("{:.0}%", (value * 100.0).round())
        };
        Update::new()
            .set("available", available)
            .set("volume", value.clamp(0.0, 1.0))
            .set("volume_text", text.clone())
            .set("volume_label", format!("VOL {text}"))
            .set("volume_icon", if muted { "muted" } else { "volume" })
            .set("volume_selected", muted || value == 0.0)
            .set("microphone_available", self.microphone_available)
            .set("microphone_muted", self.microphone_shown)
            .set(
                "microphone_icon",
                if self.microphone_shown {
                    "muted"
                } else {
                    "volume"
                },
            )
            .set(
                "microphone_text",
                if !self.microphone_available {
                    "–"
                } else if self.microphone_shown {
                    "MUTED"
                } else {
                    "MIC"
                },
            )
            .set("brightness", value.clamp(0.0, 1.0))
            .set("brightness_text", text.clone())
            .set("brightness_label", format!("LIGHT {text}"))
    }
    fn source_unavailable(&mut self) {
        self.microphone_available = false;
        self.microphone_pending = None;
        self.microphone_shown = self.microphone_system;
    }
    fn data(&mut self, tag: &str, data: SubscriptionData) {
        match (self.config, tag, data) {
            (Config::Volume, "sink", SubscriptionData::Audio(a)) => {
                if a.available && a.volume.is_finite() && a.volume >= 0.0 {
                    self.level.available = true;
                    self.level.system = a.volume;
                    self.level.muted = a.muted;
                } else { self.level.unavailable(); }
            }
            (Config::Volume | Config::Microphone, "source", SubscriptionData::Audio(a)) => {
                if a.available {
                    self.microphone_available = true;
                    self.microphone_system = a.muted;
                    if self.microphone_pending.is_none() { self.microphone_shown = a.muted; }
                } else { self.source_unavailable(); }
            }
            (Config::Brightness, "brightness", SubscriptionData::Fact(value)) => {
                if let Some(FactValue::Number(n)) = value.filter(|v| matches!(v, FactValue::Number(n) if n.is_finite() && (0.0..=1.0).contains(n))) {
                    self.level.available = true;
                    self.level.system = n;
                } else { self.level.unavailable(); }
            }
            _ => {}
        }
    }
    fn action(&mut self, event: ActionEvent) -> Vec<EffectRequest> {
        if event.action == "microphone"
            && event.kind == ActionKind::Activated
            && self.config != Config::Brightness
            && self.microphone_available
        {
            self.microphone_shown = !self.microphone_shown;
            let tag = self.tag("microphone");
            self.microphone_pending = Some(tag.clone());
            return vec![
                Request::new(
                    tag,
                    Effect::AudioSetMute {
                        target: AudioTarget::DefaultSource,
                        value: MuteValue::Toggle,
                    },
                )
                .committed()
                .build(),
            ];
        }
        let name = match self.config {
            Config::Volume => "volume",
            Config::Brightness => "brightness",
            Config::Microphone => return vec![],
        };
        if event.action != format!("{name}-level") {
            return vec![];
        }
        match event.kind {
            ActionKind::Released | ActionKind::Cancelled | ActionKind::Activated => {
                self.level.drag = None;
                self.level.unmute_sent = false;
            }
            ActionKind::Pressed if self.level.available => {
                self.level.drag = Some(self.level.system.clamp(0.0, 1.0));
                self.level.unmute_sent = false;
            }
            ActionKind::ValueChanged if self.level.available => {
                let Some(Scalar::Number(value)) = event.value else {
                    return vec![];
                };
                if !value.is_finite() {
                    return vec![];
                }
                let value = value.clamp(0.0, 1.0);
                self.level.drag = Some(value);
                let mut requests = vec![];
                if self.config == Config::Volume && self.level.muted && !self.level.unmute_sent {
                    self.level.unmute_sent = true;
                    let tag = self.tag("unmute");
                    requests.push(
                        Request::new(
                            tag,
                            Effect::AudioSetMute {
                                target: AudioTarget::DefaultSink,
                                value: MuteValue::Off,
                            },
                        )
                        .committed()
                        .build(),
                    );
                }
                let effect = if self.config == Config::Volume {
                    Effect::AudioSetVolume {
                        target: AudioTarget::DefaultSink,
                        level: value,
                    }
                } else {
                    Effect::DesktopBrightness { level: value }
                };
                let tag = self.tag(name);
                requests.push(Request::new(tag, effect).committed().coalesce(name).build());
                return requests;
            }
            _ => {}
        }
        vec![]
    }
    fn result(&mut self, tag: &str, result: EffectOutcome) {
        if self.microphone_pending.as_deref() == Some(tag) {
            self.microphone_pending = None;
            match result {
                Ok(EffectResult::Mute(muted)) => {
                    self.microphone_system = muted;
                    self.microphone_shown = muted;
                }
                Err(EffectError::Unsupported) => self.source_unavailable(),
                _ => self.microphone_shown = self.microphone_system,
            }
        } else if matches!(result, Err(EffectError::Unsupported))
            && ((self.config == Config::Volume
                && (tag.starts_with("volume/") || tag.starts_with("unmute/")))
                || (self.config == Config::Brightness && tag.starts_with("brightness/")))
        {
            self.level.unavailable();
        }
    }
}
impl Logic for Controls {
    fn describe() -> LogicDescription {
        let kinds = vec![
            ActionKind::Pressed,
            ActionKind::ValueChanged,
            ActionKind::Activated,
            ActionKind::Released,
            ActionKind::Cancelled,
        ];
        let mut d = Description::new()
            .action("volume-level", kinds.clone(), Some(ScalarKind::Number))
            .action("brightness-level", kinds, Some(ScalarKind::Number))
            .action("microphone", vec![ActionKind::Activated], None);
        for (name, kind) in [
            ("available", ScalarKind::Boolean),
            ("volume", ScalarKind::Number),
            ("volume_text", ScalarKind::Text),
            ("volume_label", ScalarKind::Text),
            ("volume_icon", ScalarKind::Text),
            ("volume_selected", ScalarKind::Boolean),
            ("microphone_available", ScalarKind::Boolean),
            ("microphone_muted", ScalarKind::Boolean),
            ("microphone_icon", ScalarKind::Text),
            ("microphone_text", ScalarKind::Text),
            ("brightness", ScalarKind::Number),
            ("brightness_text", ScalarKind::Text),
            ("brightness_label", ScalarKind::Text),
        ] {
            d = d.field(name, FieldKind::Scalar(kind), false);
        }
        d.build()
    }
    fn init(config: &str, _: StatePatch, _: &CallContext) -> Result<(Self, LogicUpdate), String> {
        let config = match config {
            "volume" => Config::Volume,
            "microphone" => Config::Microphone,
            "brightness" => Config::Brightness,
            _ => return Err(format!("unknown controls configuration: {config}")),
        };
        let controls = Self {
            config,
            level: Level::default(),
            microphone_available: false,
            microphone_system: false,
            microphone_shown: false,
            microphone_pending: None,
            sequence: 0,
        };
        let update = controls
            .state()
            .subscriptions(controls.subscriptions())
            .build();
        Ok((controls, update))
    }
    fn handle(&mut self, event: LogicEvent, _: &CallContext) -> Result<LogicUpdate, String> {
        let mut requests = vec![];
        match event {
            LogicEvent::Action(event) => requests = self.action(event),
            LogicEvent::SubscriptionData { tag, data } => self.data(&tag, data),
            LogicEvent::SubscriptionLost { tag, .. } => match tag.as_str() {
                "sink" if self.config == Config::Volume => self.level.unavailable(),
                "brightness" if self.config == Config::Brightness => self.level.unavailable(),
                "source" if self.config != Config::Brightness => self.source_unavailable(),
                _ => {}
            },
            LogicEvent::EffectDone { tag, result } => self.result(&tag, result),
            _ => {}
        }
        let mut update = self.state().build();
        update.requests = requests;
        Ok(update)
    }
}
