use touchbar_core::*;
use touchbar_model::*;

fn plugin(id: &str) -> PluginSpec {
    PluginSpec {
        id: id.parse().unwrap(),
        revision: "v1".into(),
        enabled: true,
        has_logic: true,
        requires: vec![],
    }
}
fn profile(id: &str, target: Option<ProfileTarget>) -> ProfileSpec {
    ProfileSpec {
        for_target: target,
        ..ProfileSpec::default_for(id.parse().unwrap())
    }
}
fn surface() -> SurfaceInfo {
    SurfaceInfo {
        id: SurfaceId(1),
        name: "headless:main".parse().unwrap(),
        kind: SurfaceKind::Headless,
        width_px: 2008,
        height_px: 60,
        px_per_mm: 6.0,
        caps: SurfaceCaps::default(),
    }
}
fn catalog() -> Catalog {
    let mut c = Catalog {
        plugins: vec![plugin("p")],
        ..Catalog::default()
    };
    let mut p = profile("p/default", None);
    p.default = true;
    c.profiles.push(p);
    c.profiles.push(profile(
        "p/browser",
        Some(ProfileTarget::Category("browser".into())),
    ));
    c.profiles.push(profile(
        "p/player",
        Some(ProfileTarget::Category("player".into())),
    ));
    c.profiles
        .push(profile("p/app", Some(ProfileTarget::App("firefox".into()))));
    c
}
fn facts(app: &str, categories: &[&str]) -> Facts {
    [
        ("app.id".into(), FactValue::Text(app.into())),
        (
            "app.categories".into(),
            FactValue::List(categories.iter().map(|s| (*s).into()).collect()),
        ),
    ]
    .into_iter()
    .collect()
}
fn chosen(c: &Catalog, f: &Facts) -> Option<String> {
    resolve(c, f, &surface()).selected.map(|p| p.to_string())
}
#[test]
fn app_precedes_categories_and_category_order_precedes_priority() {
    let mut c = catalog();
    c.profiles[2].priority = 999;
    assert_eq!(
        chosen(&c, &facts("firefox", &["browser", "player"])).as_deref(),
        Some("p/app")
    );
    assert_eq!(
        chosen(&c, &facts("other", &["missing", "browser", "player"])).as_deref(),
        Some("p/browser")
    );
    assert_eq!(
        chosen(&c, &facts("other", &["player", "browser"])).as_deref(),
        Some("p/player")
    );
    assert_eq!(
        chosen(&c, &facts("other", &[])).as_deref(),
        Some("p/default")
    );
}
#[test]
fn matched_when_precedes_unconditional_even_with_negative_priority() {
    let mut c = catalog();
    for (id, priority) in [("p/video", -20), ("p/video-high", -10)] {
        let mut p = profile(id, Some(ProfileTarget::Category("browser".into())));
        p.when = Some(Condition::Eq {
            name: "media.active".into(),
            value: FactValue::Boolean(true),
        });
        p.priority = priority;
        c.profiles.push(p);
    }
    let mut f = facts("other", &["browser"]);
    assert_eq!(chosen(&c, &f).as_deref(), Some("p/browser"));
    f.insert("media.active".into(), FactValue::Boolean(true));
    assert_eq!(chosen(&c, &f).as_deref(), Some("p/video-high"));
}
#[test]
fn surface_conditions_are_derived_and_resolution_explains_rejections() {
    let mut c = catalog();
    c.profiles[3].when = Some(Condition::Eq {
        name: "surface.kind".into(),
        value: FactValue::Text("window".into()),
    });
    let r = resolve(&c, &facts("firefox", &["browser"]), &surface());
    assert_eq!(r.selected.unwrap().to_string(), "p/browser");
    let app = &r.steps[0];
    assert_eq!(app.candidates[0].profile.to_string(), "p/app");
    assert!(!app.candidates[0].when_matches);
    assert_eq!(
        r.facts.get(&"surface.kind".into()),
        Some(&FactValue::Text("headless".into()))
    );
}
#[test]
fn locked_profile_applies_to_window_but_not_touchbar() {
    let mut c = catalog();
    let mut p = profile("p/locked", None);
    p.locked = true;
    c.profiles.push(p);
    let f = [("session.locked".into(), FactValue::Boolean(true))]
        .into_iter()
        .collect();
    assert_eq!(chosen(&c, &f).as_deref(), Some("p/locked"));
    let mut s = surface();
    s.kind = SurfaceKind::Touchbar;
    s.name = "touchbar".parse().unwrap();
    assert_eq!(
        resolve(&c, &f, &s).selected.unwrap().to_string(),
        "p/default"
    );
}
#[test]
fn conflicts_skip_entire_target_and_surface_override_wins_global_choice() {
    let mut c = catalog();
    c.profiles.push(profile(
        "p/other-browser",
        Some(ProfileTarget::Category("browser".into())),
    ));
    let f = facts("other", &["browser"]);
    let r = resolve(&c, &f, &surface());
    assert_eq!(r.selected.unwrap().to_string(), "p/default");
    assert!(r.steps[1].conflict);
    c.profile_for.push(ProfileOverride {
        surface: None,
        target: ProfileTarget::Category("browser".into()),
        profile: "p/browser".parse().unwrap(),
    });
    assert_eq!(chosen(&c, &f).as_deref(), Some("p/browser"));
    c.profile_for.push(ProfileOverride {
        surface: Some(surface().name),
        target: ProfileTarget::Category("browser".into()),
        profile: "p/other-browser".parse().unwrap(),
    });
    assert_eq!(chosen(&c, &f).as_deref(), Some("p/other-browser"));
}
#[test]
fn disabled_profile_owner_is_explained_and_not_selected() {
    let mut c = catalog();
    c.plugins.push(plugin("disabled"));
    c.plugins[1].enabled = false;
    c.profiles.push(profile(
        "disabled/app",
        Some(ProfileTarget::App("other".into())),
    ));
    let r = resolve(&c, &facts("other", &[]), &surface());
    assert_eq!(r.selected.unwrap().to_string(), "p/default");
    assert!(!r.steps[0].candidates[0].available);
}
#[test]
fn explicit_manual_profile_is_included_in_explanation_with_its_condition() {
    let mut c = catalog();
    let mut manual = profile("p/manual", None);
    manual.when = Some(Condition::Exists("manual.ready".into()));
    c.profiles.push(manual);
    c.profile_for.push(ProfileOverride {
        surface: None,
        target: ProfileTarget::App("firefox".into()),
        profile: "p/manual".parse().unwrap(),
    });
    let r = resolve(&c, &facts("firefox", &["browser"]), &surface());
    assert_eq!(r.selected.unwrap().to_string(), "p/browser");
    assert!(
        r.steps[0]
            .candidates
            .iter()
            .any(|p| p.profile.to_string() == "p/manual" && !p.when_matches)
    );
}

#[test]
fn pending_target_changes_keep_deadline_and_last_contact_commits_latest_target() {
    let mut p = ProfileSwitch::default();
    let a = Some("p/a".parse().unwrap());
    let b = Some("p/b".parse().unwrap());
    let c = Some("p/c".parse().unwrap());
    p.request(Timestamp(0), a.clone(), 0, 1500);
    p.request(Timestamp(100), b, 1, 1500);
    assert_eq!(p.active, a);
    p.request(Timestamp(1000), c.clone(), 1, 1500);
    assert_eq!(p.pending.as_ref().unwrap().deadline, Timestamp(1600));
    assert!(p.contacts_finished(0));
    assert_eq!(p.active, c);
    assert!(p.pending.is_none());
}
#[test]
fn pending_return_to_active_cancels_switch_and_deadline_is_inclusive() {
    let mut p = ProfileSwitch::default();
    let a = Some("p/a".parse().unwrap());
    let b = Some("p/b".parse().unwrap());
    p.request(Timestamp(0), a.clone(), 0, 1500);
    p.request(Timestamp(10), b.clone(), 2, 1500);
    p.request(Timestamp(20), a.clone(), 2, 1500);
    assert!(p.pending.is_none());
    p.request(Timestamp(100), b.clone(), 2, 1500);
    assert!(!p.expire(Timestamp(1599)));
    assert!(p.expire(Timestamp(1600)));
    assert_eq!(p.active, b);
}
#[test]
fn switch_can_pending_to_no_profile_and_serializes_complete_state() {
    let mut p = ProfileSwitch::default();
    p.request(Timestamp(0), Some("p/a".parse().unwrap()), 0, 1500);
    p.request(Timestamp(10), None, 1, 1500);
    let mut restored: ProfileSwitch =
        serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    assert!(restored.expire(Timestamp(1510)));
    assert!(restored.active.is_none());
}
