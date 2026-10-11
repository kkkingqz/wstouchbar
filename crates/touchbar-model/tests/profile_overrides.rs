use touchbar_model::*;
fn catalog() -> Catalog {
    Catalog {
        plugins: vec![PluginSpec {
            id: "p".parse().unwrap(),
            revision: "v1".into(),
            enabled: true,
            has_logic: false,
            requires: vec![],
        }],
        profiles: vec![ProfileSpec::default_for("p/manual".parse().unwrap())],
        profile_for: vec![ProfileOverride {
            surface: None,
            target: ProfileTarget::App("editor".into()),
            profile: "p/manual".parse().unwrap(),
        }],
        ..Catalog::default()
    }
}
#[test]
fn overrides_reject_missing_profile_and_duplicate_scope_target() {
    let mut c = catalog();
    assert!(c.validate().is_empty());
    c.profile_for[0].profile = "p/missing".parse().unwrap();
    assert!(
        c.validate()
            .iter()
            .any(|e| e.code == CatalogIssueCode::InvalidProfile)
    );
    let mut c = catalog();
    c.profile_for.push(c.profile_for[0].clone());
    assert!(
        c.validate()
            .iter()
            .any(|e| e.code == CatalogIssueCode::InvalidProfile)
    );
}
#[test]
fn global_and_surface_overrides_are_distinct_and_round_trip_revisions() {
    let mut c = catalog();
    let mut o = c.profile_for[0].clone();
    o.surface = Some("window:main".parse().unwrap());
    c.profile_for.push(o);
    assert!(c.validate().is_empty());
    let restored: Catalog = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
    assert_eq!(c, restored);
}
