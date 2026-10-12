use touchbar_model::*;

fn catalog() -> Catalog {
    let plugin = PluginSpec {
        id: "controls".parse().unwrap(),
        revision: "controls-v1".into(),
        enabled: true,
        has_logic: true,
        requires: vec![],
    };
    let set = SetSpec {
        reference: "controls/volume".parse().unwrap(),
        logic: Some(plugin.id.clone()),
        ..SetSpec::default_for("controls/volume".parse().unwrap())
    };
    let profile = ProfileSpec {
        reference: "controls/default".parse().unwrap(),
        default: true,
        slots: vec![SlotSpec {
            id: "volume".parse().unwrap(),
            candidates: vec![set.reference.clone()],
        }],
        ..ProfileSpec::default_for("controls/default".parse().unwrap())
    };
    Catalog {
        plugins: vec![plugin],
        sets: vec![set],
        profiles: vec![profile],
        bindings: vec![],
        profile_for: vec![],
    }
}
fn rejects(c: &Catalog, code: CatalogIssueCode) {
    assert!(
        c.validate().iter().any(|issue| issue.code == code),
        "{:?}",
        c.validate()
    );
}
#[test]
fn logic_bindings_and_persist_require_a_logic_provider() {
    for part in 0..3 {
        let mut c = catalog();
        c.sets[0].logic = None;
        if part == 2 {
            c.sets[0].persist.push("value".into());
        } else {
            let mut widget = WidgetSpec::new("button".parse().unwrap(), WidgetKind::Button);
            if part == 0 {
                widget
                    .handlers
                    .insert(ActionKind::Activated, "activate".into());
            } else {
                widget
                    .state_bindings
                    .insert(StateBinding::Text, "caption".into());
            }
            c.sets[0].widgets.push(widget);
        }
        rejects(&c, CatalogIssueCode::InvalidWidget);
    }
}

#[test]
fn valid_catalog_round_trips_with_ordered_candidates() {
    let c = catalog();
    assert!(c.validate().is_empty());
    assert_eq!(
        serde_json::from_str::<Catalog>(&serde_json::to_string(&c).unwrap()).unwrap(),
        c
    );
}
#[test]
fn duplicate_plugins_are_rejected() {
    let mut c = catalog();
    c.plugins.push(c.plugins[0].clone());
    rejects(&c, CatalogIssueCode::DuplicatePlugin);
}
#[test]
fn duplicate_sets_are_rejected() {
    let mut c = catalog();
    c.sets.push(c.sets[0].clone());
    rejects(&c, CatalogIssueCode::DuplicateSet);
}
#[test]
fn duplicate_profiles_are_rejected() {
    let mut c = catalog();
    c.profiles.push(c.profiles[0].clone());
    rejects(&c, CatalogIssueCode::DuplicateProfile);
}
#[test]
fn duplicate_slots_are_rejected() {
    let mut c = catalog();
    let s = c.profiles[0].slots[0].clone();
    c.profiles[0].slots.push(s);
    rejects(&c, CatalogIssueCode::DuplicateSlot);
}
#[test]
fn unknown_owner_is_rejected() {
    let mut c = catalog();
    c.sets[0].reference = "missing/volume".parse().unwrap();
    rejects(&c, CatalogIssueCode::UnknownPlugin);
}
#[test]
fn missing_set_in_installed_plugin_is_rejected() {
    let mut c = catalog();
    c.profiles[0].slots[0].candidates = vec!["controls/missing".parse().unwrap()];
    rejects(&c, CatalogIssueCode::UnknownSet);
}
#[test]
fn missing_required_plugin_leaves_profile_valid() {
    let mut c = catalog();
    c.plugins[0].requires.push("media".parse().unwrap());
    c.profiles[0].slots[0]
        .candidates
        .push("media/transport".parse().unwrap());
    assert!(c.validate().is_empty());
}
#[test]
fn missing_undeclared_plugin_is_rejected() {
    let mut c = catalog();
    c.profiles[0].slots[0]
        .candidates
        .push("missing/foo".parse().unwrap());
    rejects(&c, CatalogIssueCode::UnknownSet);
}
#[test]
fn disabled_plugin_is_unavailable_without_invalidating_reference() {
    let mut c = catalog();
    c.plugins[0].enabled = false;
    assert!(c.validate().is_empty());
    assert!(!c.set_available(&c.sets[0].reference));
}
#[test]
fn unconditioned_profiles_for_same_target_conflict() {
    let mut c = catalog();
    let mut p = c.profiles[0].clone();
    p.reference = "controls/other".parse().unwrap();
    c.profiles.push(p);
    rejects(&c, CatalogIssueCode::ProfileConflict);
}
#[test]
fn conditional_profiles_with_equal_priority_conflict() {
    let mut c = catalog();
    c.profiles[0].when = Some(Condition::Exists("a".into()));
    let mut p = c.profiles[0].clone();
    p.reference = "controls/other".parse().unwrap();
    p.when = Some(Condition::Exists("b".into()));
    c.profiles.push(p);
    rejects(&c, CatalogIssueCode::ProfileConflict);
}
#[test]
fn conditional_profiles_with_distinct_priority_are_valid() {
    let mut c = catalog();
    c.profiles[0].when = Some(Condition::Exists("a".into()));
    let mut p = c.profiles[0].clone();
    p.reference = "controls/other".parse().unwrap();
    p.priority = 1;
    c.profiles.push(p);
    assert!(c.validate().is_empty());
}
#[test]
fn reference_only_profiles_do_not_conflict() {
    let mut c = catalog();
    c.profiles[0].default = false;
    let mut p = c.profiles[0].clone();
    p.reference = "controls/other".parse().unwrap();
    c.profiles.push(p);
    assert!(c.validate().is_empty());
}
#[test]
fn different_application_targets_do_not_conflict() {
    let mut c = catalog();
    c.profiles[0].default = false;
    c.profiles[0].for_target = Some(ProfileTarget::App("firefox".into()));
    let mut p = c.profiles[0].clone();
    p.reference = "controls/other".parse().unwrap();
    p.for_target = Some(ProfileTarget::App("vlc".into()));
    c.profiles.push(p);
    assert!(c.validate().is_empty());
}
#[test]
fn unknown_logic_module_is_rejected() {
    let mut c = catalog();
    c.sets[0].logic = Some("missing".parse().unwrap());
    rejects(&c, CatalogIssueCode::UnknownLogic);
}
#[test]
fn plugin_without_logic_cannot_supply_module() {
    let mut c = catalog();
    c.plugins[0].has_logic = false;
    rejects(&c, CatalogIssueCode::UnknownLogic);
}
#[test]
fn duplicate_widgets_are_rejected() {
    let mut c = catalog();
    let w = WidgetSpec::new("volume".parse().unwrap(), WidgetKind::Button);
    c.sets[0].widgets = vec![w.clone(), w];
    rejects(&c, CatalogIssueCode::DuplicateWidget);
}
#[test]
fn unknown_panel_is_rejected() {
    let mut c = catalog();
    let mut w = WidgetSpec::new("volume".parse().unwrap(), WidgetKind::Button);
    w.expanded_panel = Some("absent".parse().unwrap());
    c.sets[0].widgets.push(w);
    rejects(&c, CatalogIssueCode::UnknownPanel);
}
#[test]
fn canvas_requires_canvas_from() {
    let mut c = catalog();
    c.sets[0]
        .widgets
        .push(WidgetSpec::new("plot".parse().unwrap(), WidgetKind::Canvas));
    rejects(&c, CatalogIssueCode::CanvasBindingRequired);
}
#[test]
fn excessive_condition_depth_is_rejected() {
    let mut c = catalog();
    let mut condition = Condition::Exists("a".into());
    for _ in 0..8 {
        condition = Condition::Not(Box::new(condition));
    }
    c.profiles[0].when = Some(condition);
    rejects(&c, CatalogIssueCode::InvalidCondition);
}
#[test]
fn invalid_programmatic_length_is_rejected() {
    let mut c = catalog();
    let mut w = WidgetSpec::new("volume".parse().unwrap(), WidgetKind::Button);
    w.width = Some(Length::Unit(f32::NAN));
    c.sets[0].widgets.push(w);
    rejects(&c, CatalogIssueCode::InvalidLength);
}
#[test]
fn duplicate_panels_are_rejected() {
    let mut c = catalog();
    let p = PanelSpec {
        id: "expanded".parse().unwrap(),
        widgets: vec![],
        dismiss_on_selection: false,
        principal_item: None,
        dismissal: DismissalPolicy::default(),
    };
    c.sets[0].panels = vec![p.clone(), p];
    rejects(&c, CatalogIssueCode::DuplicatePanel);
}
#[test]
fn principal_item_must_exist_in_its_panel() {
    let mut c = catalog();
    c.sets[0].panels.push(PanelSpec {
        id: "expanded".parse().unwrap(),
        widgets: vec![],
        dismiss_on_selection: false,
        principal_item: Some("missing".parse().unwrap()),
        dismissal: DismissalPolicy::default(),
    });
    rejects(&c, CatalogIssueCode::InvalidWidget);
}
#[test]
fn slider_range_must_be_finite_and_increasing() {
    let mut c = catalog();
    c.sets[0].widgets.push(WidgetSpec::new(
        "level".parse().unwrap(),
        WidgetKind::Slider {
            minimum: 1.0,
            maximum: 0.0,
        },
    ));
    rejects(&c, CatalogIssueCode::InvalidWidget);
}
#[test]
fn zero_hold_duration_is_rejected() {
    let mut c = catalog();
    let mut w = WidgetSpec::new("volume".parse().unwrap(), WidgetKind::Button);
    w.hold_ms = Some(0);
    c.sets[0].widgets.push(w);
    rejects(&c, CatalogIssueCode::InvalidWidget);
}
#[test]
fn profile_cannot_have_multiple_selection_targets() {
    let mut c = catalog();
    c.profiles[0].locked = true;
    rejects(&c, CatalogIssueCode::InvalidProfile);
}
#[test]
fn duplicate_persist_fields_are_rejected() {
    let mut c = catalog();
    c.sets[0].persist = vec!["muted".into(), "muted".into()];
    rejects(&c, CatalogIssueCode::DuplicatePersist);
}
#[test]
fn malformed_binding_is_rejected() {
    let mut c = catalog();
    c.bindings.push(Binding {
        id: "fn".into(),
        trigger: Trigger::Key(KeyTrigger {
            keys: vec![],
            gesture: KeyGesture::WhileHeld,
        }),
        when: None,
        action: LayerAction {
            operation: LayerOperation::ClearLayers,
            surfaces: LayerSurfaces::Source,
        },
    });
    rejects(&c, CatalogIssueCode::InvalidBinding);
}
#[test]
fn duplicate_binding_ids_are_rejected() {
    let mut c = catalog();
    let b = Binding {
        id: "fn".into(),
        trigger: Trigger::Key(KeyTrigger {
            keys: vec![WatchKey::Fn],
            gesture: KeyGesture::WhileHeld,
        }),
        when: None,
        action: LayerAction {
            operation: LayerOperation::ClearLayers,
            surfaces: LayerSurfaces::Source,
        },
    };
    c.bindings = vec![b.clone(), b];
    rejects(&c, CatalogIssueCode::DuplicateBinding);
}
#[test]
fn conflicting_binding_triggers_are_reported_by_catalog_validation() {
    let mut c = catalog();
    let b = Binding {
        id: "fn".into(),
        trigger: Trigger::Key(KeyTrigger {
            keys: vec![WatchKey::Fn],
            gesture: KeyGesture::Press,
        }),
        when: None,
        action: LayerAction {
            operation: LayerOperation::ClearLayers,
            surfaces: LayerSurfaces::Source,
        },
    };
    let mut other = b.clone();
    other.id = "fn-other".into();
    other.action.surfaces = LayerSurfaces::All;
    c.bindings = vec![b, other];
    rejects(&c, CatalogIssueCode::InvalidBinding);
}

#[test]
fn hold_callback_and_panel_require_explicit_positive_duration() {
    for panel in [false, true] {
        let mut c = catalog();
        let mut w = WidgetSpec::new("hold".parse().unwrap(), WidgetKind::Button);
        if panel {
            c.sets[0].panels.push(PanelSpec {
                id: "expanded".parse().unwrap(),
                widgets: vec![],
                dismiss_on_selection: false,
                principal_item: None,
                dismissal: DismissalPolicy::default(),
            });
            w.press_and_hold_panel = Some("expanded".parse().unwrap());
        } else {
            w.handlers.insert(ActionKind::LongPressed, "hold".into());
        }
        c.sets[0].widgets.push(w);
        rejects(&c, CatalogIssueCode::InvalidWidget);
        c.sets[0].widgets[0].hold_ms = Some(420);
        assert!(c.validate().is_empty(), "{:?}", c.validate());
    }
}
