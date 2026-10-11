//! Small reusable catalogs and explicit Rust logic for engine scenarios.
use crate::*;
use touchbar_model::*;
pub fn controls_with_panel() -> Catalog {
    catalog()
        .plugin("controls")
        .set("main", |s| {
            s.slider("volume");
            let mut microphone =
                WidgetSpec::new("microphone".parse().expect("widget"), WidgetKind::Button);
            microphone
                .handlers
                .insert(ActionKind::Activated, "microphone".into());
            microphone.press_and_hold_panel = Some("devices".parse().expect("panel"));
            microphone.hold_ms = Some(100);
            s.widget(microphone);
            let mut choose = WidgetSpec::new("choose".parse().expect("widget"), WidgetKind::Button);
            choose
                .handlers
                .insert(ActionKind::Activated, "microphone".into());
            s.panel(PanelSpec {
                id: "devices".parse().expect("panel"),
                widgets: vec![choose],
                dismiss_on_selection: true,
                principal_item: None,
                dismissal: DismissalPolicy {
                    release: false,
                    selection: true,
                    ..DismissalPolicy::default()
                },
            });
        })
        .set("fkeys", |s| {
            s.key("f1", SystemKey::F1);
            s.key("f2", SystemKey::F2);
            s.static_set();
        })
        .profile("default", |p| {
            p.default().slot("main", &["controls/main"]);
        })
        .binding(Binding {
            id: "fn".into(),
            when: None,
            trigger: Trigger::Key(KeyTrigger {
                keys: vec![WatchKey::Fn],
                gesture: KeyGesture::WhileHeld,
            }),
            action: LayerAction {
                operation: LayerOperation::Layer("controls/fkeys".parse().expect("set")),
                surfaces: LayerSurfaces::Source,
            },
        })
        .build()
}
pub fn controls_logic() -> FakeLogic {
    let mut logic = FakeLogic::default();
    logic.register("controls/main", None, |_, call| {
        let mut update = LogicUpdate::default();
        if let LogicCall::Handle(LogicEvent::Action(action)) = call {
            let effect = match (action.action.as_str(), action.kind, &action.value) {
                ("volume", ActionKind::ValueChanged, Some(Scalar::Number(level))) => {
                    Some(Effect::AudioSetVolume {
                        target: AudioTarget::DefaultSink,
                        level: *level,
                    })
                }
                ("microphone", ActionKind::Activated, _) => Some(Effect::AudioSetMute {
                    target: AudioTarget::DefaultSource,
                    value: MuteValue::Toggle,
                }),
                _ => None,
            };
            if let Some(effect) = effect {
                update.requests.push(EffectRequest {
                    tag: action.action.clone(),
                    effect,
                    lifetime: None,
                    coalesce: Some(action.action.clone()),
                    timeout_ms: None,
                });
            }
        }
        LogicReaction::Update(update)
    });
    logic
}
