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
fn button_finishes_at_original_owner_after_layout_moves() {
    let (mut c, id) = setup(WidgetKind::Button, true);
    let down = touch(&mut c, 10, 1, TouchPhase::Down, 20.0, 20.0);
    assert_eq!(action(&down).kind, ActionKind::Pressed);
    ack(&mut c, 11, &down);
    layout(&mut c, 12, id, vec![hit(id, "control", 200.0)]);
    let up = touch(&mut c, 20, 1, TouchPhase::Up, 20.0, 20.0);
    assert_eq!(action(&up).kind, ActionKind::Activated);
    let released = ack(&mut c, 21, &up);
    assert_eq!(action(&released).kind, ActionKind::Released);
    ack(&mut c, 22, &released);
    assert!(call(&touch(&mut c, 23, 1, TouchPhase::Up, 20.0, 20.0)).is_none());
}
#[test]
fn column_fallback_is_bounded_in_mm_and_allows_release_in_column() {
    let (mut c, _) = setup(WidgetKind::Button, true);
    assert!(call(&touch(&mut c, 10, 1, TouchPhase::Down, 20.0, 46.0)).is_none());
    let down = touch(&mut c, 11, 1, TouchPhase::Down, 20.0, 44.0);
    assert_eq!(action(&down).kind, ActionKind::Pressed);
    ack(&mut c, 12, &down);
    assert_eq!(
        action(&touch(&mut c, 20, 1, TouchPhase::Up, 20.0, 44.0)).kind,
        ActionKind::Activated
    );
}
#[test]
fn key_cancel_is_terminal_even_without_finite_coordinates() {
    let (mut c, _) = setup(WidgetKind::Key { key: SystemKey::F1 }, false);
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0, 20.0);
    assert!(down.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Pressed
    }));
    let cancelled = touch(&mut c, 2, 1, TouchPhase::Cancel, f32::NAN, f32::NAN);
    assert!(cancelled.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Released
    }));
    assert!(touch(&mut c, 3, 1, TouchPhase::Up, 20.0, 20.0).is_empty());
}
#[test]
fn removing_widget_from_layout_cancels_capture_and_releases_key() {
    let (mut c, id) = setup(WidgetKind::Key { key: SystemKey::F1 }, false);
    touch(&mut c, 1, 1, TouchPhase::Down, 20.0, 20.0);
    let generation = c.snapshot().surfaces[0].generation;
    let out = c.handle(
        Timestamp(2),
        Input::Layout {
            slots: vec![],
            presentation: None,
            surface: SurfaceId(1),
            generation,
            hit_map: vec![],
            representations: vec![],
            scrubbers: vec![],
        },
    );
    assert!(out.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Released
    }));
    assert!(c.snapshot().captures.is_empty());
    // A stale map from the disposed generation cannot resurrect the contact.
    layout(&mut c, 3, id, vec![hit(id, "control", 10.0)]);
    assert!(touch(&mut c, 4, 1, TouchPhase::Up, 20.0, 20.0).is_empty());
}
#[test]
fn layout_cancel_is_synthetic_and_reports_reason() {
    let (mut c, _) = setup(WidgetKind::Button, true);
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0, 20.0);
    ack(&mut c, 2, &down);
    let generation = c.snapshot().surfaces[0].generation;
    let out = c.handle(
        Timestamp(3),
        Input::Layout {
            slots: vec![],
            presentation: None,
            surface: SurfaceId(1),
            generation,
            hit_map: vec![],
            representations: vec![],
            scrubbers: vec![],
        },
    );
    let a = action(&out);
    assert_eq!(
        (a.kind, a.origin, a.cancel_reason),
        (
            ActionKind::Cancelled,
            Origin::Synthetic,
            Some(CancelReason::Layout)
        )
    );
}
#[test]
fn invalid_scrubber_geometry_is_rejected_without_poisoning_existing_layout() {
    let (mut c, id) = setup(WidgetKind::Scrubber, true);
    scrubber_layout(&mut c, id);
    let generation = c.snapshot().surfaces[0].generation;
    let out = c.handle(
        Timestamp(1),
        Input::Layout {
            slots: vec![],
            presentation: None,
            surface: SurfaceId(1),
            generation,
            hit_map: vec![hit(id, "control", 10.0)],
            representations: vec![],
            scrubbers: vec![ScrubberLayout {
                instance: id,
                widget: "control".parse().unwrap(),
                viewport: Rect {
                    x: 10.0,
                    y: 10.0,
                    width: 100.0,
                    height: 20.0,
                },
                content_width: f32::NAN,
                items: vec![],
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
    touch(&mut c, 10, 1, TouchPhase::Down, 100.0, 20.0);
    assert_eq!(
        action(&touch(&mut c, 20, 1, TouchPhase::Move, 50.0, 20.0)).value,
        Some(Scalar::Number(100.0))
    );
}
#[test]
fn surface_geometry_change_cancels_instead_of_mixing_coordinate_spaces() {
    let (mut c, _) = setup(WidgetKind::Key { key: SystemKey::F1 }, false);
    touch(&mut c, 1, 1, TouchPhase::Down, 20.0, 20.0);
    let mut changed = info(1);
    changed.px_per_mm = 10.0;
    let out = c.handle(Timestamp(2), Input::SurfaceChanged(changed));
    assert!(out.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Released
    }));
    assert!(c.snapshot().captures.is_empty());
    assert!(touch(&mut c, 3, 2, TouchPhase::Down, 20.0, 20.0).is_empty());
}
#[test]
fn late_layout_for_pre_resize_render_cannot_restore_obsolete_hit_map() {
    let (mut c, id) = setup(WidgetKind::Key { key: SystemKey::F1 }, false);
    let generation = c.snapshot().surfaces[0].generation;
    let mut changed = info(1);
    changed.width_px = 600;
    c.handle(Timestamp(1), Input::SurfaceChanged(changed));
    c.handle(
        Timestamp(2),
        Input::Layout {
            slots: vec![],
            presentation: None,
            surface: SurfaceId(1),
            generation,
            hit_map: vec![hit(id, "control", 10.0)],
            representations: vec![],
            scrubbers: vec![],
        },
    );
    assert!(touch(&mut c, 3, 1, TouchPhase::Down, 20.0, 20.0).is_empty());
    let out = c.handle(
        Timestamp(4),
        Input::FrameDone {
            surface: SurfaceId(1),
            generation,
        },
    );
    assert!(out.iter().any(|o| matches!(o, Output::Render { .. })));
}
#[test]
fn pending_profile_waits_for_queued_physical_actions_after_up() {
    let (mut c, id) = setup(WidgetKind::Button, true);
    let down = touch(&mut c, 10, 1, TouchPhase::Down, 20.0, 20.0);
    c.handle(
        Timestamp(11),
        Input::Fact {
            name: "app.id".into(),
            value: Some(FactValue::Text("other".into())),
        },
    );
    assert!(c.snapshot().surfaces[0].profile.pending.is_some());
    touch(&mut c, 20, 1, TouchPhase::Up, 20.0, 20.0);
    assert_eq!(c.snapshot().instances[0].id, id);
    let activate = ack(&mut c, 21, &down);
    assert_eq!(action(&activate).kind, ActionKind::Activated);
    let released = ack(&mut c, 22, &activate);
    assert_eq!(action(&released).kind, ActionKind::Released);
    ack(&mut c, 23, &released);
    assert_eq!(
        c.snapshot().surfaces[0]
            .profile
            .active
            .as_ref()
            .unwrap()
            .to_string(),
        "p/other"
    );
    assert!(c.snapshot().instances.is_empty());
}
#[test]
fn hardware_cancel_carries_physical_origin() {
    let (mut c, _) = setup(WidgetKind::Button, true);
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0, 20.0);
    ack(&mut c, 2, &down);
    let a = action(&touch(&mut c, 3, 1, TouchPhase::Cancel, 0.0, 0.0));
    assert_eq!(
        (a.kind, a.origin, a.cancel_reason),
        (
            ActionKind::Cancelled,
            Origin::Physical,
            Some(CancelReason::Hardware)
        )
    );
}
#[test]
fn stationary_long_press_uses_timer_and_suppresses_activation() {
    let (mut c, _) = setup(WidgetKind::Button, true);
    let down = touch(&mut c, 10, 1, TouchPhase::Down, 20.0, 20.0);
    let timer = down
        .iter()
        .find_map(|o| match o {
            Output::SetTimer {
                id,
                at: Timestamp(410),
            } => Some(*id),
            _ => None,
        })
        .expect("hold timer");
    ack(&mut c, 11, &down);
    let held = c.handle(Timestamp(410), Input::Timer { id: timer });
    assert_eq!(
        (action(&held).kind, action(&held).origin),
        (ActionKind::LongPressed, Origin::Physical)
    );
    ack(&mut c, 411, &held);
    let up = touch(&mut c, 420, 1, TouchPhase::Up, 20.0, 20.0);
    assert_eq!(action(&up).kind, ActionKind::Released);
    assert!(c.snapshot().captures.is_empty());
}
#[test]
fn slider_emits_latest_value_once_per_frame_and_commits_final_value_before_activation() {
    let (mut c, _) = setup(
        WidgetKind::Slider {
            minimum: -10.0,
            maximum: 10.0,
        },
        true,
    );
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 10.0, 20.0);
    let value = ack(&mut c, 2, &down);
    assert_eq!(action(&value).value, Some(Scalar::Number(-10.0)));
    ack(&mut c, 3, &value);
    assert!(call(&touch(&mut c, 4, 1, TouchPhase::Move, 35.0, 20.0)).is_none());
    assert!(call(&touch(&mut c, 5, 1, TouchPhase::Move, 60.0, 20.0)).is_none());
    let generation = c.snapshot().surfaces[0].generation;
    let frame = c.handle(
        Timestamp(6),
        Input::FrameDone {
            surface: SurfaceId(1),
            generation,
        },
    );
    assert_eq!(action(&frame).value, Some(Scalar::Number(0.0)));
    ack(&mut c, 7, &frame);
    let up = touch(&mut c, 8, 1, TouchPhase::Up, 200.0, 20.0);
    assert_eq!(action(&up).value, Some(Scalar::Number(10.0)));
    let activated = ack(&mut c, 9, &up);
    assert_eq!(action(&activated).kind, ActionKind::Activated);
}
fn scrubber_layout(c: &mut Core, id: InstanceId) {
    let generation = c.snapshot().surfaces[0].generation;
    c.handle(
        Timestamp(0),
        Input::Layout {
            slots: vec![],
            presentation: None,
            surface: SurfaceId(1),
            generation,
            hit_map: vec![hit(id, "control", 10.0)],
            representations: vec![],
            scrubbers: vec![ScrubberLayout {
                instance: id,
                widget: "control".parse().unwrap(),
                viewport: Rect {
                    x: 10.0,
                    y: 10.0,
                    width: 100.0,
                    height: 20.0,
                },
                content_width: 300.0,
                items: [("a", 0.0), ("b", 100.0), ("c", 200.0)]
                    .into_iter()
                    .map(|(key, x)| ScrubberItemGeometry {
                        record_key: key.into(),
                        rect: Rect {
                            x,
                            y: 0.0,
                            width: 100.0,
                            height: 20.0,
                        },
                    })
                    .collect(),
            }],
        },
    );
}
#[test]
fn scrubber_scrolls_content_and_selects_stable_record_key_on_release() {
    let (mut c, id) = setup(WidgetKind::Scrubber, true);
    scrubber_layout(&mut c, id);
    // Scrubbers do not send Pressed: only scrolling and selection have handlers.
    assert!(call(&touch(&mut c, 10, 1, TouchPhase::Down, 100.0, 20.0)).is_none());
    let moved = touch(&mut c, 20, 1, TouchPhase::Move, 50.0, 20.0);
    assert_eq!(action(&moved).value, Some(Scalar::Number(100.0))); // 50 px / 0.5 px/u
    ack(&mut c, 21, &moved);
    let up = touch(&mut c, 30, 1, TouchPhase::Up, 50.0, 20.0);
    let selected = action(&up);
    assert_eq!(
        (selected.kind, selected.record_key, selected.value),
        (ActionKind::Selected, Some("a".into()), None)
    );
    let interaction = &c.snapshot().surfaces[0].scene.items[0].interactions[0];
    assert_eq!(interaction.scrubber.offset_px, 50.0);
    assert_eq!(interaction.scrubber.selected.as_deref(), Some("a"));
}
#[test]
fn scrubber_cancel_restores_previous_offset_and_selection() {
    let (mut c, id) = setup(WidgetKind::Scrubber, true);
    scrubber_layout(&mut c, id);
    touch(&mut c, 10, 1, TouchPhase::Down, 100.0, 20.0);
    let moved = touch(&mut c, 20, 1, TouchPhase::Move, 20.0, 20.0);
    ack(&mut c, 21, &moved);
    touch(&mut c, 22, 1, TouchPhase::Cancel, 0.0, 0.0);
    let interaction = &c.snapshot().surfaces[0].scene.items[0].interactions[0];
    assert_eq!(interaction.scrubber, ScrubberState::default());
    assert!(c.snapshot().captures.is_empty());
}
#[test]
fn same_contact_id_on_two_surfaces_keeps_separate_owners() {
    let (mut c, _) = setup(WidgetKind::Key { key: SystemKey::F1 }, false);
    c.handle(Timestamp(0), Input::SurfaceAdded(info(2)));
    let snapshot = c.snapshot();
    let second = &snapshot.surfaces[1];
    let id = second.scene.items[0].instance;
    c.handle(
        Timestamp(0),
        Input::Layout {
            slots: vec![],
            presentation: None,
            surface: SurfaceId(2),
            generation: second.generation,
            hit_map: vec![hit(id, "control", 10.0)],
            representations: vec![],
            scrubbers: vec![],
        },
    );
    touch(&mut c, 1, 1, TouchPhase::Down, 20.0, 20.0);
    let other_down = c.handle(
        Timestamp(1),
        Input::Touch {
            surface: SurfaceId(2),
            contact: ContactId::new(1).unwrap(),
            phase: TouchPhase::Down,
            x: 20.0,
            y: 20.0,
        },
    );
    assert!(other_down.contains(&Output::SendKey {
        surface: SurfaceId(2),
        key: SystemKey::F1,
        phase: KeyPhase::Pressed
    }));
    c.handle(
        Timestamp(2),
        Input::Revoked {
            surface: SurfaceId(1),
        },
    );
    assert_eq!(c.snapshot().captures.len(), 1);
    assert_eq!(c.snapshot().captures[0].surface, SurfaceId(2));
    let up = c.handle(
        Timestamp(3),
        Input::Touch {
            surface: SurfaceId(2),
            contact: ContactId::new(1).unwrap(),
            phase: TouchPhase::Up,
            x: 300.0,
            y: 20.0,
        },
    );
    assert!(up.contains(&Output::SendKey {
        surface: SurfaceId(2),
        key: SystemKey::F1,
        phase: KeyPhase::Released
    }));
    assert!(c.snapshot().captures.is_empty());
}
#[test]
fn shared_key_releases_only_after_last_finger_and_duplicate_down_does_not_replace_owner() {
    let (mut c, _) = setup(WidgetKind::Key { key: SystemKey::F1 }, false);
    touch(&mut c, 1, 1, TouchPhase::Down, 20.0, 20.0);
    assert!(touch(&mut c, 2, 1, TouchPhase::Down, 300.0, 20.0).is_empty());
    touch(&mut c, 3, 2, TouchPhase::Down, 20.0, 20.0);
    assert_eq!(c.snapshot().captures.len(), 2);
    let first = touch(&mut c, 4, 1, TouchPhase::Up, 300.0, 20.0);
    assert!(!first.iter().any(|o| matches!(
        o,
        Output::SendKey {
            phase: KeyPhase::Released,
            ..
        }
    )));
    let last = touch(&mut c, 5, 2, TouchPhase::Up, 300.0, 20.0);
    assert!(last.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Released
    }));
}
#[test]
fn forced_profile_deadline_releases_keys_and_never_waits_for_unfinished_logic() {
    for kind in [WidgetKind::Button, WidgetKind::Key { key: SystemKey::F1 }] {
        let logic = kind == WidgetKind::Button;
        let (mut c, _) = setup(kind, logic);
        let down = touch(&mut c, 10, 1, TouchPhase::Down, 20.0, 20.0);
        let out = c.handle(
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
        // A late wakeup on the advertised nearest timer processes every due job.
        let timer = down
            .iter()
            .chain(&out)
            .find_map(|o| match o {
                Output::SetTimer { id, .. } => Some(*id),
                _ => None,
            })
            .unwrap();
        let expired = c.handle(Timestamp(1511), Input::Timer { id: timer });
        assert!(c.snapshot().captures.is_empty());
        assert!(c.snapshot().instances.is_empty());
        if !logic {
            assert!(expired.contains(&Output::SendKey {
                surface: SurfaceId(1),
                key: SystemKey::F1,
                phase: KeyPhase::Released
            }));
        }
    }
}
#[test]
fn call_fault_cancels_its_capture_and_late_up_never_activates() {
    let (mut c, id) = setup(WidgetKind::Button, true);
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0, 20.0);
    let (_, call_id, _) = call(&down).unwrap();
    c.handle(
        Timestamp(2),
        Input::LogicFault {
            instance: id,
            call: call_id,
            reason: "trap".into(),
        },
    );
    assert!(c.snapshot().captures.is_empty());
    assert!(call(&touch(&mut c, 3, 1, TouchPhase::Up, 20.0, 20.0)).is_none());
}
#[test]
fn logic_hiding_pressed_widget_cancels_it_before_late_release() {
    let mut cat = catalog(WidgetKind::Button, true);
    cat.sets[0].widgets[0]
        .state_bindings
        .insert(StateBinding::Visible, "visible".into());
    let mut c = Core::new(CoreConfig::default());
    c.handle(Timestamp(0), Input::Catalog(cat));
    c.handle(Timestamp(0), Input::SurfaceAdded(info(1)));
    let id = c.snapshot().instances[0].id;
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
            instance: id,
            description: LogicDescription {
                actions,
                state: vec![StateField {
                    name: "visible".into(),
                    kind: FieldKind::Scalar(ScalarKind::Boolean),
                    persist: false,
                }],
            },
        },
    );
    let shown = ack(&mut c, 0, &init);
    ack(&mut c, 0, &shown);
    layout(&mut c, 0, id, vec![hit(id, "control", 10.0)]);
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 20.0, 20.0);
    let (_, call, _) = call(&down).unwrap();
    let out = c.handle(
        Timestamp(2),
        Input::LogicUpdate {
            instance: id,
            call,
            update: LogicUpdate {
                state: vec![("visible".into(), StateValue::Scalar(Scalar::Boolean(false)))],
                ..LogicUpdate::default()
            },
        },
    );
    assert_eq!(action(&out).cancel_reason, Some(CancelReason::Hidden));
    assert!(c.snapshot().captures.is_empty());
    assert!(touch(&mut c, 3, 1, TouchPhase::Up, 20.0, 20.0).is_empty());
}
#[test]
fn slider_only_one_finger_can_drive_same_widget_in_a_frame() {
    let (mut c, _) = setup(
        WidgetKind::Slider {
            minimum: 0.0,
            maximum: 1.0,
        },
        true,
    );
    let down = touch(&mut c, 1, 1, TouchPhase::Down, 10.0, 20.0);
    let value = ack(&mut c, 2, &down);
    ack(&mut c, 3, &value);
    assert!(call(&touch(&mut c, 4, 2, TouchPhase::Down, 100.0, 20.0)).is_none());
    assert_eq!(c.snapshot().captures.len(), 1);
}
#[test]
fn key_without_injection_or_desktop_fallback_cannot_capture() {
    let mut c = Core::new(CoreConfig::default());
    c.handle(
        Timestamp(0),
        Input::Catalog(catalog(WidgetKind::Key { key: SystemKey::F1 }, false)),
    );
    let mut surface = info(1);
    surface.caps.inject_keys = false;
    c.handle(Timestamp(0), Input::SurfaceAdded(surface));
    let id = c.snapshot().instances[0].id;
    layout(&mut c, 0, id, vec![hit(id, "control", 10.0)]);
    touch(&mut c, 1, 1, TouchPhase::Down, 20.0, 20.0);
    assert!(c.snapshot().captures.is_empty());
    c.handle(
        Timestamp(2),
        Input::DesktopCapabilities(DesktopCapabilities { key: true }),
    );
    let down = touch(&mut c, 3, 1, TouchPhase::Down, 20.0, 20.0);
    assert!(down.contains(&Output::DesktopKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Pressed
    }));
    let up = touch(&mut c, 4, 1, TouchPhase::Up, 20.0, 20.0);
    assert!(up.contains(&Output::DesktopKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Released
    }));
}
#[test]
fn profile_deadline_discards_physical_work_even_when_instance_is_shared() {
    let (mut c, id) = setup(WidgetKind::Button, true);
    let mut cat = catalog(WidgetKind::Button, true);
    cat.profiles[1].slots = cat.profiles[0].slots.clone();
    c.handle(Timestamp(0), Input::Catalog(cat));
    let down = touch(&mut c, 10, 1, TouchPhase::Down, 20.0, 20.0);
    c.handle(
        Timestamp(11),
        Input::Fact {
            name: "app.id".into(),
            value: Some(FactValue::Text("other".into())),
        },
    );
    let up = touch(&mut c, 20, 1, TouchPhase::Up, 20.0, 20.0);
    let timer = up
        .iter()
        .find_map(|o| match o {
            Output::SetTimer {
                id,
                at: Timestamp(1511),
            } => Some(*id),
            _ => None,
        })
        .unwrap();
    c.handle(Timestamp(1511), Input::Timer { id: timer });
    assert_eq!(c.snapshot().instances[0].id, id);
    let (_, call_id, _) = call(&down).unwrap();
    let completed = c.handle(
        Timestamp(1512),
        Input::LogicUpdate {
            instance: id,
            call: call_id,
            update: LogicUpdate {
                state: vec![("level".into(), StateValue::Scalar(Scalar::Number(9.0)))],
                ..LogicUpdate::default()
            },
        },
    );
    assert!(c.snapshot().instances[0].state.is_empty());
    assert!(call(&completed).is_none());
}
#[test]
fn native_teardown_releases_key_capture_and_rejects_new_touches() {
    let mut c = Core::new(CoreConfig::default());
    c.handle(
        Timestamp(0),
        Input::Catalog(catalog(WidgetKind::Key { key: SystemKey::F1 }, false)),
    );
    let mut surface = info(1);
    surface.caps.modes = true;
    c.handle(Timestamp(0), Input::SurfaceAdded(surface));
    c.handle(
        Timestamp(0),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Display,
        },
    );
    let id = c.snapshot().instances[0].id;
    layout(&mut c, 0, id, vec![hit(id, "control", 10.0)]);
    touch(&mut c, 1, 1, TouchPhase::Down, 20.0, 20.0);
    let out = c.handle(
        Timestamp(2),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Native,
        },
    );
    assert!(out.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Released
    }));
    assert!(c.snapshot().captures.is_empty());
    assert!(!out.iter().any(|o| matches!(o, Output::Render { .. })));
    assert!(touch(&mut c, 3, 1, TouchPhase::Down, 20.0, 20.0).is_empty());
}
