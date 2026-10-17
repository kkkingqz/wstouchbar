use serde::Deserialize;
use touchbar_model::{Condition, FactName, FactValue, Facts};
use touchbar_sets::config::{When, parse_document};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    when: When,
}
fn parse(source: &str) -> Result<Condition, touchbar_sets::DocumentError> {
    let doc: Document = parse_document("profile.toml".as_ref(), source, |d: &Document| {
        d.when.to_condition("when").map(|_| ())
    })
    .unwrap();
    doc.when.to_condition("when")
}
#[test]
fn approved_conditions_evaluate_and_preserve_json_value_shape() {
    let facts = Facts::from([
        (FactName::from("app.id"), FactValue::Text("firefox".into())),
        (FactName::from("media.active"), FactValue::Boolean(true)),
    ]);
    for source in [
        r#"when = { "app.id" = "firefox" }"#,
        r#"when = { "app.id" = {in = ["firefox", "chromium"]} }"#,
        r#"when = {exists = "app.id"}"#,
        r#"when = {not = {"session.locked" = true}}"#,
        r#"when = {all = [{"media.active" = true}, {exists = "app.id"}]}"#,
        r#"when = {any = [{"app.id" = "firefox"}, {"app.id" = "chromium"}]}"#,
        r#"when = {"app.id" = "firefox", "media.active" = true}"#,
    ] {
        let condition = parse(source).unwrap();
        assert!(condition.evaluate(&facts));
        condition.validate().unwrap();
        let doc: Document =
            parse_document("profile.toml".as_ref(), source, |_: &Document| Ok(())).unwrap();
        let restored: When =
            serde_json::from_slice(&serde_json::to_vec(&doc.when).unwrap()).unwrap();
        assert_eq!(restored, doc.when);
    }
}
#[test]
fn invalid_conditions_report_the_fact_or_operator_location() {
    for (source, path) in [
        (r#"when = {exists = ""}"#, "when.exists"),
        (
            r#"when = {exists = "app.id", "media.active" = true}"#,
            "when",
        ),
        (r#"when = {"app.id" = {in = []}}"#, "when.app.id.in"),
        (r#"when = {"app.id" = {oops = ["x"]}}"#, "when.app.id"),
        (r#"when = {all = "x"}"#, "when.all"),
    ] {
        let e = parse_document::<Document>("profile.toml".as_ref(), source, |d| {
            d.when.to_condition("when").map(|_| ())
        })
        .err()
        .unwrap();
        assert_eq!(e.path, path);
        assert_eq!(e.file, std::path::PathBuf::from("profile.toml"));
    }
}
#[test]
fn condition_depth_is_bounded_at_eight_including_implicit_all() {
    let make = |depth: usize| {
        let mut value = r#"{ "media.active" = true }"#.to_owned();
        for _ in 1..depth {
            value = format!("{{not = {value}}}");
        }
        format!("when = {value}")
    };
    assert!(parse(&make(8)).is_ok());
    let source = make(9);
    assert!(
        parse_document::<Document>("profile.toml".as_ref(), &source, |d| d
            .when
            .to_condition("when")
            .map(|_| ()))
        .is_err()
    );
}
#[test]
fn manifest_declares_profile_files_and_checks_duplicates_and_paths() {
    let base =
        "id = 'profile-default'\nversion = '1'\napi = '1'\nname = 'Default'\ndescription = ''\n";
    let source = format!("{base}[[profiles]]\nid='default'\nfile='profiles/default.toml'\n");
    let p = touchbar_sets::PluginManifest::from_toml("plugin.toml", &source).unwrap();
    assert_eq!(p.profiles[0].file, "profiles/default.toml");
    assert_eq!(p.profiles[0].id, "default");
    for suffix in [
        "[[profiles]]\nid='default'\nfile='profiles/other.toml'\n",
        "[[profiles]]\nid='other'\nfile='profiles/default.toml'\n",
    ] {
        assert!(
            touchbar_sets::PluginManifest::from_toml("plugin.toml", &format!("{source}{suffix}"))
                .is_err()
        );
    }
    assert!(
        touchbar_sets::PluginManifest::from_toml(
            "plugin.toml",
            &source.replace("profiles/default.toml", "../default.toml")
        )
        .is_err()
    );
}
#[test]
fn logical_array_and_implicit_all_budgets_are_enforced() {
    for value in [
        "{}",
        "{all=[]}",
        "{any=[]}",
        "{exists=1}",
        "{not=true}",
        "{'app.id'=false,all=[{exists='app.id'}]}",
    ] {
        let source = format!("when={value}");
        assert!(
            parse_document::<Document>("profile.toml".as_ref(), &source, |d| d
                .when
                .to_condition("when")
                .map(|_| ()))
            .is_err()
        );
    }
    let mut value = "{'a'=true,'b'=true}".to_owned();
    for _ in 1..8 {
        value = format!("{{not={value}}}");
    }
    let source = format!("when={value}");
    assert!(
        parse_document::<Document>("profile.toml".as_ref(), &source, |d| d
            .when
            .to_condition("when")
            .map(|_| ()))
        .is_err()
    );
    for count in [127, 128] {
        let source = format!(
            "when={{all=[{}]}}",
            vec!["{exists='app.id'}"; count].join(",")
        );
        assert_eq!(
            parse_document::<Document>("profile.toml".as_ref(), &source, |d| d
                .when
                .to_condition("when")
                .map(|_| ()))
            .is_ok(),
            count == 127
        );
    }
}

#[test]
fn explicit_manifest_assets_are_strict_and_bounded() {
    let base = "id='images'\nversion='1'\napi='1'\nname='Images'\ndescription=''\n";
    let asset = "\n[[assets]]\nid='speaker'\nfile='assets/speaker.svg'\nkind='symbolic-svg'\nwidth=32\nheight=32\n";
    let manifest =
        touchbar_sets::PluginManifest::from_toml("plugin.toml", &format!("{base}{asset}")).unwrap();
    assert_eq!(manifest.assets.len(), 1);
    for invalid in [
        asset.replace("32", "0"),
        asset.replace("32", "4096"),
        asset.replace("symbolic-svg", "svg"),
        asset.replace("assets/speaker.svg", "../outside.svg"),
        format!("{asset}{asset}"),
    ] {
        assert!(
            touchbar_sets::PluginManifest::from_toml("plugin.toml", &format!("{base}{invalid}"))
                .is_err()
        );
    }
}
