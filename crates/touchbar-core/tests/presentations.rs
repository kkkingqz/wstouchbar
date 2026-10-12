use touchbar_core::*;
use touchbar_model::*;

fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}
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
fn catalog() -> Catalog {
    let mut set = SetSpec::default_for("p/main".parse().unwrap());
    set.logic = Some("p".parse().unwrap());
    let mut source = WidgetSpec::new("source".parse().unwrap(), WidgetKind::Button);
    for k in [
        ActionKind::Pressed,
        ActionKind::Activated,
        ActionKind::Released,
        ActionKind::Cancelled,
    ] {
        source.handlers.insert(k, format!("{k:?}"));
    }
    let mut choose = WidgetSpec::new("choose".parse().unwrap(), WidgetKind::Button);
    choose
        .handlers
        .insert(ActionKind::Activated, "choose".into());
    set.widgets = vec![source];
    for id in ["first", "second"] {
        let widget = if id == "first" {
            choose.clone()
        } else {
            WidgetSpec::new(
                "key".parse().unwrap(),
                WidgetKind::Key { key: SystemKey::F1 },
            )
        };
        set.panels.push(PanelSpec {
            id: id.parse().unwrap(),
            widgets: vec![widget],
            dismiss_on_selection: false,
            principal_item: None,
            dismissal: DismissalPolicy {
                release: false,
                outside_press: true,
                ..DismissalPolicy::default()
            },
        });
    }
    let mut profile = ProfileSpec::default_for("p/default".parse().unwrap());
    profile.default = true;
    profile.slots.push(SlotSpec {
        id: "main".parse().unwrap(),
        candidates: vec![set.reference.clone()],
    });
    Catalog {
        plugins: vec![PluginSpec {
            id: "p".parse().unwrap(),
            revision: "v1".into(),
            enabled: true,
            has_logic: true,
            requires: vec![],
        }],
        sets: vec![set],
        profiles: vec![profile],
        ..Catalog::default()
    }
}
fn call(out: &[Output]) -> Option<(InstanceId, CallId, LogicCall)> {
    out.iter().find_map(|o| match o {
        Output::CallLogic {
            instance,
            call,
            invocation,
        } => Some((*instance, *call, invocation.clone())),
        _ => None,
    })
}
fn reply(
    c: &mut Core,
    at: u64,
    out: &[Output],
    presentation: Option<PresentationCommand>,
) -> Vec<Output> {
    let (instance, call, _) = call(out).expect("logic call");
    c.handle(
        Timestamp(at),
        Input::LogicUpdate {
            instance,
            call,
            update: LogicUpdate {
                presentation,
                ..LogicUpdate::default()
            },
        },
    )
}
fn drain(c: &mut Core, at: u64, mut out: Vec<Output>) {
    while call(&out).is_some() {
        out = reply(c, at, &out, None);
    }
}
fn open(
    panel: &str,
    placement: Placement,
    lifecycle: PresentationLifecycle,
) -> Option<PresentationCommand> {
    Some(PresentationCommand::Open {
        panel: panel.parse().unwrap(),
        placement,
        lifecycle,
    })
}
fn hit(id: InstanceId, widget: &str, plane: ScenePlane, bounds: Rect) -> HitTarget {
    HitTarget {
        instance: id,
        widget: widget.parse().unwrap(),
        plane,
        rect: bounds,
        record_key: None,
    }
}
fn layout(c: &mut Core, at: u64, surface: u64, map: HitMap, bounds: Option<Rect>) {
    let snapshot = c.snapshot();
    let s = snapshot
        .surfaces
        .iter()
        .find(|s| s.info.id == SurfaceId(surface))
        .unwrap();
    let presentation = bounds.map(|bounds| PresentationLayout {
        id: s.scene.presentation.as_ref().unwrap().id,
        bounds,
    });
    let out = c.handle(
        Timestamp(at),
        Input::Layout {
            surface: SurfaceId(surface),
            generation: s.generation,
            hit_map: map,
            representations: vec![],
            scrubbers: vec![],
            slots: vec![SlotBounds {
                instance: s.scene.items[0].instance,
                rect: rect(0.0, 0.0, 200.0, 50.0),
            }],
            presentation,
        },
    );
    // The fake runtime acknowledges context metadata before panel actions.
    for command in out {
        if let Output::CallLogic {
            instance,
            call,
            invocation: LogicCall::Handle(LogicEvent::ContextChanged(_)),
        } = command
        {
            c.handle(
                Timestamp(at),
                Input::LogicUpdate {
                    instance,
                    call,
                    update: LogicUpdate::default(),
                },
            );
        }
    }
}
fn frame(c: &mut Core, at: u64, surface: u64) {
    let generation = c
        .snapshot()
        .surfaces
        .iter()
        .find(|s| s.info.id == SurfaceId(surface))
        .unwrap()
        .generation;
    c.handle(
        Timestamp(at),
        Input::FrameDone {
            surface: SurfaceId(surface),
            generation,
        },
    );
}
fn touch(c: &mut Core, at: u64, contact: u32, phase: TouchPhase, x: f32) -> Vec<Output> {
    c.handle(
        Timestamp(at),
        Input::Touch {
            surface: SurfaceId(1),
            contact: ContactId::new(contact).unwrap(),
            phase,
            x,
            y: 20.0,
        },
    )
}
fn setup() -> (Core, InstanceId, Vec<Output>) {
    let mut c = Core::new(CoreConfig::default());
    c.handle(Timestamp(0), Input::Catalog(catalog()));
    let created = c.handle(Timestamp(0), Input::SurfaceAdded(info(1)));
    let id = created
        .iter()
        .find_map(|o| match o {
            Output::CreateInstance { instance, .. } => Some(*instance),
            _ => None,
        })
        .unwrap();
    let mut actions: Vec<_> = [
        ActionKind::Pressed,
        ActionKind::Activated,
        ActionKind::Released,
        ActionKind::Cancelled,
    ]
    .into_iter()
    .map(|k| {
        (
            format!("{k:?}"),
            ActionDescription {
                kinds: vec![k],
                value: None,
            },
        )
    })
    .collect();
    actions.push((
        "choose".into(),
        ActionDescription {
            kinds: vec![ActionKind::Activated],
            value: None,
        },
    ));
    let init = c.handle(
        Timestamp(0),
        Input::LogicDescribed {
            persisted: vec![],
            instance: id,
            description: LogicDescription {
                actions,
                state: vec![],
            },
        },
    );
    let shown = reply(&mut c, 0, &init, None);
    drain(&mut c, 0, shown);
    frame(&mut c, 0, 1);
    layout(
        &mut c,
        0,
        1,
        vec![hit(
            id,
            "source",
            ScenePlane::Profile,
            rect(10.0, 10.0, 100.0, 20.0),
        )],
        None,
    );
    (c, id, init)
}
#[test]
fn physical_open_keeps_owner_and_supplies_anchor_and_region() {
    for (placement, expected) in [
        (Placement::Anchored, rect(0.0, 0.0, 200.0, 50.0)),
        (Placement::Region, rect(0.0, 0.0, 200.0, 50.0)),
        (Placement::Full, rect(0.0, 0.0, 500.0, 50.0)),
    ] {
        let (mut c, id, _) = setup();
        let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
        let out = reply(
            &mut c,
            2,
            &down,
            open("first", placement, PresentationLifecycle::Persistent),
        );
        let s = c.snapshot();
        let panel = s.surfaces[0].scene.presentation.as_ref().expect("panel");
        assert_eq!(panel.owner, id);
        assert_eq!(panel.anchor, rect(10.0, 10.0, 100.0, 20.0));
        assert_eq!(panel.region, expected);
        assert_eq!(s.instances.len(), 1);
        assert_eq!(s.instances[0].status, InstanceStatus::Visible);
        assert!(!out.iter().any(|o| matches!(
            o,
            Output::CreateInstance { .. } | Output::DisposeInstance { .. }
        )));
    }
}
#[test]
fn init_cannot_open_presentation() {
    let (mut c, id, _) = setup();
    // Hide/show through modes is tested separately; initial Init is also synthetic authority.
    let other = c.handle(Timestamp(1), Input::SurfaceAdded(info(2)));
    let id2 = other
        .iter()
        .find_map(|o| match o {
            Output::CreateInstance { instance, .. } => Some(*instance),
            _ => None,
        })
        .unwrap();
    let init = c.handle(
        Timestamp(1),
        Input::LogicDescribed {
            persisted: vec![],
            instance: id2,
            description: LogicDescription {
                actions: catalog().sets[0]
                    .all_widgets()
                    .into_iter()
                    .flat_map(|w| {
                        w.handlers.iter().map(|(kind, action)| {
                            (
                                action.clone(),
                                ActionDescription {
                                    kinds: vec![*kind],
                                    value: None,
                                },
                            )
                        })
                    })
                    .collect(),
                state: vec![],
            },
        },
    );
    let out = reply(
        &mut c,
        2,
        &init,
        open("first", Placement::Full, PresentationLifecycle::Persistent),
    );
    assert!(out.iter().any(|o| matches!(
        o,
        Output::Log {
            level: LogLevel::Warn,
            ..
        }
    )));
    assert!(
        c.snapshot()
            .surfaces
            .iter()
            .all(|s| s.scene.presentation.is_none())
    );
    assert_eq!(c.snapshot().instances[0].id, id);
}
#[test]
fn unrelated_fact_keeps_panel_in_render_scene() {
    let (mut c, _, _) = setup();
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    reply(
        &mut c,
        2,
        &down,
        open("first", Placement::Full, PresentationLifecycle::Persistent),
    );
    c.handle(
        Timestamp(3),
        Input::Fact {
            name: "irrelevant".into(),
            value: Some(FactValue::Boolean(true)),
        },
    );
    assert!(c.snapshot().surfaces[0].scene.presentation.is_some());
}
#[test]
fn synthetic_presentation_closed_cannot_reopen_panel() {
    let (mut c, _, _) = setup();
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    reply(
        &mut c,
        2,
        &down,
        open("first", Placement::Full, PresentationLifecycle::Transient),
    );
    let up = touch(&mut c, 3, 1, TouchPhase::Up, 20.0);
    let released = reply(&mut c, 4, &up, None);
    let closed = reply(&mut c, 4, &released, None);
    let out = reply(
        &mut c,
        5,
        &closed,
        open("first", Placement::Full, PresentationLifecycle::Persistent),
    );
    assert!(out.iter().any(|o| matches!(
        o,
        Output::Log {
            level: LogLevel::Warn,
            ..
        }
    )));
    assert!(c.snapshot().surfaces[0].scene.presentation.is_none());
}
#[test]
fn delayed_persistent_uses_original_anchor_but_transient_cannot_use_reused_contact_id() {
    for lifecycle in [
        PresentationLifecycle::Transient,
        PresentationLifecycle::Persistent,
    ] {
        let (mut c, _, _) = setup();
        let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
        touch(&mut c, 2, 1, TouchPhase::Up, 20.0);
        touch(&mut c, 3, 1, TouchPhase::Down, 30.0);
        reply(
            &mut c,
            4,
            &down,
            open("first", Placement::Anchored, lifecycle),
        );
        let snapshot = c.snapshot();
        let panel = snapshot.surfaces[0].scene.presentation.as_ref();
        assert_eq!(
            panel.is_some(),
            lifecycle == PresentationLifecycle::Persistent
        );
        if let Some(panel) = panel {
            assert_eq!(panel.anchor.x, 10.0);
        }
    }
}
#[test]
fn contacts_started_before_panel_keep_owner_even_when_panel_map_omits_them() {
    let (mut c, id, _) = setup();
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    reply(
        &mut c,
        2,
        &down,
        open("first", Placement::Full, PresentationLifecycle::Persistent),
    );
    frame(&mut c, 3, 1);
    layout(
        &mut c,
        3,
        1,
        vec![hit(
            id,
            "choose",
            ScenePlane::Presentation,
            rect(150.0, 10.0, 100.0, 20.0),
        )],
        Some(rect(100.0, 0.0, 200.0, 50.0)),
    );
    assert_eq!(c.snapshot().captures.len(), 1);
    let up = touch(&mut c, 4, 1, TouchPhase::Up, 20.0);
    assert!(matches!(
        call(&up).unwrap().2,
        LogicCall::Handle(LogicEvent::Action(ActionEvent {
            kind: ActionKind::Activated,
            ..
        }))
    ));
    drain(&mut c, 4, up);
    assert!(c.snapshot().captures.is_empty());
}
#[test]
fn transient_closes_on_original_anchor_release() {
    let (mut c, _, _) = setup();
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    reply(
        &mut c,
        2,
        &down,
        open(
            "first",
            Placement::Anchored,
            PresentationLifecycle::Transient,
        ),
    );
    let up = touch(&mut c, 3, 1, TouchPhase::Up, 20.0);
    assert!(c.snapshot().surfaces[0].scene.presentation.is_none());
    let activated = reply(&mut c, 4, &up, None);
    let closed = reply(&mut c, 4, &activated, None);
    assert!(matches!(
        call(&closed).unwrap().2,
        LogicCall::Handle(LogicEvent::PresentationClosed(_))
    ));
}
#[test]
fn panel_routes_new_contact_to_its_widgets_and_outside_press_cancels_panel_key() {
    let (mut c, id, _) = setup();
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    reply(
        &mut c,
        2,
        &down,
        open("second", Placement::Full, PresentationLifecycle::Persistent),
    );
    frame(&mut c, 3, 1);
    layout(
        &mut c,
        3,
        1,
        vec![hit(
            id,
            "key",
            ScenePlane::Presentation,
            rect(150.0, 10.0, 100.0, 20.0),
        )],
        Some(rect(100.0, 0.0, 200.0, 50.0)),
    );
    let key = touch(&mut c, 4, 2, TouchPhase::Down, 160.0);
    assert!(key.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Pressed
    }));
    // Inside a panel gap: neither outside dismissal nor activation of the profile.
    touch(&mut c, 5, 3, TouchPhase::Down, 110.0);
    assert!(c.snapshot().surfaces[0].scene.presentation.is_some());
    assert_eq!(c.snapshot().captures.len(), 2);
    let outside = touch(&mut c, 6, 4, TouchPhase::Down, 20.0);
    assert!(c.snapshot().surfaces[0].scene.presentation.is_none());
    assert!(outside.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Released
    }));
    assert_eq!(c.snapshot().captures.len(), 1); // original pre-panel contact
    assert!(call(&outside).is_some()); // presentation-closed; no obscured profile press
}
#[test]
fn selection_closes_only_after_logic_reply_and_never_closes_replacement() {
    for replace in [false, true] {
        let (mut c, id, _) = setup();
        let mut cat = catalog();
        cat.sets[0].panels[0].dismiss_on_selection = true;
        c.handle(Timestamp(0), Input::Catalog(cat));
        let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
        reply(
            &mut c,
            2,
            &down,
            open("first", Placement::Full, PresentationLifecycle::Persistent),
        );
        frame(&mut c, 3, 1);
        layout(
            &mut c,
            3,
            1,
            vec![hit(
                id,
                "choose",
                ScenePlane::Presentation,
                rect(150.0, 10.0, 100.0, 20.0),
            )],
            Some(rect(100.0, 0.0, 200.0, 50.0)),
        );
        touch(&mut c, 4, 2, TouchPhase::Down, 160.0);
        let selected = touch(&mut c, 5, 2, TouchPhase::Up, 160.0);
        assert!(
            matches!(call(&selected).unwrap().2, LogicCall::Handle(LogicEvent::Action(ActionEvent { action, kind: ActionKind::Activated, .. })) if action == "choose")
        );
        assert!(c.snapshot().surfaces[0].scene.presentation.is_some());
        let response = if replace {
            open("second", Placement::Full, PresentationLifecycle::Persistent)
        } else {
            None
        };
        reply(&mut c, 6, &selected, response);
        let snapshot = c.snapshot();
        let panel = snapshot.surfaces[0].scene.presentation.as_ref();
        assert_eq!(panel.is_some(), replace);
        if let Some(panel) = panel {
            assert_eq!(panel.panel.id.to_string(), "second");
        }
    }
}
#[test]
fn timeout_closes_panel_and_stale_timer_cannot_close_replacement() {
    let (mut c, _, _) = setup();
    let mut cat = catalog();
    cat.sets[0].panels[0].dismissal.timeout_ms = Some(100);
    c.handle(Timestamp(0), Input::Catalog(cat));
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    let opened = reply(
        &mut c,
        2,
        &down,
        open("first", Placement::Full, PresentationLifecycle::Persistent),
    );
    let timer = opened
        .iter()
        .find_map(|o| match o {
            Output::SetTimer {
                id,
                at: Timestamp(102),
            } => Some(*id),
            _ => None,
        })
        .unwrap();
    let expired = c.handle(Timestamp(102), Input::Timer { id: timer });
    assert!(c.snapshot().surfaces[0].scene.presentation.is_none());
    drain(&mut c, 102, expired);
    let up = touch(&mut c, 103, 1, TouchPhase::Up, 20.0);
    drain(&mut c, 103, up);
    let next = touch(&mut c, 104, 2, TouchPhase::Down, 20.0);
    reply(
        &mut c,
        105,
        &next,
        open("second", Placement::Full, PresentationLifecycle::Persistent),
    );
    c.handle(Timestamp(106), Input::Timer { id: timer });
    assert_eq!(
        c.snapshot().surfaces[0]
            .scene
            .presentation
            .as_ref()
            .unwrap()
            .panel
            .id
            .to_string(),
        "second"
    );
}
#[test]
fn anchor_hardware_cancel_closes_transient_panel() {
    let (mut c, _, _) = setup();
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    reply(
        &mut c,
        2,
        &down,
        open("first", Placement::Full, PresentationLifecycle::Transient),
    );
    touch(&mut c, 3, 1, TouchPhase::Cancel, 20.0);
    assert!(c.snapshot().surfaces[0].scene.presentation.is_none());
}
#[test]
fn no_logic_expanded_panel_uses_same_owner_and_opens_on_activation() {
    let mut cat = catalog();
    cat.sets[0].logic = None;
    cat.sets[0].widgets[0].handlers.clear();
    cat.sets[0].widgets[0].expanded_panel = Some("second".parse().unwrap());
    cat.sets[0].panels.remove(0);
    let mut c = Core::new(CoreConfig::default());
    c.handle(Timestamp(0), Input::Catalog(cat));
    c.handle(Timestamp(0), Input::SurfaceAdded(info(1)));
    let id = c.snapshot().instances[0].id;
    layout(
        &mut c,
        0,
        1,
        vec![hit(
            id,
            "source",
            ScenePlane::Profile,
            rect(10.0, 10.0, 100.0, 20.0),
        )],
        None,
    );
    touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    let up = touch(&mut c, 2, 1, TouchPhase::Up, 20.0);
    assert_eq!(
        c.snapshot().surfaces[0]
            .scene
            .presentation
            .as_ref()
            .unwrap()
            .owner,
        id
    );
    assert!(call(&up).is_none());
}
#[test]
fn profile_switch_closes_panel_even_when_owner_instance_is_preserved() {
    let (mut c, id, _) = setup();
    let mut cat = catalog();
    let mut other = cat.profiles[0].clone();
    other.reference = "p/other".parse().unwrap();
    other.default = false;
    other.for_target = Some(ProfileTarget::App("other".into()));
    cat.profiles.push(other);
    c.handle(Timestamp(0), Input::Catalog(cat));
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    reply(
        &mut c,
        2,
        &down,
        open("first", Placement::Full, PresentationLifecycle::Persistent),
    );
    let up = touch(&mut c, 3, 1, TouchPhase::Up, 20.0);
    drain(&mut c, 3, up);
    c.handle(
        Timestamp(4),
        Input::Fact {
            name: "app.id".into(),
            value: Some(FactValue::Text("other".into())),
        },
    );
    assert_eq!(c.snapshot().instances[0].id, id);
    assert!(c.snapshot().surfaces[0].scene.presentation.is_none());
}
#[test]
fn stale_panel_layout_cannot_restore_hits_while_close_waits_for_next_render() {
    let (mut c, id, _) = setup();
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    reply(
        &mut c,
        2,
        &down,
        open("second", Placement::Full, PresentationLifecycle::Persistent),
    );
    frame(&mut c, 3, 1);
    let snapshot = c.snapshot();
    let generation = snapshot.surfaces[0].generation;
    let panel_id = snapshot.surfaces[0].scene.presentation.as_ref().unwrap().id;
    let up = touch(&mut c, 4, 1, TouchPhase::Up, 20.0);
    let released = reply(&mut c, 5, &up, Some(PresentationCommand::Close));
    drain(&mut c, 5, released);
    assert!(c.snapshot().surfaces[0].scene.presentation.is_none());
    c.handle(
        Timestamp(6),
        Input::Layout {
            surface: SurfaceId(1),
            generation,
            hit_map: vec![hit(
                id,
                "source",
                ScenePlane::Profile,
                rect(10.0, 10.0, 100.0, 20.0),
            )],
            representations: vec![],
            scrubbers: vec![],
            slots: vec![],
            presentation: Some(PresentationLayout {
                id: panel_id,
                bounds: rect(100.0, 0.0, 200.0, 50.0),
            }),
        },
    );
    assert_eq!(
        c.snapshot().surfaces[0].layout_generation,
        snapshot.surfaces[0].layout_generation
    );
}
#[test]
fn invalid_slot_geometry_rejects_layout_atomically() {
    let (mut c, id, _) = setup();
    let generation = c.snapshot().surfaces[0].generation;
    let out = c.handle(
        Timestamp(1),
        Input::Layout {
            surface: SurfaceId(1),
            generation,
            hit_map: vec![],
            representations: vec![],
            scrubbers: vec![],
            presentation: None,
            slots: vec![SlotBounds {
                instance: id,
                rect: rect(f32::NAN, 0.0, 200.0, 50.0),
            }],
        },
    );
    assert!(out.iter().any(|o| matches!(
        o,
        Output::Log {
            level: LogLevel::Warn,
            ..
        }
    )));
    let down = touch(&mut c, 2, 1, TouchPhase::Down, 20.0);
    assert!(call(&down).is_some()); // prior valid map survived
}
#[test]
fn removing_surface_cancels_panel_timeout() {
    let (mut c, _, _) = setup();
    let mut cat = catalog();
    cat.sets[0].panels[0].dismissal.timeout_ms = Some(100);
    c.handle(Timestamp(0), Input::Catalog(cat));
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    let opened = reply(
        &mut c,
        2,
        &down,
        open("first", Placement::Full, PresentationLifecycle::Persistent),
    );
    let timer = opened
        .iter()
        .find_map(|o| match o {
            Output::SetTimer { id, .. } => Some(*id),
            _ => None,
        })
        .unwrap();
    let removed = c.handle(Timestamp(3), Input::SurfaceRemoved(SurfaceId(1)));
    assert!(removed.contains(&Output::ClearTimer { id: timer }));
    assert!(c.snapshot().captures.is_empty());
}

#[test]
fn selection_error_still_dismisses_panel() {
    let (mut c, id, _) = setup();
    let mut cat = catalog();
    cat.sets[0].panels[0].dismissal.selection = true;
    c.handle(Timestamp(0), Input::Catalog(cat));
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    reply(
        &mut c,
        2,
        &down,
        open("first", Placement::Full, PresentationLifecycle::Persistent),
    );
    frame(&mut c, 3, 1);
    layout(
        &mut c,
        3,
        1,
        vec![hit(
            id,
            "choose",
            ScenePlane::Presentation,
            rect(150.0, 10.0, 100.0, 20.0),
        )],
        Some(rect(100.0, 0.0, 200.0, 50.0)),
    );
    touch(&mut c, 4, 2, TouchPhase::Down, 160.0);
    let selected = touch(&mut c, 5, 2, TouchPhase::Up, 160.0);
    let (instance, call, _) = call(&selected).unwrap();
    let closed = c.handle(
        Timestamp(6),
        Input::LogicError {
            instance,
            call,
            message: "ordinary error".into(),
        },
    );
    assert!(c.snapshot().surfaces[0].scene.presentation.is_none());
    assert!(matches!(
        crate::call(&closed).unwrap().2,
        LogicCall::Handle(LogicEvent::PresentationClosed(_))
    ));
}
#[test]
fn resize_closes_persistent_panel_and_invalidates_slot_bounds() {
    let (mut c, _, _) = setup();
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    reply(
        &mut c,
        2,
        &down,
        open("first", Placement::Full, PresentationLifecycle::Persistent),
    );
    let mut resized = info(1);
    resized.width_px = 600;
    c.handle(Timestamp(3), Input::SurfaceChanged(resized));
    assert!(c.snapshot().surfaces[0].scene.presentation.is_none());
}
#[test]
fn mismatched_panel_bounds_cannot_replace_valid_layout() {
    let (mut c, id, _) = setup();
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    reply(
        &mut c,
        2,
        &down,
        open("first", Placement::Full, PresentationLifecycle::Persistent),
    );
    frame(&mut c, 3, 1);
    layout(
        &mut c,
        3,
        1,
        vec![hit(
            id,
            "choose",
            ScenePlane::Presentation,
            rect(150.0, 10.0, 100.0, 20.0),
        )],
        Some(rect(100.0, 0.0, 200.0, 50.0)),
    );
    let s = c.snapshot().surfaces.remove(0);
    c.handle(
        Timestamp(4),
        Input::Layout {
            surface: SurfaceId(1),
            generation: s.generation,
            hit_map: vec![],
            representations: vec![],
            scrubbers: vec![],
            slots: vec![],
            presentation: Some(PresentationLayout {
                id: PresentationId(999),
                bounds: rect(0.0, 0.0, 10.0, 10.0),
            }),
        },
    );
    touch(&mut c, 5, 2, TouchPhase::Down, 160.0);
    assert_eq!(c.snapshot().captures.len(), 2);
}

#[test]
fn built_in_panel_key_selection_dismisses_without_logic_callback() {
    let (mut c, id, _) = setup();
    let mut cat = catalog();
    cat.sets[0].panels[1].dismissal.selection = true;
    c.handle(Timestamp(0), Input::Catalog(cat));
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    reply(
        &mut c,
        2,
        &down,
        open("second", Placement::Full, PresentationLifecycle::Persistent),
    );
    frame(&mut c, 3, 1);
    layout(
        &mut c,
        3,
        1,
        vec![hit(
            id,
            "key",
            ScenePlane::Presentation,
            rect(150.0, 10.0, 100.0, 20.0),
        )],
        Some(rect(100.0, 0.0, 200.0, 50.0)),
    );
    touch(&mut c, 4, 2, TouchPhase::Down, 160.0);
    let up = touch(&mut c, 5, 2, TouchPhase::Up, 160.0);
    assert!(up.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Released
    }));
    assert!(c.snapshot().surfaces[0].scene.presentation.is_none());
}
#[test]
fn hold_panel_opens_at_threshold_then_closes_on_anchor_release() {
    let mut cat = catalog();
    cat.sets[0].logic = None;
    cat.sets[0].widgets[0].handlers.clear();
    cat.sets[0].widgets[0].hold_ms = Some(100);
    cat.sets[0].widgets[0].press_and_hold_panel = Some("second".parse().unwrap());
    cat.sets[0].panels.remove(0);
    let mut c = Core::new(CoreConfig::default());
    c.handle(Timestamp(0), Input::Catalog(cat));
    c.handle(Timestamp(0), Input::SurfaceAdded(info(1)));
    let id = c.snapshot().instances[0].id;
    layout(
        &mut c,
        0,
        1,
        vec![hit(
            id,
            "source",
            ScenePlane::Profile,
            rect(10.0, 10.0, 100.0, 20.0),
        )],
        None,
    );
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    let timer = down
        .iter()
        .find_map(|o| match o {
            Output::SetTimer {
                id,
                at: Timestamp(101),
            } => Some(*id),
            _ => None,
        })
        .unwrap();
    c.handle(Timestamp(101), Input::Timer { id: timer });
    assert_eq!(
        c.snapshot().surfaces[0]
            .scene
            .presentation
            .as_ref()
            .unwrap()
            .lifecycle,
        PresentationLifecycle::Transient
    );
    touch(&mut c, 102, 1, TouchPhase::Up, 20.0);
    assert!(c.snapshot().surfaces[0].scene.presentation.is_none());
}

#[test]
fn response_from_closed_panel_cannot_open_another_panel() {
    let (mut c, id, _) = setup();
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    reply(
        &mut c,
        2,
        &down,
        open("first", Placement::Full, PresentationLifecycle::Persistent),
    );
    frame(&mut c, 3, 1);
    layout(
        &mut c,
        3,
        1,
        vec![hit(
            id,
            "choose",
            ScenePlane::Presentation,
            rect(150.0, 10.0, 100.0, 20.0),
        )],
        Some(rect(100.0, 0.0, 200.0, 50.0)),
    );
    touch(&mut c, 4, 2, TouchPhase::Down, 160.0);
    let selected = touch(&mut c, 5, 2, TouchPhase::Up, 160.0);
    touch(&mut c, 6, 3, TouchPhase::Down, 20.0); // dismiss while callback runs
    reply(
        &mut c,
        7,
        &selected,
        open("second", Placement::Full, PresentationLifecycle::Persistent),
    );
    assert!(c.snapshot().surfaces[0].scene.presentation.is_none());
}
#[test]
fn each_surface_has_its_own_panel_and_outside_policy_consumes_press() {
    let mut cat = catalog();
    cat.sets[0].logic = None;
    cat.sets[0].widgets[0].handlers.clear();
    cat.sets[0].widgets[0].expanded_panel = Some("second".parse().unwrap());
    cat.sets[0].panels.remove(0);
    cat.sets[0].panels[0].dismissal.outside_press = false;
    let mut c = Core::new(CoreConfig::default());
    c.handle(Timestamp(0), Input::Catalog(cat));
    for surface in [1, 2] {
        let base = surface * 10;
        c.handle(Timestamp(base), Input::SurfaceAdded(info(surface)));
        let id = c
            .snapshot()
            .surfaces
            .iter()
            .find(|s| s.info.id == SurfaceId(surface))
            .unwrap()
            .scene
            .items[0]
            .instance;
        layout(
            &mut c,
            base,
            surface,
            vec![hit(
                id,
                "source",
                ScenePlane::Profile,
                rect(10.0, 10.0, 100.0, 20.0),
            )],
            None,
        );
        for phase in [TouchPhase::Down, TouchPhase::Up] {
            c.handle(
                Timestamp(base + 1),
                Input::Touch {
                    surface: SurfaceId(surface),
                    contact: ContactId::new(1).unwrap(),
                    phase,
                    x: 20.0,
                    y: 20.0,
                },
            );
        }
        frame(&mut c, base + 2, surface);
        layout(
            &mut c,
            base + 2,
            surface,
            vec![hit(
                id,
                "key",
                ScenePlane::Presentation,
                rect(150.0, 10.0, 100.0, 20.0),
            )],
            Some(rect(100.0, 0.0, 200.0, 50.0)),
        );
    }
    assert!(
        c.snapshot()
            .surfaces
            .iter()
            .all(|s| s.scene.presentation.is_some())
    );
    touch(&mut c, 23, 2, TouchPhase::Down, 20.0);
    assert!(c.snapshot().captures.is_empty());
    assert!(
        c.snapshot()
            .surfaces
            .iter()
            .all(|s| s.scene.presentation.is_some())
    );
    c.handle(
        Timestamp(24),
        Input::Revoked {
            surface: SurfaceId(1),
        },
    );
    let s = c.snapshot().surfaces.remove(0);
    assert_eq!(s.info.id, SurfaceId(2));
    assert!(s.scene.presentation.is_some());
}
#[test]
fn persistent_release_policy_uses_only_original_anchor() {
    let (mut c, id, _) = setup();
    let mut cat = catalog();
    cat.sets[0].panels[1].dismissal.release = true;
    c.handle(Timestamp(0), Input::Catalog(cat));
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    reply(
        &mut c,
        2,
        &down,
        open("second", Placement::Full, PresentationLifecycle::Persistent),
    );
    frame(&mut c, 3, 1);
    layout(
        &mut c,
        3,
        1,
        vec![hit(
            id,
            "key",
            ScenePlane::Presentation,
            rect(150.0, 10.0, 100.0, 20.0),
        )],
        Some(rect(100.0, 0.0, 200.0, 50.0)),
    );
    touch(&mut c, 4, 2, TouchPhase::Down, 160.0);
    touch(&mut c, 5, 2, TouchPhase::Up, 160.0);
    assert!(c.snapshot().surfaces[0].scene.presentation.is_some());
    touch(&mut c, 6, 1, TouchPhase::Up, 20.0);
    assert!(c.snapshot().surfaces[0].scene.presentation.is_none());
}

#[test]
fn built_in_selection_can_replace_panel_before_dismissal() {
    let (mut c, id, _) = setup();
    let mut cat = catalog();
    cat.sets[0].panels[0].dismissal.selection = true;
    cat.sets[0].panels[0].widgets[0].handlers.clear();
    cat.sets[0].panels[0].widgets[0].expanded_panel = Some("second".parse().unwrap());
    c.handle(Timestamp(0), Input::Catalog(cat));
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    reply(
        &mut c,
        2,
        &down,
        open("first", Placement::Full, PresentationLifecycle::Persistent),
    );
    frame(&mut c, 3, 1);
    layout(
        &mut c,
        3,
        1,
        vec![hit(
            id,
            "choose",
            ScenePlane::Presentation,
            rect(150.0, 10.0, 100.0, 20.0),
        )],
        Some(rect(100.0, 0.0, 200.0, 50.0)),
    );
    touch(&mut c, 4, 2, TouchPhase::Down, 160.0);
    touch(&mut c, 5, 2, TouchPhase::Up, 160.0);
    assert_eq!(
        c.snapshot().surfaces[0]
            .scene
            .presentation
            .as_ref()
            .unwrap()
            .panel
            .id
            .to_string(),
        "second"
    );
}

#[test]
fn cancelled_physical_callback_cannot_open_persistent_panel() {
    let (mut c, _, _) = setup();
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    touch(&mut c, 2, 1, TouchPhase::Cancel, 20.0);
    assert!(c.snapshot().captures.is_empty());
    // Revocation survives a checkpoint, without affecting normal Up semantics.
    let state = serde_json::to_vec(&c.checkpoint()).unwrap();
    c = Core::restore(serde_json::from_slice(&state).unwrap()).unwrap();
    let cancelled = reply(
        &mut c,
        3,
        &down,
        open("first", Placement::Full, PresentationLifecycle::Persistent),
    );
    assert!(
        c.snapshot().surfaces[0].presentation.is_none(),
        "cancelled press opened a panel"
    );
    reply(
        &mut c,
        4,
        &cancelled,
        open("first", Placement::Full, PresentationLifecycle::Persistent),
    );
    assert!(
        c.snapshot().surfaces[0].presentation.is_none(),
        "cancel callback opened a panel"
    );
}

#[test]
fn cancelled_queued_physical_callback_cannot_open_persistent_panel() {
    let (mut c, _, _) = setup();
    let first = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    touch(&mut c, 2, 2, TouchPhase::Down, 20.0);
    touch(&mut c, 3, 2, TouchPhase::Cancel, 20.0);
    let second = reply(&mut c, 4, &first, None);
    assert!(matches!(
        call(&second),
        Some((
            _,
            _,
            LogicCall::Handle(LogicEvent::Action(ActionEvent {
                kind: ActionKind::Pressed,
                ..
            }))
        ))
    ));
    reply(
        &mut c,
        5,
        &second,
        open("first", Placement::Full, PresentationLifecycle::Persistent),
    );
    assert!(c.snapshot().surfaces[0].presentation.is_none());
}

#[test]
fn cancelling_other_contact_preserves_live_callback_panel_permission() {
    let (mut c, _, _) = setup();
    let first = touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    touch(&mut c, 2, 2, TouchPhase::Down, 20.0);
    touch(&mut c, 3, 2, TouchPhase::Cancel, 20.0);
    reply(
        &mut c,
        4,
        &first,
        open("first", Placement::Full, PresentationLifecycle::Persistent),
    );
    assert!(c.snapshot().surfaces[0].presentation.is_some());
}

#[test]
fn expanded_panel_survives_the_release_that_opens_it() {
    let mut cat = catalog();
    cat.sets[0].logic = None;
    cat.sets[0].widgets[0].handlers.clear();
    cat.sets[0].widgets[0].expanded_panel = Some("second".parse().unwrap());
    cat.sets[0].panels.remove(0);
    cat.sets[0].panels[0].dismissal.release = true;
    let mut c = Core::new(CoreConfig::default());
    c.handle(Timestamp(0), Input::Catalog(cat));
    c.handle(Timestamp(0), Input::SurfaceAdded(info(1)));
    let id = c.snapshot().instances[0].id;
    layout(
        &mut c,
        0,
        1,
        vec![hit(
            id,
            "source",
            ScenePlane::Profile,
            rect(10.0, 10.0, 100.0, 20.0),
        )],
        None,
    );
    touch(&mut c, 1, 1, TouchPhase::Down, 20.0);
    touch(&mut c, 2, 1, TouchPhase::Up, 20.0);
    let panel = c.snapshot().surfaces[0]
        .scene
        .presentation
        .clone()
        .expect("expanded panel");
    assert_eq!(panel.panel.id, "second".parse().unwrap());
    assert_eq!(panel.lifecycle, PresentationLifecycle::Persistent);
    assert!(c.snapshot().captures.is_empty());
}
