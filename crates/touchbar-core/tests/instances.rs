use touchbar_core::*;
use touchbar_model::*;

fn catalog() -> Catalog {
    let mut set = SetSpec::default_for("p/button".parse().unwrap());
    set.logic = Some("p".parse().unwrap());
    set.logic_config = "button-config".into();
    let mut widget = WidgetSpec::new("button".parse().unwrap(), WidgetKind::Button);
    widget
        .handlers
        .insert(ActionKind::Activated, "activate".into());
    widget
        .state_bindings
        .insert(StateBinding::Text, "caption".into());
    set.widgets.push(widget);
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
fn info(id: u64, modes: bool) -> SurfaceInfo {
    SurfaceInfo {
        id: SurfaceId(id),
        name: format!("headless:{id}").parse().unwrap(),
        kind: SurfaceKind::Headless,
        width_px: 1000,
        height_px: 100,
        px_per_mm: 5.0,
        caps: SurfaceCaps {
            modes,
            ..SurfaceCaps::default()
        },
    }
}
fn description() -> LogicDescription {
    LogicDescription {
        actions: vec![(
            "activate".into(),
            ActionDescription {
                kinds: vec![ActionKind::Activated],
                value: None,
            },
        )],
        state: vec![
            StateField {
                name: "caption".into(),
                kind: FieldKind::Scalar(ScalarKind::Text),
                persist: false,
            },
            StateField {
                name: "level".into(),
                kind: FieldKind::Scalar(ScalarKind::Number),
                persist: false,
            },
            StateField {
                name: "rows".into(),
                kind: FieldKind::Records(vec![("id".into(), ScalarKind::Text)]),
                persist: false,
            },
        ],
    }
}
fn setup(modes: bool) -> (Core, InstanceId) {
    let mut c = Core::new(CoreConfig::default());
    c.handle(Timestamp(0), Input::Catalog(catalog()));
    let out = c.handle(Timestamp(0), Input::SurfaceAdded(info(1, modes)));
    let id = out
        .iter()
        .find_map(|o| {
            if let Output::CreateInstance { instance, .. } = o {
                Some(*instance)
            } else {
                None
            }
        })
        .unwrap();
    (c, id)
}
fn invocation(out: &[Output]) -> (CallId, LogicCall) {
    out.iter()
        .find_map(|o| {
            if let Output::CallLogic {
                call, invocation, ..
            } = o
            {
                Some((*call, invocation.clone()))
            } else {
                None
            }
        })
        .unwrap()
}
fn init(c: &mut Core, id: InstanceId) -> CallId {
    let (call, invocation) = invocation(&c.handle(
        Timestamp(0),
        Input::LogicDescribed {
            persisted: vec![],
            instance: id,
            description: description(),
        },
    ));
    assert_eq!(
        invocation,
        LogicCall::Init {
            config: "button-config".into(),
            persisted: vec![]
        }
    );
    call
}
fn ack(c: &mut Core, id: InstanceId, call: CallId, now: u64, state: StatePatch) -> Vec<Output> {
    c.handle(
        Timestamp(now),
        Input::LogicUpdate {
            instance: id,
            call,
            update: LogicUpdate {
                state,
                ..LogicUpdate::default()
            },
        },
    )
}
fn text(s: &str) -> StateValue {
    StateValue::Scalar(Scalar::Text(s.into()))
}
fn instance(c: &Core, id: InstanceId) -> InstanceSnapshot {
    c.snapshot()
        .instances
        .into_iter()
        .find(|i| i.id == id)
        .unwrap()
}
fn timer(out: &[Output]) -> (TimerId, Timestamp) {
    out.iter()
        .find_map(|o| {
            if let Output::SetTimer { id, at } = o {
                Some((*id, *at))
            } else {
                None
            }
        })
        .unwrap()
}

#[test]
fn init_is_correlated_and_success_applies_state_before_shown() {
    let (mut c, id) = setup(false);
    assert_eq!(instance(&c, id).status, InstanceStatus::Loading);
    let call = init(&mut c, id);
    assert_eq!(instance(&c, id).status, InstanceStatus::Loading);
    assert!(
        ack(
            &mut c,
            id,
            CallId(call.0 + 99),
            1,
            vec![("caption".into(), text("wrong"))]
        )
        .is_empty()
    );
    let out = ack(&mut c, id, call, 2, vec![("caption".into(), text("ready"))]);
    assert_eq!(invocation(&out).1, LogicCall::Handle(LogicEvent::Shown));
    assert_eq!(instance(&c, id).status, InstanceStatus::Visible);
    assert_eq!(
        instance(&c, id).state,
        vec![("caption".into(), text("ready"))]
    );
    assert_eq!(
        c.snapshot().surfaces[0].scene.items[0].state,
        vec![("caption".into(), text("ready"))]
    );
}
#[test]
fn native_init_finishes_hidden_then_display_shows_and_native_hides() {
    let (mut c, id) = setup(true);
    let call = init(&mut c, id);
    assert!(
        !ack(&mut c, id, call, 1, vec![])
            .iter()
            .any(|o| matches!(o, Output::CallLogic { .. }))
    );
    assert_eq!(instance(&c, id).status, InstanceStatus::Hidden);
    let out = c.handle(
        Timestamp(2),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Display,
        },
    );
    let (shown, event) = invocation(&out);
    assert_eq!(event, LogicCall::Handle(LogicEvent::Shown));
    ack(&mut c, id, shown, 3, vec![]);
    let out = c.handle(
        Timestamp(4),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Native,
        },
    );
    assert_eq!(invocation(&out).1, LogicCall::Handle(LogicEvent::Hidden));
    assert_eq!(instance(&c, id).status, InstanceStatus::Hidden);
    assert!(!out.iter().any(|o| matches!(o, Output::Render { .. })));
}
#[test]
fn hide_while_shown_is_running_waits_and_handle_error_advances_queue() {
    let (mut c, id) = setup(true);
    let call = init(&mut c, id);
    ack(&mut c, id, call, 1, vec![]);
    let (shown, _) = invocation(&c.handle(
        Timestamp(2),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Display,
        },
    ));
    let out = c.handle(
        Timestamp(3),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Native,
        },
    );
    assert!(!out.iter().any(|o| matches!(o, Output::CallLogic { .. })));
    assert_eq!(instance(&c, id).queue_len, 1);
    let out = c.handle(
        Timestamp(4),
        Input::LogicError {
            instance: id,
            call: shown,
            message: "ordinary error".into(),
        },
    );
    assert_eq!(invocation(&out).1, LogicCall::Handle(LogicEvent::Hidden));
    assert_eq!(instance(&c, id).status, InstanceStatus::Hidden);
    assert!(
        !out.iter()
            .any(|o| matches!(o, Output::DisposeInstance { .. } | Output::SetTimer { .. }))
    );
}
#[test]
fn init_error_faults_and_retries_with_a_new_generation() {
    let (mut c, id) = setup(false);
    let call = init(&mut c, id);
    let out = c.handle(
        Timestamp(10),
        Input::LogicError {
            instance: id,
            call,
            message: "init failed".into(),
        },
    );
    assert_eq!(instance(&c, id).status, InstanceStatus::Faulted);
    assert!(out.contains(&Output::DisposeInstance { instance: id }));
    let (timer, at) = timer(&out);
    assert_eq!(at, Timestamp(1010));
    assert!(
        c.handle(Timestamp(1009), Input::Timer { id: timer })
            .iter()
            .all(|o| !matches!(o, Output::CreateInstance { .. }))
    );
    let out = c.handle(at, Input::Timer { id: timer });
    let new = out
        .iter()
        .find_map(|o| {
            if let Output::CreateInstance { instance, .. } = o {
                Some(*instance)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(new.number, id.number);
    assert_eq!(new.generation, Generation(id.generation.0 + 1));
    assert!(
        ack(
            &mut c,
            id,
            call,
            1011,
            vec![("caption".into(), text("stale"))]
        )
        .is_empty()
    );
    assert!(instance(&c, new).state.is_empty());
    assert_eq!(c.snapshot().surfaces[0].scene.items[0].instance, new);
}
#[test]
fn load_failure_before_describe_uses_backoff_and_stale_instance_fault_is_ignored() {
    let (mut c, id) = setup(false);
    let out = c.handle(
        Timestamp(1),
        Input::InstanceFault {
            instance: id,
            reason: "load failed".into(),
        },
    );
    let (timer, at) = timer(&out);
    let out = c.handle(at, Input::Timer { id: timer });
    let new = c.snapshot().instances[0].id;
    assert!(out.contains(&Output::CreateInstance {
        persist_key: PersistKey {
            plugin: "p".parse().unwrap(),
            surface: "headless:1".parse().unwrap(),
            scope: PersistScope::ProfileSlot("main".parse().unwrap())
        },
        instance: new,
        surface: SurfaceId(1),
        plugin: "p".parse().unwrap(),
        set: "p/button".parse().unwrap()
    }));
    assert!(
        c.handle(
            at,
            Input::InstanceFault {
                instance: id,
                reason: "stale failure".into()
            }
        )
        .is_empty()
    );
    assert_eq!(instance(&c, new).status, InstanceStatus::Loading);
}
#[test]
fn repeated_init_failures_back_off_one_two_four_seconds_then_cap_at_sixty() {
    let (mut c, mut id) = setup(false);
    let mut now = 0;
    for delay in [1000, 2000, 4000, 8000, 16000, 32000, 60000, 60000] {
        let out = c.handle(
            Timestamp(now),
            Input::InstanceFault {
                instance: id,
                reason: "load".into(),
            },
        );
        let (timer, at) = timer(&out);
        assert_eq!(at, Timestamp(now + delay));
        now += delay;
        c.handle(Timestamp(now), Input::Timer { id: timer });
        id = c.snapshot().instances[0].id;
    }
}
#[test]
fn description_mismatch_faults_without_retry_until_catalog_reload() {
    for bad in 0..3 {
        let (mut c, id) = setup(false);
        let mut desc = description();
        match bad {
            0 => desc.actions.clear(),
            1 => desc.state[0].kind = FieldKind::Scalar(ScalarKind::Boolean),
            _ => desc.actions[0].1.kinds.clear(),
        };
        let out = c.handle(
            Timestamp(0),
            Input::LogicDescribed {
                persisted: vec![],
                instance: id,
                description: desc,
            },
        );
        assert_eq!(instance(&c, id).status, InstanceStatus::Faulted);
        assert!(instance(&c, id).retry_at.is_none());
        assert!(
            !out.iter()
                .any(|o| matches!(o, Output::CallLogic { .. } | Output::SetTimer { .. }))
        );
        let out = c.handle(Timestamp(5000), Input::Catalog(catalog()));
        assert!(
            out.iter()
                .any(|o| matches!(o, Output::CreateInstance { .. }))
        );
        assert_eq!(c.snapshot().instances[0].status, InstanceStatus::Loading);
    }
}
#[test]
fn invalid_update_fields_are_dropped_individually_and_valid_patch_is_retained() {
    let (mut c, id) = setup(false);
    let call = init(&mut c, id);
    let out = ack(
        &mut c,
        id,
        call,
        1,
        vec![
            ("caption".into(), text("valid")),
            ("level".into(), StateValue::Scalar(Scalar::Number(f64::NAN))),
            ("unknown".into(), text("bad")),
        ],
    );
    assert_eq!(
        instance(&c, id).state,
        vec![("caption".into(), text("valid"))]
    );
    assert_eq!(instance(&c, id).status, InstanceStatus::Visible);
    assert!(
        out.iter()
            .filter(|o| matches!(
                o,
                Output::Log {
                    level: LogLevel::Warn,
                    ..
                }
            ))
            .count()
            >= 2
    );
}
#[test]
fn overlong_text_and_too_many_records_are_rejected_without_losing_valid_fields() {
    let (mut c, id) = setup(false);
    let call = init(&mut c, id);
    let out = ack(
        &mut c,
        id,
        call,
        1,
        vec![
            ("caption".into(), text(&"x".repeat(4097))),
            (
                "rows".into(),
                StateValue::Records(vec![
                    [("id".into(), Scalar::Text("a".into()))]
                        .into_iter()
                        .collect();
                    1001
                ]),
            ),
            ("level".into(), StateValue::Scalar(Scalar::Number(0.5))),
        ],
    );
    assert_eq!(
        instance(&c, id).state,
        vec![("level".into(), StateValue::Scalar(Scalar::Number(0.5)))]
    );
    assert!(out.iter().any(|o| matches!(
        o,
        Output::Log {
            level: LogLevel::Warn,
            ..
        }
    )));
}
#[test]
fn fault_is_isolated_and_surface_removal_cancels_retry() {
    let (mut c, id) = setup(false);
    c.handle(Timestamp(0), Input::SurfaceAdded(info(2, false)));
    let other = c
        .snapshot()
        .instances
        .iter()
        .find(|i| i.surface == SurfaceId(2))
        .unwrap()
        .id;
    let out = c.handle(
        Timestamp(1),
        Input::InstanceFault {
            instance: id,
            reason: "broken".into(),
        },
    );
    let (timer, at) = timer(&out);
    assert_eq!(instance(&c, other).status, InstanceStatus::Loading);
    let out = c.handle(Timestamp(2), Input::SurfaceRemoved(SurfaceId(1)));
    assert!(out.contains(&Output::ClearTimer { id: timer }));
    assert!(c.handle(at, Input::Timer { id: timer }).is_empty());
    assert_eq!(c.snapshot().instances.len(), 1);
}
#[test]
fn duplicate_describe_and_duplicate_response_never_issue_another_init_or_shown() {
    let (mut c, id) = setup(false);
    let call = init(&mut c, id);
    assert!(
        c.handle(
            Timestamp(0),
            Input::LogicDescribed {
                persisted: vec![],
                instance: id,
                description: description()
            }
        )
        .is_empty()
    );
    ack(&mut c, id, call, 1, vec![]);
    assert!(ack(&mut c, id, call, 2, vec![]).is_empty());
}
#[test]
fn ready_instance_call_fault_releases_runtime_and_stale_call_fault_is_ignored() {
    let (mut c, id) = setup(false);
    let init = init(&mut c, id);
    let (shown, _) = invocation(&ack(&mut c, id, init, 1, vec![]));
    assert!(
        c.handle(
            Timestamp(2),
            Input::LogicFault {
                instance: id,
                call: init,
                reason: "late trap".into()
            }
        )
        .is_empty()
    );
    let out = c.handle(
        Timestamp(3),
        Input::LogicFault {
            instance: id,
            call: shown,
            reason: "trap".into(),
        },
    );
    assert!(out.contains(&Output::DisposeInstance { instance: id }));
    assert_eq!(instance(&c, id).status, InstanceStatus::Faulted);
    assert_eq!(timer(&out).1, Timestamp(1003));
}
#[test]
fn removing_instance_with_call_in_flight_disposes_and_discards_late_results() {
    let (mut c, id) = setup(false);
    let call = init(&mut c, id);
    let out = c.handle(Timestamp(1), Input::SurfaceRemoved(SurfaceId(1)));
    assert!(out.contains(&Output::DisposeInstance { instance: id }));
    assert!(c.snapshot().instances.is_empty());
    assert!(ack(&mut c, id, call, 2, vec![("caption".into(), text("late"))]).is_empty());
}
#[test]
fn patch_updates_existing_field_without_dropping_other_state() {
    let (mut c, id) = setup(false);
    let call = init(&mut c, id);
    let (shown, _) = invocation(&ack(
        &mut c,
        id,
        call,
        1,
        vec![
            ("caption".into(), text("old")),
            ("level".into(), StateValue::Scalar(Scalar::Number(0.5))),
        ],
    ));
    ack(&mut c, id, shown, 2, vec![("caption".into(), text("new"))]);
    assert_eq!(
        instance(&c, id).state,
        vec![
            ("caption".into(), text("new")),
            ("level".into(), StateValue::Scalar(Scalar::Number(0.5)))
        ]
    );
}
#[test]
fn healthy_catalog_reload_preserves_visible_status_without_reinitializing() {
    let (mut c, id) = setup(false);
    let call = init(&mut c, id);
    let (shown, _) = invocation(&ack(&mut c, id, call, 1, vec![]));
    ack(&mut c, id, shown, 2, vec![]);
    assert!(c.handle(Timestamp(3), Input::Catalog(catalog())).is_empty());
    assert_eq!(instance(&c, id).status, InstanceStatus::Visible);
}
#[test]
fn action_payload_schema_must_match_physical_value_changed_events() {
    for value in [None, Some(ScalarKind::Text), Some(ScalarKind::Boolean)] {
        let mut catalog = catalog();
        catalog.sets[0].widgets[0].handlers.clear();
        catalog.sets[0].widgets[0]
            .handlers
            .insert(ActionKind::ValueChanged, "adjust".into());
        let mut c = Core::new(CoreConfig::default());
        c.handle(Timestamp(0), Input::Catalog(catalog));
        let out = c.handle(Timestamp(0), Input::SurfaceAdded(info(1, false)));
        let id = out
            .iter()
            .find_map(|o| {
                if let Output::CreateInstance { instance, .. } = o {
                    Some(*instance)
                } else {
                    None
                }
            })
            .unwrap();
        let mut desc = description();
        desc.actions = vec![(
            "adjust".into(),
            ActionDescription {
                kinds: vec![ActionKind::ValueChanged],
                value,
            },
        )];
        let out = c.handle(
            Timestamp(0),
            Input::LogicDescribed {
                persisted: vec![],
                instance: id,
                description: desc,
            },
        );
        assert_eq!(instance(&c, id).status, InstanceStatus::Faulted);
        assert!(!out.iter().any(|o| matches!(o, Output::CallLogic { .. })));
    }
}
#[test]
fn repeated_children_resolve_record_bindings_in_the_parent_records_schema() {
    let mut catalog = catalog();
    let mut parent = WidgetSpec::new("records".parse().unwrap(), WidgetKind::Noninteractive);
    parent
        .state_bindings
        .insert(StateBinding::Records, "rows".into());
    let mut child = WidgetSpec::new("child".parse().unwrap(), WidgetKind::Button);
    child
        .state_bindings
        .insert(StateBinding::Text, "record.id".into());
    parent.children.push(child);
    catalog.sets[0].widgets = vec![parent];
    let mut c = Core::new(CoreConfig::default());
    c.handle(Timestamp(0), Input::Catalog(catalog));
    c.handle(Timestamp(0), Input::SurfaceAdded(info(1, false)));
    let id = c.snapshot().instances[0].id;
    let out = c.handle(
        Timestamp(0),
        Input::LogicDescribed {
            persisted: vec![],
            instance: id,
            description: description(),
        },
    );
    assert!(matches!(invocation(&out).1, LogicCall::Init { .. }));
}
#[test]
fn nonfinite_canvas_geometry_is_dropped_while_valid_state_is_applied() {
    let (mut c, id) = setup(false);
    let mut desc = description();
    desc.state.push(StateField {
        name: "canvas".into(),
        kind: FieldKind::Canvas,
        persist: false,
    });
    let (call, _) = invocation(&c.handle(
        Timestamp(0),
        Input::LogicDescribed {
            persisted: vec![],
            instance: id,
            description: desc,
        },
    ));
    let invalid = StateValue::Canvas(vec![CanvasCommand::FillRect {
        rect: Rect {
            x: f32::NAN,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        },
        radius: 1.0,
        paint: CanvasPaint {
            color: CanvasColor::Role(ColorRole::Accent),
            opacity: 1.0,
        },
    }]);
    let out = ack(
        &mut c,
        id,
        call,
        1,
        vec![
            ("canvas".into(), invalid),
            ("caption".into(), text("valid")),
        ],
    );
    assert_eq!(
        instance(&c, id).state,
        vec![("caption".into(), text("valid"))]
    );
    assert!(out.iter().any(|o| matches!(
        o,
        Output::Log {
            level: LogLevel::Warn,
            ..
        }
    )));
}
#[test]
fn duplicate_description_fields_and_unscoped_record_bindings_are_schema_faults() {
    for duplicate in [true, false] {
        let mut catalog = catalog();
        let mut desc = description();
        if duplicate {
            desc.state.push(desc.state[0].clone());
        } else {
            catalog.sets[0].widgets[0]
                .state_bindings
                .insert(StateBinding::Text, "record.id".into());
        }
        let mut c = Core::new(CoreConfig::default());
        c.handle(Timestamp(0), Input::Catalog(catalog));
        c.handle(Timestamp(0), Input::SurfaceAdded(info(1, false)));
        let id = c.snapshot().instances[0].id;
        c.handle(
            Timestamp(0),
            Input::LogicDescribed {
                persisted: vec![],
                instance: id,
                description: desc,
            },
        );
        assert_eq!(instance(&c, id).status, InstanceStatus::Faulted);
        assert!(instance(&c, id).retry_at.is_none());
    }
}
