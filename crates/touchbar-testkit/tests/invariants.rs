use touchbar_core::*;
use touchbar_model::*;
use touchbar_testkit::*;

fn ready() -> Harness {
    let mut h = Harness::new(CoreConfig::default());
    h.send(Input::Catalog(fixtures::controls_with_panel()));
    h.send(Input::SurfaceAdded(headless(1)));
    h
}
fn checker_at(h: &Harness) -> InvariantChecker {
    let mut checker = InvariantChecker::new(Core::new(CoreConfig::default()).snapshot());
    let mut core = Core::new(CoreConfig::default());
    for entry in &h.trace {
        assert_eq!(core.handle(entry.at, entry.input.clone()), entry.outputs);
        checker.observe(entry, &core.snapshot()).unwrap();
    }
    checker
}
#[test]
fn healthy_trace_and_terminal_keys_pass() {
    let mut h = ready();
    h.tap(SurfaceId(1), "microphone");
    h.send(Input::Key {
        source: KeySource::Surface(SurfaceId(1)),
        code: WatchKey::Fn,
        phase: KeyPhase::Pressed,
    });
    h.tap(SurfaceId(1), "f1");
    h.send(Input::SurfaceRemoved(SurfaceId(1)));
    checker_at(&h).finish().unwrap();
}
#[test]
fn checker_reports_first_violation_with_zero_based_input_index() {
    let h = ready();
    let mut checker = checker_at(&h);
    let mut after = h.snapshot();
    after.surfaces[0].mode = Mode::Native;
    let entry = TraceEntry {
        at: h.now(),
        input: Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Native,
        },
        outputs: vec![Output::Render {
            surface: SurfaceId(1),
            generation: after.surfaces[0].generation,
            scene: after.surfaces[0].scene.clone(),
        }],
    };
    let error = checker.observe(&entry, &after).unwrap_err();
    assert_eq!(error.invariant, Invariant::I11);
    assert_eq!(error.step, h.trace.len());
    assert_eq!(checker.observe(&entry, &after).unwrap_err(), error);
}
#[test]
fn checker_detects_retargeted_and_unfinished_capture() {
    let mut h = ready();
    h.touch(
        SurfaceId(1),
        1,
        TouchPhase::Down,
        h.hit(SurfaceId(1), "microphone"),
    );
    let mut checker = checker_at(&h);
    let mut after = h.snapshot();
    after.captures[0].target.widget = "volume".parse().unwrap();
    let entry = TraceEntry {
        at: h.now(),
        input: Input::Locale("C".into()),
        outputs: vec![],
    };
    assert_eq!(
        checker.observe(&entry, &after).unwrap_err().invariant,
        Invariant::I1
    );

    let mut checker = checker_at(&h);
    let entry = TraceEntry {
        at: h.now(),
        input: Input::Touch {
            surface: SurfaceId(1),
            contact: ContactId::new(1).unwrap(),
            phase: TouchPhase::Up,
            x: 0.0,
            y: 0.0,
        },
        outputs: vec![],
    };
    assert_eq!(
        checker
            .observe(&entry, &h.snapshot())
            .unwrap_err()
            .invariant,
        Invariant::I2
    );
}

fn fabricated(h: &Harness, input: Input) -> TraceEntry {
    TraceEntry {
        at: h.now(),
        input,
        outputs: vec![],
    }
}
#[test]
fn checker_detects_overdue_pending_and_panel_owner() {
    let h = ready();
    let mut after = h.snapshot();
    after.now = Timestamp(1501);
    after.surfaces[0].profile.pending = Some(PendingProfile {
        target: None,
        deadline: Timestamp(1500),
    });
    assert_eq!(
        checker_at(&h)
            .observe(&fabricated(&h, Input::Locale("C".into())), &after)
            .unwrap_err()
            .invariant,
        Invariant::I3
    );
    let mut h = ready();
    h.logic.register("controls/main", None, |_, call| {
        let mut update = LogicUpdate::default();
        if matches!(
            call,
            LogicCall::Handle(LogicEvent::Action(ActionEvent {
                kind: ActionKind::Activated,
                ..
            }))
        ) {
            update.presentation = Some(PresentationCommand::Open {
                panel: "devices".parse().unwrap(),
                placement: Placement::Full,
                lifecycle: PresentationLifecycle::Persistent,
            });
        }
        LogicReaction::Update(update)
    });
    h.tap(SurfaceId(1), "microphone");
    let mut after = h.snapshot();
    let owner = after.surfaces[0].presentation.as_ref().unwrap().owner;
    after
        .instances
        .iter_mut()
        .find(|i| i.id == owner)
        .unwrap()
        .status = InstanceStatus::Hidden;
    assert_eq!(
        checker_at(&h)
            .observe(&fabricated(&h, Input::Locale("C".into())), &after)
            .unwrap_err()
            .invariant,
        Invariant::I4
    );
}
#[test]
fn checker_detects_hidden_action_and_active_foreground_subscription() {
    let h = ready();
    let id = h.snapshot().instances[0].id;
    let mut after = h.snapshot();
    after.instances[0].status = InstanceStatus::Hidden;
    let mut entry = fabricated(&h, Input::Locale("C".into()));
    entry.outputs.push(Output::CallLogic {
        instance: id,
        call: CallId(999),
        invocation: LogicCall::Handle(LogicEvent::Action(ActionEvent {
            action: "microphone".into(),
            kind: ActionKind::Activated,
            value: None,
            record_key: None,
            origin: Origin::Physical,
            cancel_reason: None,
        })),
    });
    assert_eq!(
        checker_at(&h)
            .observe(&entry, &after)
            .unwrap_err()
            .invariant,
        Invariant::I5
    );
    entry.outputs.clear();
    after.subscriptions.push(SubscriptionSnapshot {
        owner: id,
        spec: SubscriptionSpec {
            tag: "ticks".into(),
            kind: SubscriptionKind::TimerInterval { ms: 32 },
            background: false,
        },
        active: Some(SubscriptionId {
            instance: id,
            number: 999,
        }),
        failures: 0,
        retry_at: None,
    });
    assert_eq!(
        checker_at(&h)
            .observe(&entry, &after)
            .unwrap_err()
            .invariant,
        Invariant::I5
    );
}
#[test]
fn checker_detects_ui_cancellation_of_committed_request() {
    let mut h = ready();
    h.logic = fixtures::controls_logic();
    h.effects.delay_ms = 1000;
    h.tap(SurfaceId(1), "microphone");
    assert_eq!(h.snapshot().requests.len(), 1);
    let mut after = h.snapshot();
    after.requests.clear();
    assert_eq!(
        checker_at(&h)
            .observe(&fabricated(&h, Input::SurfaceRemoved(SurfaceId(1))), &after)
            .unwrap_err()
            .invariant,
        Invariant::I6
    );
}
#[test]
fn checker_detects_dead_result_and_loading_event() {
    let h = ready();
    let mut entry = fabricated(&h, Input::Locale("C".into()));
    entry.outputs.push(Output::CallLogic {
        instance: InstanceId {
            number: 999,
            generation: Generation(1),
        },
        call: CallId(999),
        invocation: LogicCall::Handle(LogicEvent::EffectDone {
            tag: "write".into(),
            result: Ok(EffectResult::None),
        }),
    });
    assert_eq!(
        checker_at(&h)
            .observe(&entry, &h.snapshot())
            .unwrap_err()
            .invariant,
        Invariant::I7
    );
    // A command following Dispose is invalid even if the old generation was alive
    // at the beginning of the input.
    let id = h.snapshot().instances[0].id;
    entry.outputs = vec![
        Output::DisposeInstance { instance: id },
        Output::CallLogic {
            instance: id,
            call: CallId(999),
            invocation: LogicCall::Handle(LogicEvent::EffectDone {
                tag: "write".into(),
                result: Ok(EffectResult::None),
            }),
        },
    ];
    assert_eq!(
        checker_at(&h)
            .observe(&entry, &h.snapshot())
            .unwrap_err()
            .invariant,
        Invariant::I7
    );
    let mut h = ready();
    let id = h.snapshot().instances[0].id;
    let mut after = h.snapshot();
    after.instances[0].status = InstanceStatus::Loading;
    entry.outputs = vec![Output::CallLogic {
        instance: id,
        call: CallId(999),
        invocation: LogicCall::Handle(LogicEvent::Shown),
    }];
    // Make the initial observation agree about Loading: it is the event that violates I8.
    let mut checker = InvariantChecker::new(after.clone());
    assert_eq!(
        checker.observe(&entry, &after).unwrap_err().invariant,
        Invariant::I8
    );
    h.send(Input::SurfaceRemoved(SurfaceId(1)));
}
#[test]
fn checker_detects_missing_release_and_dead_lease_owner() {
    let mut h = ready();
    h.send(Input::Key {
        source: KeySource::Surface(SurfaceId(1)),
        code: WatchKey::Fn,
        phase: KeyPhase::Pressed,
    });
    h.touch(SurfaceId(1), 1, TouchPhase::Down, h.hit(SurfaceId(1), "f1"));
    let mut after = h.snapshot();
    after.captures.clear();
    after.key_leases.clear();
    let capture = h.snapshot().captures[0].clone();
    let mut entry = fabricated(
        &h,
        Input::Touch {
            surface: SurfaceId(1),
            contact: ContactId::new(1).unwrap(),
            phase: TouchPhase::Cancel,
            x: 0.0,
            y: 0.0,
        },
    );
    entry.outputs.push(Output::CaptureFinished {
        capture,
        completion: CaptureCompletion::Cancel(CancelReason::Hardware),
    });
    assert_eq!(
        checker_at(&h)
            .observe(&entry, &after)
            .unwrap_err()
            .invariant,
        Invariant::I9
    );
}
#[test]
fn checker_accepts_fault_with_preexisting_pending_profile_completion() {
    let dump = ReplayDump::from_json(include_str!(
        "regressions/fault_during_pending_completion.replay.json"
    ))
    .unwrap();
    let mut core = Core::restore(dump.checkpoint).unwrap();
    let mut checker = InvariantChecker::new(core.snapshot());
    for entry in dump.inputs {
        if matches!(entry.input, Input::LogicFault { .. }) {
            let before = core.snapshot();
            let pending = &before
                .surfaces
                .iter()
                .find(|s| s.info.id == SurfaceId(2))
                .unwrap()
                .profile;
            assert_eq!(pending.active, Some("controls/default".parse().unwrap()));
            assert_eq!(
                pending.pending.as_ref().unwrap().target,
                Some("controls/app1".parse().unwrap())
            );
            let old = before
                .instances
                .iter()
                .find(|i| i.surface == SurfaceId(2))
                .unwrap()
                .id;
            let mut neutral = core.clone();
            neutral.handle(entry.at, Input::Locale("test-0".into()));
            assert_eq!(core.handle(entry.at, entry.input.clone()), entry.outputs);
            for snapshot in [core.snapshot(), neutral.snapshot()] {
                assert!(!snapshot.instances.iter().any(|i| i.id == old));
                let profile = &snapshot
                    .surfaces
                    .iter()
                    .find(|s| s.info.id == SurfaceId(2))
                    .unwrap()
                    .profile;
                assert_eq!(profile.active, Some("controls/app1".parse().unwrap()));
                assert!(profile.pending.is_none());
            }
            checker.observe(&entry, &core.snapshot()).unwrap();
            return;
        }
        assert_eq!(core.handle(entry.at, entry.input.clone()), entry.outputs);
        checker.observe(&entry, &core.snapshot()).unwrap();
    }
    panic!("fixture must contain a fault");
}

#[test]
fn checker_rejects_live_instance_mutations_during_pending_completion() {
    let mut h = ready();
    h.send(Input::SurfaceAdded(headless(2)));
    let mut before = h.snapshot();
    let first = before.instances[0].id;
    let other = before
        .instances
        .iter()
        .find(|i| i.surface == SurfaceId(2))
        .unwrap()
        .id;
    let target: ProfileRef = "controls/next".parse().unwrap();
    before.surfaces[1].profile.pending = Some(PendingProfile {
        target: Some(target.clone()),
        deadline: before.now.after(1500),
    });
    let mut completed = before.clone();
    completed.surfaces[1].profile.active = Some(target.clone());
    completed.surfaces[1].profile.pending = None;
    completed.surfaces[1].scene.profile = Some(target);
    for change_state in [false, true] {
        let mut after = completed.clone();
        let instance = after.instances.iter_mut().find(|i| i.id == other).unwrap();
        if change_state {
            instance.state.push((
                "fabricated".into(),
                StateValue::Scalar(Scalar::Boolean(true)),
            ));
        } else {
            instance.status = InstanceStatus::Hidden;
        }
        let mut checker = InvariantChecker::new(before.clone());
        assert_eq!(
            checker
                .observe(
                    &fabricated(
                        &h,
                        Input::InstanceFault {
                            instance: first,
                            reason: "trap".into(),
                        }
                    ),
                    &after
                )
                .unwrap_err()
                .invariant,
            Invariant::I10
        );
    }
}

#[test]
fn checker_rejects_unjustified_removal_during_pending_completion() {
    let dump = ReplayDump::from_json(include_str!(
        "regressions/fault_during_pending_completion.replay.json"
    ))
    .unwrap();
    let mut core = Core::restore(dump.checkpoint).unwrap();
    let entry = dump
        .inputs
        .into_iter()
        .find(|entry| {
            if matches!(entry.input, Input::LogicFault { .. }) {
                true
            } else {
                assert_eq!(core.handle(entry.at, entry.input.clone()), entry.outputs);
                false
            }
        })
        .unwrap();
    let before = core.snapshot();
    assert_eq!(core.handle(entry.at, entry.input.clone()), entry.outputs);
    let after = core.snapshot();
    let old = before
        .instances
        .iter()
        .find(|i| i.surface == SurfaceId(2))
        .unwrap();
    for invalid in [
        "no pending",
        "unresolved",
        "wrong target",
        "no dispose",
        "layer",
        "retained placement",
    ] {
        let mut before = before.clone();
        let mut after = after.clone();
        let mut entry = entry.clone();
        match invalid {
            "no pending" => before.surfaces[1].profile.pending = None,
            "unresolved" => {
                after.surfaces[1].profile.pending = before.surfaces[1].profile.pending.clone()
            }
            "wrong target" => before.surfaces[1].profile.pending.as_mut().unwrap().target = None,
            "no dispose" => entry.outputs.retain(
                |o| !matches!(o, Output::DisposeInstance { instance } if *instance == old.id),
            ),
            "layer" => {
                before
                    .instances
                    .iter_mut()
                    .find(|i| i.id == old.id)
                    .unwrap()
                    .persist_key
                    .scope = PersistScope::Layer(old.set.clone())
            }
            "retained placement" => {
                let mut item = before.surfaces[1]
                    .scene
                    .items
                    .iter()
                    .find(|i| i.instance == old.id)
                    .unwrap()
                    .clone();
                item.instance = after.surfaces[1].scene.items[0].instance;
                after.surfaces[1].scene.items[0] = item;
            }
            _ => unreachable!(),
        }
        let mut checker = InvariantChecker::new(before);
        assert_eq!(
            checker.observe(&entry, &after).unwrap_err().invariant,
            Invariant::I10,
            "{invalid}"
        );
    }
}

#[test]
fn checker_detects_fault_mutating_another_instance_and_surface() {
    let mut h = ready();
    h.send(Input::SurfaceAdded(headless(2)));
    let mut after = h.snapshot();
    let first = after.instances[0].id;
    let other = after
        .instances
        .iter_mut()
        .find(|i| i.surface == SurfaceId(2))
        .unwrap();
    other.state.push((
        "fabricated".into(),
        StateValue::Scalar(Scalar::Boolean(true)),
    ));
    assert_eq!(
        checker_at(&h)
            .observe(
                &fabricated(
                    &h,
                    Input::InstanceFault {
                        instance: first,
                        reason: "trap".into()
                    }
                ),
                &after
            )
            .unwrap_err()
            .invariant,
        Invariant::I10
    );
    let mut after = h.snapshot();
    after.surfaces[1].layers.push(LayerSnapshot {
        set: "controls/fkeys".parse().unwrap(),
        latched: true,
        held: vec![],
    });
    assert_eq!(
        checker_at(&h)
            .observe(
                &fabricated(
                    &h,
                    Input::Mode {
                        surface: SurfaceId(1),
                        mode: Mode::Native
                    }
                ),
                &after
            )
            .unwrap_err()
            .invariant,
        Invariant::I12
    );
}

#[test]
fn checker_accepts_only_prequeued_cleanup_on_hidden_instance() {
    let h = ready();
    let mut before = h.snapshot();
    let id = before.instances[0].id;
    before.instances[0].status = InstanceStatus::Hidden;
    let cancelled = LogicCall::Handle(LogicEvent::Action(ActionEvent {
        action: "microphone".into(),
        kind: ActionKind::Cancelled,
        value: None,
        record_key: None,
        origin: Origin::Synthetic,
        cancel_reason: Some(CancelReason::Mode),
    }));
    let entry = TraceEntry {
        at: h.now(),
        input: Input::Locale("C".into()),
        outputs: vec![Output::CallLogic {
            instance: id,
            call: CallId(999),
            invocation: cancelled.clone(),
        }],
    };
    // The exception cannot introduce a new action on an already hidden owner.
    let mut checker = InvariantChecker::new(before.clone());
    assert_eq!(
        checker.observe(&entry, &before).unwrap_err().invariant,
        Invariant::I5
    );
    before.instances[0].queue.push(cancelled);
    before.instances[0].queue_len = 1;
    let mut after = before.clone();
    after.instances[0].queue.clear();
    after.instances[0].queue_len = 0;
    let mut checker = InvariantChecker::new(before.clone());
    checker.observe(&entry, &after).unwrap();
    let mut after_dispose = entry;
    after_dispose
        .outputs
        .insert(0, Output::DisposeInstance { instance: id });
    let mut checker = InvariantChecker::new(before);
    assert_eq!(
        checker
            .observe(&after_dispose, &after)
            .unwrap_err()
            .invariant,
        Invariant::I8
    );
}

fn cleanup_harness() -> Harness {
    let mut h = Harness::new(CoreConfig::default());
    h.enable_invariant_checks();
    let mut cat = fixtures::controls_with_panel();
    let microphone = &mut cat.sets[0].widgets[1];
    microphone
        .handlers
        .insert(ActionKind::Pressed, "pressed".into());
    microphone
        .handlers
        .insert(ActionKind::Cancelled, "cancelled".into());
    h.logic.register("controls/main", None, |_, call| {
        if matches!(
            call,
            LogicCall::Handle(LogicEvent::Action(ActionEvent {
                kind: ActionKind::Pressed,
                ..
            }))
        ) {
            LogicReaction::Delayed {
                ms: 10,
                update: LogicUpdate::default(),
            }
        } else {
            LogicReaction::Update(LogicUpdate::default())
        }
    });
    let mut other = ProfileSpec::default_for("controls/other".parse().unwrap());
    other.for_target = Some(ProfileTarget::App("other".into()));
    cat.profiles.push(other);
    h.send(Input::Catalog(cat));
    let mut surface = headless(1);
    surface.caps.modes = true;
    h.send(Input::SurfaceAdded(surface));
    h.send(Input::Mode {
        surface: SurfaceId(1),
        mode: Mode::Display,
    });
    h
}

#[test]
fn delayed_cancel_before_hidden_passes_checker_and_replay() {
    let mut h = cleanup_harness();
    let start = h.logic.calls.len();
    h.touch(
        SurfaceId(1),
        1,
        TouchPhase::Down,
        h.hit(SurfaceId(1), "microphone"),
    );
    h.send(Input::Mode {
        surface: SurfaceId(1),
        mode: Mode::Native,
    });
    assert!(h.snapshot().captures.is_empty());
    h.advance_ms(10);
    let calls: Vec<_> = h.logic.calls[start..]
        .iter()
        .map(|(_, _, call)| call)
        .collect();
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert!(matches!(
        calls[0],
        LogicCall::Handle(LogicEvent::Action(ActionEvent {
            kind: ActionKind::Pressed,
            ..
        }))
    ));
    assert!(matches!(
        calls[1],
        LogicCall::Handle(LogicEvent::Action(ActionEvent {
            kind: ActionKind::Cancelled,
            ..
        }))
    ));
    assert!(matches!(calls[2], LogicCall::Handle(LogicEvent::Hidden)));
    assert!(h.invariant_error.is_none(), "{:?}", h.invariant_error);
    h.finish_invariant_checks().unwrap();
    assert_eq!(
        ReplayDump::from_json(&h.dump().to_json().unwrap())
            .unwrap()
            .replay()
            .unwrap()
            .core
            .snapshot(),
        h.snapshot()
    );
}

#[test]
fn delayed_cleanup_before_dispose_passes_checker_and_replay() {
    let mut h = cleanup_harness();
    let start = h.logic.calls.len();
    h.touch(
        SurfaceId(1),
        1,
        TouchPhase::Down,
        h.hit(SurfaceId(1), "microphone"),
    );
    h.send(Input::Mode {
        surface: SurfaceId(1),
        mode: Mode::Native,
    });
    h.send(Input::Fact {
        name: "app.id".into(),
        value: Some(FactValue::Text("other".into())),
    });
    assert!(h.snapshot().surfaces[0].profile.pending.is_some());
    h.advance_ms(10);
    assert!(h.snapshot().instances.is_empty());
    let calls: Vec<_> = h.logic.calls[start..]
        .iter()
        .map(|(_, _, call)| call)
        .collect();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(matches!(
        calls[1],
        LogicCall::Handle(LogicEvent::Action(ActionEvent {
            kind: ActionKind::Cancelled,
            ..
        }))
    ));
    assert!(h.invariant_error.is_none(), "{:?}", h.invariant_error);
    h.finish_invariant_checks().unwrap();
    assert_eq!(
        ReplayDump::from_json(&h.dump().to_json().unwrap())
            .unwrap()
            .replay()
            .unwrap()
            .core
            .snapshot(),
        h.snapshot()
    );
}
