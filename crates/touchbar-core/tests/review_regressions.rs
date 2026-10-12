use touchbar_core::*;
use touchbar_model::*;

fn info(id: u64) -> SurfaceInfo {
    SurfaceInfo {
        id: SurfaceId(id),
        name: format!("headless:{id}").parse().unwrap(),
        kind: SurfaceKind::Headless,
        width_px: 500,
        height_px: 50,
        px_per_mm: 5.0,
        caps: SurfaceCaps {
            inject_keys: true,
            ..SurfaceCaps::default()
        },
    }
}
fn catalog(kind: WidgetKind, logic: bool) -> Catalog {
    let mut set = SetSpec::default_for("p/control".parse().unwrap());
    let mut w = WidgetSpec::new("control".parse().unwrap(), kind);
    if logic {
        set.logic = Some("p".parse().unwrap());
        for k in [
            ActionKind::Pressed,
            ActionKind::LongPressed,
            ActionKind::Activated,
            ActionKind::Released,
            ActionKind::Cancelled,
            ActionKind::ValueChanged,
            ActionKind::Selected,
        ] {
            w.handlers.insert(k, format!("{k:?}"));
        }
        w.hold_ms = Some(400);
    }
    set.widgets.push(w);
    let mut p = ProfileSpec::default_for("p/default".parse().unwrap());
    p.default = true;
    p.slots.push(SlotSpec {
        id: "main".parse().unwrap(),
        candidates: vec![set.reference.clone()],
    });
    let mut other = ProfileSpec::default_for("p/other".parse().unwrap());
    other.for_target = Some(ProfileTarget::App("other".into()));
    Catalog {
        plugins: vec![PluginSpec {
            id: "p".parse().unwrap(),
            revision: "v1".into(),
            enabled: true,
            has_logic: logic,
            requires: vec![],
        }],
        sets: vec![set],
        profiles: vec![p, other],
        ..Catalog::default()
    }
}
fn call(out: &[Output]) -> Option<(InstanceId, CallId, LogicCall)> {
    out.iter().find_map(|o| {
        if let Output::CallLogic {
            instance,
            call,
            invocation,
        } = o
        {
            Some((*instance, *call, invocation.clone()))
        } else {
            None
        }
    })
}
fn ack(c: &mut Core, at: u64, out: &[Output]) -> Vec<Output> {
    let (instance, call, _) = call(out).expect("logic call");
    c.handle(
        Timestamp(at),
        Input::LogicUpdate {
            instance,
            call,
            update: LogicUpdate::default(),
        },
    )
}
fn setup(kind: WidgetKind, logic: bool) -> (Core, InstanceId) {
    let mut c = Core::new(CoreConfig::default());
    c.handle(Timestamp(0), Input::Catalog(catalog(kind, logic)));
    c.handle(Timestamp(0), Input::SurfaceAdded(info(1)));
    let id = c.snapshot().instances[0].id;
    if logic {
        let actions = [
            ActionKind::Pressed,
            ActionKind::LongPressed,
            ActionKind::Activated,
            ActionKind::Released,
            ActionKind::Cancelled,
            ActionKind::ValueChanged,
            ActionKind::Selected,
        ]
        .into_iter()
        .map(|k| {
            (
                format!("{k:?}"),
                ActionDescription {
                    kinds: vec![k],
                    value: (k == ActionKind::ValueChanged).then_some(ScalarKind::Number),
                },
            )
        })
        .collect();
        let init = c.handle(
            Timestamp(0),
            Input::LogicDescribed {
                persisted: vec![],
                instance: id,
                description: LogicDescription {
                    actions,
                    state: vec![StateField {
                        name: "level".into(),
                        kind: FieldKind::Scalar(ScalarKind::Number),
                        persist: false,
                    }],
                },
            },
        );
        let shown = ack(&mut c, 0, &init);
        ack(&mut c, 0, &shown);
    }
    layout(&mut c, 0, id, vec![hit(id, "control", 10.0)]);
    (c, id)
}
fn hit(id: InstanceId, widget: &str, x: f32) -> HitTarget {
    HitTarget {
        instance: id,
        widget: widget.parse().unwrap(),
        plane: ScenePlane::Profile,
        rect: Rect {
            x,
            y: 10.0,
            width: 100.0,
            height: 20.0,
        },
        record_key: None,
    }
}
fn layout(c: &mut Core, at: u64, _id: InstanceId, hit_map: HitMap) {
    let generation = c.snapshot().surfaces[0].generation;
    c.handle(
        Timestamp(at),
        Input::Layout {
            slots: vec![],
            presentation: None,
            surface: SurfaceId(1),
            generation,
            hit_map,
            scrubbers: vec![],
            representations: vec![],
        },
    );
}
fn touch(c: &mut Core, at: u64, id: u32, phase: TouchPhase, x: f32, y: f32) -> Vec<Output> {
    c.handle(
        Timestamp(at),
        Input::Touch {
            surface: SurfaceId(1),
            contact: ContactId::new(id).unwrap(),
            phase,
            x,
            y,
        },
    )
}
fn action(out: &[Output]) -> ActionEvent {
    match call(out).expect("action call").2 {
        LogicCall::Handle(LogicEvent::Action(a)) => a,
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn pending_expiry_via_fact_must_cancel_preserved_capture() {
    let (mut c, id) = setup(WidgetKind::Key { key: SystemKey::F1 }, false);
    let mut cat = catalog(WidgetKind::Key { key: SystemKey::F1 }, false);
    cat.profiles[1].slots = cat.profiles[0].slots.clone();
    c.handle(Timestamp(1), Input::Catalog(cat));
    let down = touch(&mut c, 10, 1, TouchPhase::Down, 20.0, 20.0);
    c.handle(
        Timestamp(11),
        Input::Fact {
            name: "app.id".into(),
            value: Some(FactValue::Text("other".into())),
        },
    );
    assert_eq!(
        c.snapshot().surfaces[0]
            .profile
            .pending
            .as_ref()
            .unwrap()
            .deadline,
        Timestamp(1511)
    );
    let out = c.handle(
        Timestamp(1511),
        Input::Fact {
            name: "unrelated".into(),
            value: Some(FactValue::Boolean(true)),
        },
    );
    assert_eq!(c.snapshot().instances[0].id, id);
    assert_eq!(
        c.snapshot().surfaces[0]
            .profile
            .active
            .as_ref()
            .unwrap()
            .to_string(),
        "p/other"
    );
    assert!(
        c.snapshot().captures.is_empty(),
        "capture survived deadline: {out:?}"
    );
    assert!(out.iter().any(|o| matches!(
        o,
        Output::SendKey {
            phase: KeyPhase::Released,
            ..
        }
    )));
    let _ = down;
}
#[test]
fn queued_cancel_must_survive_native_hide() {
    let (mut c, _) = setup(WidgetKind::Button, true);
    let mut s = info(1);
    s.caps.modes = true;
    c.handle(Timestamp(1), Input::SurfaceChanged(s));
    let id = c.snapshot().instances[0].id;
    let generation = c.snapshot().surfaces[0].generation;
    c.handle(
        Timestamp(1),
        Input::FrameDone {
            surface: SurfaceId(1),
            generation,
        },
    );
    layout(&mut c, 1, id, vec![hit(id, "control", 10.0)]);
    let down = touch(&mut c, 10, 1, TouchPhase::Down, 20.0, 20.0);
    assert_eq!(action(&down).kind, ActionKind::Pressed);
    c.handle(
        Timestamp(11),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Native,
        },
    );
    assert!(c.snapshot().captures.is_empty());
    assert_eq!(c.snapshot().instances[0].status, InstanceStatus::Hidden);
    let next = ack(&mut c, 12, &down);
    assert!(
        matches!(
            call(&next),
            Some((
                _,
                _,
                LogicCall::Handle(LogicEvent::Action(ActionEvent {
                    kind: ActionKind::Cancelled,
                    ..
                }))
            ))
        ),
        "cancel lost: {next:?}"
    );
    let hidden = ack(&mut c, 13, &next);
    assert!(matches!(
        call(&hidden),
        Some((_, _, LogicCall::Handle(LogicEvent::Hidden)))
    ));
    assert!(call(&ack(&mut c, 14, &hidden)).is_none());
}

#[test]
fn pending_expiry_via_fact_must_discard_recent_physical_callback() {
    let (mut c, id) = setup(WidgetKind::Button, true);
    let mut cat = catalog(WidgetKind::Button, true);
    cat.sets[0].widgets[0].hold_ms = None;
    cat.profiles[1].slots = cat.profiles[0].slots.clone();
    c.handle(Timestamp(1), Input::Catalog(cat));
    let down = touch(&mut c, 10, 1, TouchPhase::Down, 20.0, 20.0);
    ack(&mut c, 10, &down);
    c.handle(
        Timestamp(11),
        Input::Fact {
            name: "app.id".into(),
            value: Some(FactValue::Text("other".into())),
        },
    );
    let up = touch(&mut c, 1505, 1, TouchPhase::Up, 20.0, 20.0);
    assert_eq!(action(&up).kind, ActionKind::Activated);
    c.handle(
        Timestamp(1511),
        Input::Fact {
            name: "unrelated".into(),
            value: Some(FactValue::Boolean(true)),
        },
    );
    let (_, call, _) = call(&up).unwrap();
    let out = c.handle(
        Timestamp(1512),
        Input::LogicUpdate {
            instance: id,
            call,
            update: LogicUpdate {
                requests: vec![EffectRequest {
                    tag: "stale-write".into(),
                    effect: Effect::MediaNext,
                    lifetime: None,
                    coalesce: None,
                    timeout_ms: None,
                }],
                ..LogicUpdate::default()
            },
        },
    );
    assert!(
        !out.iter().any(|o| matches!(o, Output::StartEffect { .. })),
        "expired callback started effect: {out:?}"
    );
}
