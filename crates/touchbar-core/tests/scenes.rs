use touchbar_core::*;
use touchbar_model::*;

fn plugin(id: &str) -> PluginSpec {
    PluginSpec {
        id: id.parse().unwrap(),
        revision: format!("{id}-v1"),
        enabled: true,
        has_logic: true,
        requires: vec![],
    }
}
fn info(id: u64) -> SurfaceInfo {
    SurfaceInfo {
        id: SurfaceId(id),
        name: format!("headless:{id}").parse().unwrap(),
        kind: SurfaceKind::Headless,
        width_px: 1000,
        height_px: 100,
        px_per_mm: 5.0,
        caps: SurfaceCaps::default(),
    }
}
fn catalog() -> Catalog {
    let mut c = Catalog {
        plugins: vec![plugin("profiles"), plugin("controls"), plugin("shared")],
        ..Catalog::default()
    };
    for name in ["volume", "brightness", "static"] {
        let mut set = SetSpec::default_for(format!("controls/{name}").parse().unwrap());
        if name != "static" {
            set.logic = Some("shared".parse().unwrap());
        }
        set.widgets.push(WidgetSpec::new(
            "button".parse().unwrap(),
            if name == "static" {
                WidgetKind::Key { key: SystemKey::F1 }
            } else {
                WidgetKind::Button
            },
        ));
        c.sets.push(set);
    }
    for name in ["default", "app"] {
        let mut p = ProfileSpec::default_for(format!("profiles/{name}").parse().unwrap());
        p.default = name == "default";
        if name == "app" {
            p.for_target = Some(ProfileTarget::App("editor".into()));
        }
        p.slots.push(SlotSpec {
            id: "volume".parse().unwrap(),
            candidates: vec!["controls/volume".parse().unwrap()],
        });
        p.slots.push(SlotSpec {
            id: "second".parse().unwrap(),
            candidates: vec![
                format!(
                    "controls/{}",
                    if name == "default" {
                        "brightness"
                    } else {
                        "static"
                    }
                )
                .parse()
                .unwrap(),
            ],
        });
        c.profiles.push(p);
    }
    c
}
fn core(c: Catalog) -> Core {
    assert!(c.validate().is_empty(), "{:?}", c.validate());
    let mut core = Core::new(CoreConfig::default());
    core.handle(Timestamp(0), Input::Catalog(c));
    core.handle(Timestamp(0), Input::SurfaceAdded(info(1)));
    core
}
fn scene(core: &Core, id: u64) -> Scene {
    core.snapshot()
        .surfaces
        .iter()
        .find(|s| s.info.id == SurfaceId(id))
        .unwrap()
        .scene
        .clone()
}
fn item(core: &Core, id: u64, slot: &str) -> InstanceId {
    scene(core, id)
        .items
        .into_iter()
        .find(|i| i.slot.to_string() == slot)
        .unwrap()
        .instance
}
#[test]
fn initial_scene_uses_slot_order_shared_logic_provider_and_separate_surface_instances() {
    let mut core = Core::new(CoreConfig::default());
    core.handle(Timestamp(0), Input::Catalog(catalog()));
    let outputs = core.handle(Timestamp(0), Input::SurfaceAdded(info(1)));
    let s = scene(&core, 1);
    assert_eq!(s.profile.unwrap().to_string(), "profiles/default");
    assert_eq!(
        s.items
            .iter()
            .map(|i| i.slot.to_string())
            .collect::<Vec<_>>(),
        ["volume", "second"]
    );
    assert_eq!(outputs.iter().filter(|o| matches!(o, Output::CreateInstance {plugin, ..} if plugin.to_string() == "shared")).count(), 2);
    core.handle(Timestamp(0), Input::SurfaceAdded(info(2)));
    assert_ne!(item(&core, 1, "volume"), item(&core, 2, "volume"));
}
#[test]
fn profile_switch_preserves_shared_slot_and_disposes_removed_logic_instance() {
    let mut core = core(catalog());
    let volume = item(&core, 1, "volume");
    let removed = item(&core, 1, "second");
    let outputs = core.handle(
        Timestamp(10),
        Input::Fact {
            name: "app.id".into(),
            value: Some(FactValue::Text("editor".into())),
        },
    );
    assert_eq!(scene(&core, 1).profile.unwrap().to_string(), "profiles/app");
    assert_eq!(item(&core, 1, "volume"), volume);
    assert!(outputs.contains(&Output::DisposeInstance { instance: removed }));
    assert!(
        !outputs
            .iter()
            .any(|o| matches!(o, Output::CreateInstance { .. }))
    ); // New static key needs no runtime.
}
#[test]
fn unchanged_catalog_preserves_ids_but_set_and_shared_logic_revisions_replace_them() {
    let mut c = catalog();
    let mut core = core(c.clone());
    let volume = item(&core, 1, "volume");
    assert!(
        core.handle(Timestamp(1), Input::Catalog(c.clone()))
            .is_empty()
    );
    assert_eq!(item(&core, 1, "volume"), volume);
    c.plugins[2].revision = "shared-v2".into();
    let outputs = core.handle(Timestamp(2), Input::Catalog(c.clone()));
    assert!(outputs.contains(&Output::DisposeInstance { instance: volume }));
    let new = item(&core, 1, "volume");
    assert_ne!(new, volume);
    c.plugins[1].revision = "controls-v2".into();
    core.handle(Timestamp(3), Input::Catalog(c));
    assert_ne!(item(&core, 1, "volume"), new);
}
#[test]
fn unrelated_plugin_revision_does_not_recreate_instances() {
    let mut c = catalog();
    let mut core = core(c.clone());
    let before = item(&core, 1, "volume");
    c.plugins[0].revision = "profiles-v2".into();
    let outputs = core.handle(Timestamp(1), Input::Catalog(c));
    assert_eq!(item(&core, 1, "volume"), before);
    assert!(outputs.is_empty());
}
#[test]
fn config_slot_or_provider_change_recreates_instance() {
    for kind in 0..3 {
        let mut c = catalog();
        let mut core = core(c.clone());
        let before = item(&core, 1, "volume");
        match kind {
            0 => c.sets[0].logic_config = "new-config".into(),
            1 => c.profiles[0].slots[0].id = "renamed".parse().unwrap(),
            _ => c.sets[0].logic = Some("controls".parse().unwrap()),
        }
        let out = core.handle(Timestamp(1), Input::Catalog(c));
        assert!(out.contains(&Output::DisposeInstance { instance: before }));
        assert!(
            out.iter()
                .any(|o| matches!(o, Output::CreateInstance { .. }))
        );
        assert_ne!(
            item(&core, 1, if kind == 1 { "renamed" } else { "volume" }),
            before
        );
    }
}
#[test]
fn first_available_candidate_is_selected_and_missing_dependency_keeps_empty_slot() {
    let mut c = catalog();
    c.plugins[0].requires.push("missing".parse().unwrap());
    c.plugins[1].enabled = false;
    c.profiles[0].slots[0]
        .candidates
        .insert(0, "missing/set".parse().unwrap());
    assert!(scene(&core(c.clone()), 1).items.is_empty());
    c.plugins[1].enabled = true;
    assert_eq!(
        scene(&core(c), 1).items[0].set.reference.to_string(),
        "controls/volume"
    );
}
#[test]
fn resolution_explains_empty_slots_and_all_candidate_availability() {
    let mut c = catalog();
    c.plugins[0].requires.push("missing".parse().unwrap());
    c.plugins[1].enabled = false;
    c.profiles[0].slots[0]
        .candidates
        .insert(0, "missing/set".parse().unwrap());
    let snapshot = core(c).snapshot();
    let slots = &snapshot.surfaces[0].resolution.slots;
    assert_eq!(slots.len(), 2);
    assert_eq!(
        slots[0].candidates[0].availability,
        SetAvailability::MissingPlugin
    );
    assert_eq!(
        slots[0].candidates[1].availability,
        SetAvailability::DisabledPlugin
    );
    assert!(slots[0].selected.is_none());
}
#[test]
fn units_follow_set_profile_and_short_side_and_update_without_recreating() {
    let mut c = catalog();
    c.profiles[0].units = Some(UnitSize {
        value: 2.0,
        axis: Axis::Width,
    });
    c.sets[1].units = Some(UnitSize {
        value: 3.0,
        axis: Axis::Height,
    });
    let mut core = core(c.clone());
    let before = item(&core, 1, "volume");
    assert_eq!(scene(&core, 1).items[0].units, c.profiles[0].units.unwrap());
    assert_eq!(scene(&core, 1).items[1].units, c.sets[1].units.unwrap());
    c.profiles[0].units = None;
    core.handle(Timestamp(1), Input::Catalog(c));
    assert_eq!(item(&core, 1, "volume"), before);
    assert_eq!(
        scene(&core, 1).items[0].units,
        UnitSize {
            value: 1.0,
            axis: Axis::Height
        }
    );
}
#[test]
fn surface_conditions_choose_independent_profiles_on_geometry_change() {
    let mut c = catalog();
    c.profiles[1].for_target = None;
    c.profiles[1].default = true;
    c.profiles[1].when = Some(Condition::Eq {
        name: "surface.aspect".into(),
        value: FactValue::Number(20.0),
    });
    let mut core = core(c);
    core.handle(Timestamp(0), Input::SurfaceAdded(info(2)));
    let untouched = item(&core, 2, "volume");
    let mut changed = info(1);
    changed.width_px = 2000;
    core.handle(Timestamp(1), Input::SurfaceChanged(changed));
    assert_eq!(scene(&core, 1).profile.unwrap().to_string(), "profiles/app");
    assert_eq!(
        scene(&core, 2).profile.unwrap().to_string(),
        "profiles/default"
    );
    assert_eq!(item(&core, 2, "volume"), untouched);
}
#[test]
fn removing_surface_disposes_only_its_instances_and_reconnect_uses_new_ids() {
    let mut core = core(catalog());
    core.handle(Timestamp(0), Input::SurfaceAdded(info(2)));
    let lost = item(&core, 1, "volume");
    let kept = item(&core, 2, "volume");
    let out = core.handle(Timestamp(1), Input::SurfaceRemoved(SurfaceId(1)));
    assert!(out.contains(&Output::DisposeInstance { instance: lost }));
    assert!(!out.contains(&Output::DisposeInstance { instance: kept }));
    core.handle(Timestamp(2), Input::SurfaceAdded(info(1)));
    assert_ne!(item(&core, 1, "volume"), lost);
}
#[test]
fn profile_reconciliation_coalesces_render_until_frame_done() {
    let mut core = core(catalog());
    let generation = core.snapshot().surfaces[0].generation;
    let out = core.handle(
        Timestamp(1),
        Input::Fact {
            name: "app.id".into(),
            value: Some(FactValue::Text("editor".into())),
        },
    );
    assert!(!out.iter().any(|o| matches!(o, Output::Render { .. })));
    let out = core.handle(
        Timestamp(2),
        Input::FrameDone {
            surface: SurfaceId(1),
            generation,
        },
    );
    assert!(out.iter().any(|o| matches!(o, Output::Render {scene, ..} if scene.profile.as_ref().unwrap().to_string() == "profiles/app")));
}

#[test]
fn layer_units_are_independent_of_the_active_profile() {
    for explicit in [
        None,
        Some(UnitSize {
            value: 3.0,
            axis: Axis::Width,
        }),
    ] {
        let mut cat = catalog();
        cat.profiles[0].units = Some(UnitSize {
            value: 2.0,
            axis: Axis::Width,
        });
        cat.profiles[1].units = Some(UnitSize {
            value: 4.0,
            axis: Axis::Width,
        });
        cat.sets[2].units = explicit;
        cat.bindings.push(Binding {
            id: "layer".into(),
            when: None,
            trigger: Trigger::Key(KeyTrigger {
                keys: vec![WatchKey::Fn],
                gesture: KeyGesture::WhileHeld,
            }),
            action: LayerAction {
                operation: LayerOperation::Layer("controls/static".parse().unwrap()),
                surfaces: LayerSurfaces::Source,
            },
        });
        let mut c = core(cat);
        c.handle(
            Timestamp(1),
            Input::Key {
                source: KeySource::Desktop,
                code: WatchKey::Fn,
                phase: KeyPhase::Pressed,
            },
        );
        let before = scene(&c, 1)
            .items
            .into_iter()
            .find(|i| matches!(i.plane, ScenePlane::Layer(_)))
            .unwrap();
        let expected = explicit.unwrap_or(UnitSize {
            value: 1.0,
            axis: Axis::Height,
        });
        assert_eq!(before.units, expected);
        c.handle(
            Timestamp(2),
            Input::Fact {
                name: "app.id".into(),
                value: Some(FactValue::Text("editor".into())),
            },
        );
        let after = scene(&c, 1)
            .items
            .into_iter()
            .find(|i| matches!(i.plane, ScenePlane::Layer(_)))
            .unwrap();
        assert_eq!(after.instance, before.instance);
        assert_eq!(after.units, before.units);
        assert_eq!(
            Length::Unit(2.0).to_px(&info(1), after.units),
            Length::Unit(2.0).to_px(&info(1), before.units)
        );
    }
}
