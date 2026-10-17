use touchbar_profiles::*;
use touchbar_sets::DocumentError;
const PROFILE: &str = r#"version=1
id="media"
for={category="media-player"}
when={"media.active"=true}
[[element]]
kind="slot"
id="transport"
items=["media/transport"]
[[element]]
kind="fixed-space"
width=20
[[element]]
kind="flexible-space"
minimum=27
weight=1
[[element]]
kind="group"
id="levels"
layout="equal-width"
spacing=13
element=[{kind="slot",id="volume",items=["controls/volume"]},{kind="slot",id="brightness",items=["controls/brightness"]}]
"#;
#[test]
fn documented_profile_roundtrips_and_retains_declaration_order() {
    let file = ProfileFile::from_toml("profiles/media.toml", PROFILE).unwrap();
    assert_eq!(file.id, "media");
    assert_eq!(file.elements.len(), 4);
    let json = serde_json::to_vec(&file).unwrap();
    let restored: ProfileFile = serde_json::from_slice(&json).unwrap();
    assert_eq!(file, restored);
    let toml = toml::to_string(&file).unwrap();
    assert_eq!(
        file,
        ProfileFile::from_toml("profiles/media.toml", &toml).unwrap()
    );
}
fn reject(source: &str, path: &str) -> DocumentError {
    let e = ProfileFile::from_toml("profiles/bad.toml", source).unwrap_err();
    assert_eq!(e.file.to_str(), Some("profiles/bad.toml"));
    assert!(e.path.starts_with(path), "{e}");
    e
}
#[test]
fn profile_schema_is_strict_and_rejects_invalid_selection_and_elements() {
    for (source, path) in [
        ("version=2\nid='main'", "version"),
        ("version=1\nid='bad/name'", "id"),
        (
            "version=1\nid='main'\ndefault=true\nfor={app='firefox'}",
            "for",
        ),
        ("version=1\nid='main'\nfor={locked=false}", "for"),
        ("version=1\nid='main'\nfor={app=''}", "for"),
        ("version=1\nid='main'\nfor={app='x',category='y'}", "for"),
        ("version=1\nid='main'\noops=true", "oops"),
        (
            "version=1\nid='main'\ninclude='p/base'\n[[element]]\nkind='fixed-space'\nwidth=10",
            "include",
        ),
        (
            "version=1\nid='main'\n[[element]]\nkind='slot'\nid='x'\nitems=[]",
            "element[0].items",
        ),
        (
            "version=1\nid='main'\n[[element]]\nkind='flexible-space'\nweight=0",
            "element[0].weight",
        ),
        (
            "version=1\nid='main'\n[[element]]\nkind='fixed-space'\nwidth='200%w'",
            "element[0]",
        ),
        ("version=1\nid='main'\n[units]\nu='0%h'", "units.u"),
    ] {
        reject(source, path);
    }
    let e = reject(
        "version=1\nid='main'\n\n[[element]]\nkind='flexible-space'\nweight=0",
        "element[0].weight",
    );
    assert_eq!(e.line, 6);
}
#[test]
fn duplicate_nested_slots_groups_and_candidate_sets_are_rejected() {
    reject(
        &PROFILE.replace("id=\"brightness\"", "id=\"volume\""),
        "element[3].element[1].id",
    );
    reject(
        "version=1\nid='main'\n[[element]]\nkind='slot'\nid='a'\nitems=['p/a','p/a']",
        "element[0].items[1]",
    );
    reject(
        "version=1\nid='main'\n[[element]]\nkind='group'\nid='a'\nelement=[{kind='group',id='a'}]",
        "element[0].element[0].id",
    );
}
#[test]
fn group_depth_is_bounded_at_eight() {
    let source = |depth| {
        let mut element = "{kind='slot',id='x',items=['p/x']}".to_owned();
        for i in 0..depth {
            element = format!("{{kind='group',id='g{i}',element=[{element}]}}");
        }
        format!("version=1\nid='main'\nelement=[{element}]")
    };
    assert!(ProfileFile::from_toml("profile.toml", &source(8)).is_ok());
    assert!(ProfileFile::from_toml("profile.toml", &source(9)).is_err());
}
#[test]
fn includes_share_layout_but_keep_local_selection_and_unit_override() {
    use std::collections::BTreeMap;
    let base = ProfileDocument::from_toml("profiles/media.toml", PROFILE).unwrap();
    let alias = ProfileDocument::from_toml(
        "profiles/browser.toml",
        "version=1\nid='browser'\nfor={category='browser'}\ninclude='p/media'\n[units]\nu='2%h'",
    )
    .unwrap();
    let documents = BTreeMap::from([
        ("p/media".parse().unwrap(), base),
        ("p/browser".parse().unwrap(), alias),
    ]);
    let resolved = resolve_profiles(&documents).unwrap();
    let base = &resolved[&"p/media".parse().unwrap()];
    let alias = &resolved[&"p/browser".parse().unwrap()];
    assert_eq!(alias.elements, base.elements);
    assert_eq!(alias.spec.slots, base.spec.slots);
    assert_eq!(
        alias.spec.for_target,
        Some(touchbar_model::ProfileTarget::Category("browser".into()))
    );
    assert!(alias.spec.when.is_none());
    assert_eq!(alias.spec.units.unwrap().value, 2.0);
    assert_eq!(
        alias
            .spec
            .slots
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["transport", "volume", "brightness"]
    );
}
#[test]
fn missing_include_cycles_and_chain_depth_are_reported_at_include_source() {
    use std::collections::BTreeMap;
    let make = |id: &str, include: &str| {
        ProfileDocument::from_toml(
            format!("profiles/{id}.toml"),
            &format!("version=1\nid='{id}'\n\ninclude='p/{include}'"),
        )
        .unwrap()
    };
    let documents = BTreeMap::from([("p/a".parse().unwrap(), make("a", "b"))]);
    let e = resolve_profiles(&documents).unwrap_err();
    assert_eq!(e.file.to_str(), Some("profiles/a.toml"));
    assert_eq!(e.path, "include");
    assert_eq!(e.line, 4);
    let mut cycle = documents.clone();
    cycle.insert("p/b".parse().unwrap(), make("b", "a"));
    assert!(
        resolve_profiles(&cycle)
            .unwrap_err()
            .message
            .contains("cycle")
    );
    for n in [8, 9] {
        let mut docs = BTreeMap::new();
        for i in 0..n {
            let id = format!("p{i}");
            let doc = if i + 1 < n {
                make(&id, &format!("p{}", i + 1))
            } else {
                ProfileDocument::from_toml(
                    format!("profiles/{id}.toml"),
                    &format!("version=1\nid='{id}'"),
                )
                .unwrap()
            };
            docs.insert(format!("p/{id}").parse().unwrap(), doc);
        }
        assert_eq!(resolve_profiles(&docs).is_ok(), n == 8);
    }
}
#[test]
fn shipped_default_profile_matches_manifest_and_stage_three_composition() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/profile-default");
    let manifest = touchbar_sets::PluginManifest::from_toml(
        root.join("plugin.toml"),
        &std::fs::read_to_string(root.join("plugin.toml")).unwrap(),
    )
    .unwrap();
    let mut docs = std::collections::BTreeMap::new();
    for profile in &manifest.profiles {
        let path = root.join(&profile.file);
        docs.insert(
            format!("{}/{}", manifest.id, profile.id).parse().unwrap(),
            ProfileDocument::from_toml(&path, &std::fs::read_to_string(&path).unwrap()).unwrap(),
        );
    }
    let result = resolve_profiles(&docs).unwrap();
    let profile = &result[&"profile-default/default".parse().unwrap()];
    assert!(profile.spec.default);
    assert_eq!(
        profile
            .spec
            .slots
            .iter()
            .map(|s| s.candidates[0].to_string())
            .collect::<Vec<_>>(),
        [
            "controls/volume",
            "controls/brightness",
            "controls/microphone"
        ]
    );
}
#[test]
fn internally_tagged_element_unknown_fields_report_the_actual_key_line() {
    let e = ProfileFile::from_toml(
        "profile.toml",
        "version=1\nid='main'\n[[element]]\nkind='slot'\nid='x'\nitems=['p/x']\ntyppo=1",
    )
    .unwrap_err();
    assert_eq!(e.path, "element[0].typpo");
    assert_eq!(e.line, 7);
}
#[test]
fn profile_size_and_identifier_budgets_are_checked() {
    let source = format!(
        "version=1\nid='main'\nelement=[{{kind='slot',id='{}',items=['p/x']}}]",
        "x".repeat(129)
    );
    reject(&source, "element[0].id");
    let spaces = |n| {
        format!(
            "version=1\nid='main'\nelement=[{}]",
            vec!["{kind='fixed-space',width=0}"; n].join(",")
        )
    };
    assert!(ProfileFile::from_toml("profile.toml", &spaces(512)).is_ok());
    assert!(ProfileFile::from_toml("profile.toml", &spaces(513)).is_err());
    let slots = |n| {
        format!(
            "version=1\nid='main'\nelement=[{}]",
            (0..n)
                .map(|i| format!("{{kind='slot',id='s{i}',items=['p/x']}} "))
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    assert!(ProfileFile::from_toml("profile.toml", &slots(256)).is_ok());
    assert!(ProfileFile::from_toml("profile.toml", &slots(257)).is_err());
}
