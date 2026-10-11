use touchbar_core::*;
use touchbar_model::*;

fn binding(
    id: &str,
    key: WatchKey,
    gesture: KeyGesture,
    operation: LayerOperation,
    surfaces: LayerSurfaces,
) -> Binding<LayerAction> {
    Binding {
        id: id.into(),
        trigger: Trigger::Key(KeyTrigger {
            keys: vec![key],
            gesture,
        }),
        when: None,
        action: LayerAction {
            operation,
            surfaces,
        },
    }
}
fn catalog() -> Catalog {
    let mut base = SetSpec::default_for("p/base".parse().unwrap());
    base.widgets.push(WidgetSpec::new(
        "key".parse().unwrap(),
        WidgetKind::Key { key: SystemKey::F1 },
    ));
    let mut layer = SetSpec::default_for("p/fkeys".parse().unwrap());
    layer.widgets.push(WidgetSpec::new(
        "key".parse().unwrap(),
        WidgetKind::Key { key: SystemKey::F2 },
    ));
    let mut other = layer.clone();
    other.reference = "p/other".parse().unwrap();
    let mut profile = ProfileSpec::default_for("p/default".parse().unwrap());
    profile.default = true;
    profile.slots.push(SlotSpec {
        id: "main".parse().unwrap(),
        candidates: vec![base.reference.clone()],
    });
    Catalog {
        plugins: vec![PluginSpec {
            id: "p".parse().unwrap(),
            revision: "v1".into(),
            enabled: true,
            has_logic: false,
            requires: vec![],
        }],
        sets: vec![base, layer, other],
        profiles: vec![profile],
        bindings: vec![binding(
            "held",
            WatchKey::Fn,
            KeyGesture::WhileHeld,
            LayerOperation::Layer("p/fkeys".parse().unwrap()),
            LayerSurfaces::Source,
        )],
        ..Catalog::default()
    }
}
fn info(id: u64, own_keys: bool) -> SurfaceInfo {
    SurfaceInfo {
        id: SurfaceId(id),
        name: format!("headless:{id}").parse().unwrap(),
        kind: SurfaceKind::Headless,
        width_px: 500,
        height_px: 50,
        px_per_mm: 5.0,
        caps: SurfaceCaps {
            modes: true,
            watch_keys: own_keys,
            inject_keys: true,
            ..SurfaceCaps::default()
        },
    }
}
fn setup(cat: Catalog, own_keys: &[bool]) -> Core {
    let mut c = Core::new(CoreConfig::default());
    c.handle(Timestamp(0), Input::Catalog(cat));
    for (i, own) in own_keys.iter().enumerate() {
        let id = SurfaceId(i as u64 + 1);
        c.handle(Timestamp(0), Input::SurfaceAdded(info(id.0, *own)));
        c.handle(
            Timestamp(0),
            Input::Mode {
                surface: id,
                mode: Mode::Display,
            },
        );
    }
    c
}
fn key(c: &mut Core, at: u64, source: KeySource, code: WatchKey, phase: KeyPhase) -> Vec<Output> {
    c.handle(
        Timestamp(at),
        Input::Key {
            source,
            code,
            phase,
        },
    )
}
fn layers(c: &Core, surface: u64) -> Vec<String> {
    c.snapshot()
        .surfaces
        .iter()
        .find(|s| s.info.id == SurfaceId(surface))
        .unwrap()
        .scene
        .items
        .iter()
        .filter(|i| matches!(i.plane, ScenePlane::Layer(_)))
        .map(|i| i.set.reference.to_string())
        .collect()
}
fn layout(c: &mut Core, at: u64, surface: u64, map: HitMap) {
    let s = c
        .snapshot()
        .surfaces
        .into_iter()
        .find(|s| s.info.id == SurfaceId(surface))
        .unwrap();
    c.handle(
        Timestamp(at),
        Input::Layout {
            surface: s.info.id,
            generation: s.generation,
            hit_map: map,
            representations: vec![],
            scrubbers: vec![],
            slots: vec![],
            presentation: None,
        },
    );
}
fn hit(id: InstanceId, plane: ScenePlane) -> HitTarget {
    HitTarget {
        instance: id,
        widget: "key".parse().unwrap(),
        plane,
        rect: Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        },
        record_key: None,
    }
}
fn touch(c: &mut Core, at: u64, phase: TouchPhase, x: f32) -> Vec<Output> {
    c.handle(
        Timestamp(at),
        Input::Touch {
            surface: SurfaceId(1),
            contact: ContactId::new(1).unwrap(),
            phase,
            x,
            y: 20.0,
        },
    )
}

#[test]
fn catalog_watches_union_and_clears_removed_keys() {
    let mut c = setup(catalog(), &[true]);
    let mut cat = catalog();
    cat.bindings.push(binding(
        "other",
        WatchKey::Esc,
        KeyGesture::Press,
        LayerOperation::ClearLayers,
        LayerSurfaces::Source,
    ));
    let out = c.handle(Timestamp(1), Input::Catalog(cat));
    let watched = out
        .iter()
        .find_map(|o| match o {
            Output::WatchKeys {
                surface: SurfaceId(1),
                set,
            } => Some(set.clone()),
            _ => None,
        })
        .unwrap();
    assert!(watched.contains(&WatchKey::Fn) && watched.contains(&WatchKey::Esc));
    assert_eq!(watched.len(), 2);
    let mut empty = catalog();
    empty.bindings.clear();
    let out = c.handle(Timestamp(2), Input::Catalog(empty));
    assert!(out.contains(&Output::WatchKeys {
        surface: SurfaceId(1),
        set: vec![]
    }));
}
#[test]
fn held_and_toggle_share_one_instance_and_release_only_its_owner() {
    let mut cat = catalog();
    cat.bindings.push(binding(
        "toggle",
        WatchKey::Esc,
        KeyGesture::Press,
        LayerOperation::ToggleLayer("p/fkeys".parse().unwrap()),
        LayerSurfaces::Source,
    ));
    let mut c = setup(cat, &[true]);
    let base = c.snapshot().instances[0].id;
    key(
        &mut c,
        1,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Pressed,
    );
    assert_eq!(layers(&c, 1), ["p/fkeys"]);
    let instance = c.snapshot().instances[1].id;
    key(
        &mut c,
        2,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Esc,
        KeyPhase::Pressed,
    );
    assert_eq!(c.snapshot().instances.len(), 2);
    key(
        &mut c,
        3,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Released,
    );
    assert_eq!(layers(&c, 1), ["p/fkeys"]);
    assert_eq!(c.snapshot().instances[1].id, instance);
    key(
        &mut c,
        4,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Esc,
        KeyPhase::Released,
    );
    key(
        &mut c,
        5,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Esc,
        KeyPhase::Pressed,
    );
    assert!(layers(&c, 1).is_empty());
    assert_eq!(c.snapshot().instances[0].id, base);
    assert_eq!(c.snapshot().instances[0].status, InstanceStatus::Visible);
}
#[test]
fn reset_source_removes_all_of_its_explicit_targets_but_not_other_sources() {
    let mut cat = catalog();
    cat.bindings[0].action.surfaces = LayerSurfaces::All;
    let mut c = setup(cat, &[true, true, false]);
    key(
        &mut c,
        1,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Pressed,
    );
    key(
        &mut c,
        2,
        KeySource::Surface(SurfaceId(2)),
        WatchKey::Fn,
        KeyPhase::Pressed,
    );
    assert_eq!(layers(&c, 3), ["p/fkeys"]);
    c.handle(
        Timestamp(3),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Native,
        },
    );
    assert!(layers(&c, 1).is_empty());
    assert_eq!(layers(&c, 2), ["p/fkeys"]);
    assert_eq!(layers(&c, 3), ["p/fkeys"]);
    key(
        &mut c,
        4,
        KeySource::Surface(SurfaceId(2)),
        WatchKey::Fn,
        KeyPhase::Released,
    );
    assert!(layers(&c, 2).is_empty());
    assert!(layers(&c, 3).is_empty());
    c.handle(
        Timestamp(5),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Display,
        },
    );
    assert!(layers(&c, 1).is_empty());
}
#[test]
fn desktop_source_targets_only_surfaces_without_own_keys() {
    let mut c = setup(catalog(), &[true, false]);
    key(
        &mut c,
        1,
        KeySource::Desktop,
        WatchKey::Fn,
        KeyPhase::Pressed,
    );
    assert!(layers(&c, 1).is_empty());
    assert_eq!(layers(&c, 2), ["p/fkeys"]);
    c.handle(
        Timestamp(2),
        Input::Revoked {
            surface: SurfaceId(1),
        },
    );
    assert_eq!(layers(&c, 2), ["p/fkeys"]);
    key(
        &mut c,
        3,
        KeySource::Desktop,
        WatchKey::Fn,
        KeyPhase::Released,
    );
    assert!(layers(&c, 2).is_empty());
}
#[test]
fn nonheld_layer_toggles_and_reopening_allocates_fresh_instance() {
    let mut cat = catalog();
    cat.bindings[0].trigger = Trigger::Key(KeyTrigger {
        keys: vec![WatchKey::Fn],
        gesture: KeyGesture::Press,
    });
    cat.bindings.push(binding(
        "other",
        WatchKey::Esc,
        KeyGesture::Press,
        LayerOperation::ToggleLayer("p/other".parse().unwrap()),
        LayerSurfaces::Source,
    ));
    let mut c = setup(cat, &[true]);
    key(
        &mut c,
        1,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Pressed,
    );
    let first = c.snapshot().instances[1].id;
    key(
        &mut c,
        2,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Esc,
        KeyPhase::Pressed,
    );
    assert_eq!(layers(&c, 1), ["p/fkeys", "p/other"]);
    key(
        &mut c,
        3,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Released,
    );
    key(
        &mut c,
        4,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Pressed,
    ); // non-held layer toggles off
    assert_eq!(layers(&c, 1), ["p/other"]);
    key(
        &mut c,
        5,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Released,
    );
    key(
        &mut c,
        6,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Pressed,
    );
    assert_eq!(layers(&c, 1), ["p/other", "p/fkeys"]);
    assert_ne!(
        c.snapshot()
            .instances
            .iter()
            .find(|i| i.set.to_string() == "p/fkeys")
            .unwrap()
            .id,
        first
    ); // actually removed, fresh instance
}
#[test]
fn condition_disable_ends_layer_and_reenable_needs_fresh_key() {
    let mut cat = catalog();
    cat.bindings[0].when = Some(Condition::Eq {
        name: "enabled".into(),
        value: FactValue::Boolean(true),
    });
    let mut c = setup(cat, &[true]);
    c.handle(
        Timestamp(1),
        Input::Fact {
            name: "enabled".into(),
            value: Some(FactValue::Boolean(true)),
        },
    );
    key(
        &mut c,
        2,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Pressed,
    );
    assert_eq!(layers(&c, 1), ["p/fkeys"]);
    c.handle(
        Timestamp(3),
        Input::Fact {
            name: "enabled".into(),
            value: Some(FactValue::Boolean(false)),
        },
    );
    assert!(layers(&c, 1).is_empty());
    c.handle(
        Timestamp(4),
        Input::Fact {
            name: "enabled".into(),
            value: Some(FactValue::Boolean(true)),
        },
    );
    key(
        &mut c,
        5,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Pressed,
    );
    assert!(layers(&c, 1).is_empty());
    key(
        &mut c,
        6,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Released,
    );
    key(
        &mut c,
        7,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Pressed,
    );
    assert_eq!(layers(&c, 1), ["p/fkeys"]);
}
#[test]
fn hold_binding_uses_core_timer_and_stale_reset_timer_is_ignored() {
    let mut cat = catalog();
    cat.bindings[0].trigger = Trigger::Key(KeyTrigger {
        keys: vec![WatchKey::Fn],
        gesture: KeyGesture::Hold { ms: 100 },
    });
    let mut c = setup(cat, &[true]);
    let out = key(
        &mut c,
        1,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Pressed,
    );
    let timer = out
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
    assert_eq!(layers(&c, 1), ["p/fkeys"]);
    key(
        &mut c,
        102,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Released,
    );
    let out = key(
        &mut c,
        103,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Pressed,
    );
    let timer = out
        .iter()
        .find_map(|o| match o {
            Output::SetTimer {
                id,
                at: Timestamp(203),
            } => Some(*id),
            _ => None,
        })
        .unwrap();
    c.handle(
        Timestamp(104),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Native,
        },
    );
    c.handle(Timestamp(204), Input::Timer { id: timer });
    assert_eq!(layers(&c, 1), ["p/fkeys"]); // persistent activation remains; no second toggle
}
#[test]
fn layer_activation_releases_base_capture_and_preserves_visible_base() {
    let mut c = setup(catalog(), &[true]);
    let id = c.snapshot().instances[0].id;
    layout(&mut c, 0, 1, vec![hit(id, ScenePlane::Profile)]);
    touch(&mut c, 1, TouchPhase::Down, 20.0);
    let out = key(
        &mut c,
        2,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Pressed,
    );
    assert!(out.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Released
    }));
    assert!(c.snapshot().captures.is_empty());
    assert_eq!(c.snapshot().instances[0].status, InstanceStatus::Visible);
    assert!(
        touch(&mut c, 3, TouchPhase::Down, 20.0)
            .iter()
            .all(|o| !matches!(
                o,
                Output::SendKey {
                    phase: KeyPhase::Pressed,
                    ..
                }
            ))
    );
}

#[test]
fn held_activation_promotes_existing_layer_and_toggle_off_does_not_promote_it() {
    let mut cat = catalog();
    cat.bindings[0].action.surfaces = LayerSurfaces::All;
    cat.bindings.push(binding(
        "other",
        WatchKey::Esc,
        KeyGesture::Press,
        LayerOperation::ToggleLayer("p/other".parse().unwrap()),
        LayerSurfaces::Source,
    ));
    cat.bindings.push(binding(
        "toggle",
        WatchKey::F3,
        KeyGesture::Press,
        LayerOperation::ToggleLayer("p/fkeys".parse().unwrap()),
        LayerSurfaces::Source,
    ));
    let mut c = setup(cat, &[true]);
    key(
        &mut c,
        1,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Pressed,
    );
    let instance = c.snapshot().instances[1].id;
    key(
        &mut c,
        2,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Esc,
        KeyPhase::Pressed,
    );
    key(
        &mut c,
        3,
        KeySource::Desktop,
        WatchKey::Fn,
        KeyPhase::Pressed,
    );
    assert_eq!(layers(&c, 1), ["p/other", "p/fkeys"]);
    assert_eq!(
        c.snapshot()
            .instances
            .iter()
            .find(|i| i.set.to_string() == "p/fkeys")
            .unwrap()
            .id,
        instance
    );
    key(
        &mut c,
        4,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::F3,
        KeyPhase::Pressed,
    ); // latch fkeys
    key(
        &mut c,
        5,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Esc,
        KeyPhase::Released,
    );
    key(
        &mut c,
        6,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Esc,
        KeyPhase::Pressed,
    ); // other off
    key(
        &mut c,
        7,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Esc,
        KeyPhase::Released,
    );
    key(
        &mut c,
        8,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Esc,
        KeyPhase::Pressed,
    ); // other on top
    assert_eq!(layers(&c, 1), ["p/fkeys", "p/other"]);
    key(
        &mut c,
        9,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::F3,
        KeyPhase::Released,
    );
    key(
        &mut c,
        10,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::F3,
        KeyPhase::Pressed,
    ); // latch off, holds remain
    assert_eq!(layers(&c, 1), ["p/fkeys", "p/other"]);
}
#[test]
fn removing_lower_layer_keeps_top_capture_through_renumbered_layout() {
    let mut cat = catalog();
    cat.bindings.push(binding(
        "other",
        WatchKey::Esc,
        KeyGesture::Press,
        LayerOperation::ToggleLayer("p/other".parse().unwrap()),
        LayerSurfaces::Source,
    ));
    let mut c = setup(cat, &[true]);
    key(
        &mut c,
        1,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Pressed,
    );
    key(
        &mut c,
        2,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Esc,
        KeyPhase::Pressed,
    );
    let s = c.snapshot().surfaces.remove(0);
    c.handle(
        Timestamp(3),
        Input::FrameDone {
            surface: SurfaceId(1),
            generation: s.generation,
        },
    );
    let top = c.snapshot().surfaces[0].scene.items.last().unwrap().clone();
    layout(&mut c, 3, 1, vec![hit(top.instance, top.plane)]);
    touch(&mut c, 4, TouchPhase::Down, 20.0);
    key(
        &mut c,
        5,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Released,
    );
    let s = c.snapshot().surfaces.remove(0);
    c.handle(
        Timestamp(6),
        Input::FrameDone {
            surface: SurfaceId(1),
            generation: s.generation,
        },
    );
    let new_top = c.snapshot().surfaces[0].scene.items.last().unwrap().clone();
    layout(&mut c, 6, 1, vec![hit(new_top.instance, new_top.plane)]);
    assert_eq!(c.snapshot().captures.len(), 1);
    let up = touch(&mut c, 7, TouchPhase::Up, 20.0);
    assert!(up.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F2,
        phase: KeyPhase::Released
    }));
}
#[test]
fn top_layer_gaps_do_not_fall_through_to_profile_and_clear_releases_its_key() {
    let mut cat = catalog();
    cat.bindings.push(binding(
        "clear",
        WatchKey::Esc,
        KeyGesture::Press,
        LayerOperation::ClearLayers,
        LayerSurfaces::Source,
    ));
    let mut c = setup(cat, &[true]);
    let base = c.snapshot().instances[0].id;
    key(
        &mut c,
        1,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Pressed,
    );
    let s = c.snapshot().surfaces.remove(0);
    c.handle(
        Timestamp(2),
        Input::FrameDone {
            surface: SurfaceId(1),
            generation: s.generation,
        },
    );
    let top = c.snapshot().surfaces[0].scene.items.last().unwrap().clone();
    let mut lower = hit(base, ScenePlane::Profile);
    lower.rect.width = 500.0;
    layout(&mut c, 2, 1, vec![lower, hit(top.instance, top.plane)]);
    let gap = touch(&mut c, 3, TouchPhase::Down, 200.0);
    assert!(!gap.iter().any(|o| matches!(
        o,
        Output::SendKey {
            phase: KeyPhase::Pressed,
            ..
        }
    )));
    assert!(c.snapshot().captures.is_empty());
    let down = touch(&mut c, 4, TouchPhase::Down, 20.0);
    assert!(down.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F2,
        phase: KeyPhase::Pressed
    }));
    let cleared = key(
        &mut c,
        5,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Esc,
        KeyPhase::Pressed,
    );
    assert!(cleared.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F2,
        phase: KeyPhase::Released
    }));
    assert!(layers(&c, 1).is_empty());
    key(
        &mut c,
        6,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Released,
    );
    assert!(layers(&c, 1).is_empty());
}
#[test]
fn named_targets_and_surface_conditions_use_the_key_source() {
    let mut cat = catalog();
    cat.bindings[0].action.surfaces = LayerSurfaces::Names(vec!["headless:2".parse().unwrap()]);
    cat.bindings[0].when = Some(Condition::Eq {
        name: "surface.kind".into(),
        value: FactValue::Text("headless".into()),
    });
    let mut c = setup(cat, &[true, false]);
    key(
        &mut c,
        1,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Pressed,
    );
    assert!(layers(&c, 1).is_empty());
    assert_eq!(layers(&c, 2), ["p/fkeys"]);
    c.handle(
        Timestamp(2),
        Input::Revoked {
            surface: SurfaceId(1),
        },
    );
    assert!(layers(&c, 2).is_empty());
    key(
        &mut c,
        3,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Pressed,
    );
    assert!(layers(&c, 2).is_empty());
}

#[test]
fn layer_closes_panel_and_late_profile_reply_cannot_reopen_it() {
    let mut cat = catalog();
    cat.plugins[0].has_logic = true;
    cat.sets[0].logic = Some("p".parse().unwrap());
    cat.sets[0].widgets[0] = WidgetSpec::new("button".parse().unwrap(), WidgetKind::Button);
    cat.sets[0].widgets[0]
        .handlers
        .insert(ActionKind::Pressed, "press".into());
    cat.sets[0].widgets[0]
        .handlers
        .insert(ActionKind::Cancelled, "cancel".into());
    cat.sets[0].panels.push(PanelSpec {
        id: "panel".parse().unwrap(),
        widgets: vec![WidgetSpec::new(
            "key".parse().unwrap(),
            WidgetKind::Key { key: SystemKey::F2 },
        )],
        dismiss_on_selection: false,
        principal_item: None,
        dismissal: DismissalPolicy {
            release: false,
            ..DismissalPolicy::default()
        },
    });
    for open_before_layer in [false, true] {
        let mut c = setup(cat.clone(), &[true]);
        let id = c.snapshot().instances[0].id;
        fn call(out: &[Output]) -> (InstanceId, CallId, LogicCall) {
            out.iter()
                .find_map(|o| match o {
                    Output::CallLogic {
                        instance,
                        call,
                        invocation,
                    } => Some((*instance, *call, invocation.clone())),
                    _ => None,
                })
                .unwrap()
        }
        fn reply(c: &mut Core, at: u64, out: &[Output], open: bool) -> Vec<Output> {
            let (instance, call, _) = call(out);
            c.handle(
                Timestamp(at),
                Input::LogicUpdate {
                    instance,
                    call,
                    update: LogicUpdate {
                        presentation: open.then_some(PresentationCommand::Open {
                            panel: "panel".parse().unwrap(),
                            placement: Placement::Full,
                            lifecycle: PresentationLifecycle::Persistent,
                        }),
                        ..LogicUpdate::default()
                    },
                },
            )
        }
        let init = c.handle(
            Timestamp(0),
            Input::LogicDescribed {
                instance: id,
                description: LogicDescription {
                    actions: vec![
                        (
                            "press".into(),
                            ActionDescription {
                                kinds: vec![ActionKind::Pressed],
                                value: None,
                            },
                        ),
                        (
                            "cancel".into(),
                            ActionDescription {
                                kinds: vec![ActionKind::Cancelled],
                                value: None,
                            },
                        ),
                    ],
                    state: vec![],
                },
            },
        );
        let shown = reply(&mut c, 0, &init, false);
        reply(&mut c, 0, &shown, false);
        let mut target = hit(id, ScenePlane::Profile);
        target.widget = "button".parse().unwrap();
        layout(&mut c, 0, 1, vec![target]);
        let down = touch(&mut c, 1, TouchPhase::Down, 20.0);
        if open_before_layer {
            reply(&mut c, 2, &down, true);
        }
        assert_eq!(
            c.snapshot().surfaces[0].scene.presentation.is_some(),
            open_before_layer
        );
        key(
            &mut c,
            3,
            KeySource::Surface(SurfaceId(1)),
            WatchKey::Fn,
            KeyPhase::Pressed,
        );
        assert!(c.snapshot().surfaces[0].scene.presentation.is_none());
        // Test both the delayed physical Pressed reply and the synthetic Cancelled reply.
        let pending = c
            .snapshot()
            .instances
            .iter()
            .find(|i| i.id == id)
            .unwrap()
            .in_flight
            .unwrap();
        c.handle(
            Timestamp(4),
            Input::LogicUpdate {
                instance: id,
                call: pending,
                update: LogicUpdate {
                    presentation: Some(PresentationCommand::Open {
                        panel: "panel".parse().unwrap(),
                        placement: Placement::Full,
                        lifecycle: PresentationLifecycle::Persistent,
                    }),
                    ..LogicUpdate::default()
                },
            },
        );
        assert!(c.snapshot().surfaces[0].scene.presentation.is_none());
        assert_eq!(
            c.snapshot()
                .instances
                .iter()
                .find(|i| i.id == id)
                .unwrap()
                .status,
            InstanceStatus::Visible
        );
    }
}
#[test]
fn catalog_reload_preserves_unchanged_binding_hold_and_prunes_disabled_layer() {
    let mut c = setup(catalog(), &[true]);
    key(
        &mut c,
        1,
        KeySource::Surface(SurfaceId(1)),
        WatchKey::Fn,
        KeyPhase::Pressed,
    );
    let id = c.snapshot().instances[1].id;
    c.handle(Timestamp(2), Input::Catalog(catalog()));
    assert_eq!(c.snapshot().instances[1].id, id);
    assert_eq!(layers(&c, 1), ["p/fkeys"]);
    let mut disabled = catalog();
    disabled.plugins[0].enabled = false;
    c.handle(Timestamp(3), Input::Catalog(disabled));
    assert!(layers(&c, 1).is_empty());
    assert!(c.snapshot().instances.is_empty());
}
