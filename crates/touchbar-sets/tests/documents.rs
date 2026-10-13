use touchbar_model::{ActionKind, Catalog, PluginSpec, StateBinding, SystemKey, WidgetKind};
use touchbar_sets::{PluginManifest, SetFile};

const HEADER: &str = "version = 1\nid = 'main'\nlabel = 'Main'\n";
const MANIFEST: &str = "id='owner'\nversion='1.0.0'\napi='1'\nname='Owner'\ndescription='Example'\nlogic='logic.wasm'\n[[sets]]\nid='main'\nfile='sets/main.toml'\n";
fn set(body: &str) -> String {
    format!("{HEADER}{body}")
}
fn parse(body: &str) -> SetFile {
    SetFile::from_toml("sets/main.toml", &set(body)).unwrap()
}
fn plugin() -> PluginManifest {
    PluginManifest::from_toml("plugin.toml", MANIFEST).unwrap()
}
fn reject(body: &str, path: &str, message: &str) {
    let error = SetFile::from_toml("sets/main.toml", &set(body)).unwrap_err();
    assert_eq!(error.file.to_str(), Some("sets/main.toml"));
    assert_eq!(error.path, path, "{error}");
    assert!(error.line >= 4, "{error}");
    assert!(error.message.contains(message), "{error}");
}
macro_rules! invalid {
    ($name:ident, $body:expr, $path:expr, $message:expr) => {
        #[test]
        fn $name() {
            reject($body, $path, $message);
        }
    };
}
invalid!(
    unknown_root,
    "oops=true\n[layout]\nkind='text'",
    "oops",
    "unknown field"
);
invalid!(
    unknown_node,
    "[layout]\nkind='text'\noops=true",
    "layout.oops",
    "unknown field"
);
invalid!(
    numeric_widget_id_is_rejected,
    "[layout]\nkind='button'\nwidget_id=1",
    "layout.widget_id",
    "unknown field"
);
invalid!(
    canvas_requires_source,
    "[layout]\nkind='canvas'",
    "layout.canvas_from",
    "requires"
);
invalid!(
    canvas_source_only_on_canvas,
    "[layout]\nkind='text'\ncanvas_from='drawing'",
    "layout.canvas_from",
    "canvas"
);
invalid!(
    long_press_requires_hold,
    "[layout]\nkind='button'\nid='button'\non_long_pressed='hold'",
    "layout.hold_ms",
    "requires"
);
invalid!(
    hold_panel_requires_hold,
    "[layout]\nkind='button'\nid='button'\npress_and_hold_panel='panel'",
    "layout.hold_ms",
    "requires"
);
invalid!(
    hold_must_be_positive,
    "[layout]\nkind='button'\nhold_ms=0",
    "layout.hold_ms",
    "duration"
);
invalid!(
    hold_duration_bounded,
    "[layout]\nkind='button'\nhold_ms=60001",
    "layout.hold_ms",
    "duration"
);
invalid!(
    handlers_require_interactive_node,
    "[layout]\nkind='text'\nid='text'\non_activate='go'",
    "layout.on_activate",
    "interactive"
);
invalid!(
    handlers_require_explicit_id,
    "[layout]\nkind='button'\non_activate='go'",
    "layout.id",
    "id"
);
invalid!(
    value_changed_requires_slider,
    "[layout]\nkind='button'\nid='b'\non_value_changed='go'",
    "layout.on_value_changed",
    "slider"
);
invalid!(
    invalid_action_name,
    "[layout]\nkind='button'\nid='b'\non_activate='../go'",
    "layout.on_activate",
    "identifier"
);
invalid!(
    invalid_widget_name,
    "[layout]\nkind='text'\nid='../x'",
    "layout.id",
    "identifier"
);
invalid!(
    duplicate_widget_id,
    "[layout]\nkind='row'\nchildren=[{kind='text',id='x'},{kind='text',id='x'}]",
    "layout.children[1].id",
    "duplicate"
);
invalid!(
    negative_length,
    "[layout]\nkind='text'\nwidth=-1",
    "layout.width",
    "bounds"
);
invalid!(
    percent_length_bounded,
    "[layout]\nkind='text'\nwidth='101%w'",
    "layout.width",
    "bounds"
);
invalid!(
    pixels_are_rejected,
    "[layout]\nkind='text'\nwidth='12px'",
    "layout.width",
    "unit"
);
invalid!(
    min_exceeds_max,
    "[layout]\nkind='text'\nminimum=20\nmaximum=10",
    "layout.minimum",
    "minimum"
);
invalid!(
    opacity_bounded,
    "[layout]\nkind='text'\nopacity=1.1",
    "layout.opacity",
    "opacity"
);
invalid!(
    disabled_opacity_bounded,
    "[layout]\nkind='text'\ndisabled_opacity=-0.1",
    "layout.disabled_opacity",
    "opacity"
);
invalid!(
    non_finite_value,
    "[layout]\nkind='slider'\nvalue=nan",
    "layout.value",
    "finite"
);
invalid!(
    non_finite_grow,
    "[layout]\nkind='text'\ngrow=inf",
    "layout.grow",
    "finite"
);
invalid!(
    symbol_is_known,
    "[layout]\nkind='icon'\nsymbol='unknown'",
    "layout.symbol",
    "symbol"
);
invalid!(
    asset_is_name,
    "[layout]\nkind='image'\nasset='../secret'",
    "layout.asset",
    "identifier"
);
invalid!(
    literal_and_binding_conflict,
    "[layout]\nkind='text'\ntext='X'\ntext_from='label'",
    "layout.text_from",
    "literal"
);
invalid!(
    color_literal_and_binding_conflict,
    "[layout]\nkind='text'\ncolor='accent'\ncolor_from='color'",
    "layout.color_from",
    "literal"
);
invalid!(
    value_literal_and_binding_conflict,
    "[layout]\nkind='slider'\nvalue=0.5\nvalue_from='value'",
    "layout.value_from",
    "literal"
);
invalid!(
    leaf_cannot_have_children,
    "[layout]\nkind='text'\nchildren=[{kind='text'}]",
    "layout.children",
    "children"
);
invalid!(
    button_has_at_most_one_child,
    "[layout]\nkind='button'\nchildren=[{kind='text'},{kind='text'}]",
    "layout.children",
    "children"
);
invalid!(
    repeat_requires_source,
    "[layout]\nkind='repeat'\nrepeat_key='id'\ntemplate={kind='text'}",
    "layout.repeat_from",
    "requires"
);
invalid!(
    repeat_requires_key,
    "[layout]\nkind='repeat'\nrepeat_from='rows'\ntemplate={kind='text'}",
    "layout.repeat_key",
    "requires"
);
invalid!(
    repeat_requires_template,
    "[layout]\nkind='repeat'\nrepeat_from='rows'\nrepeat_key='id'",
    "layout.template",
    "requires"
);
invalid!(
    repeat_properties_only_on_repeat,
    "[layout]\nkind='text'\nrepeat_from='rows'",
    "layout.repeat_from",
    "repeat"
);
invalid!(
    record_binding_requires_scope,
    "[layout]\nkind='text'\ntext_from='record.label'",
    "layout.text_from",
    "record"
);
invalid!(
    responsive_requires_variant,
    "[layout]\nkind='responsive'",
    "layout.variants",
    "variants"
);
invalid!(
    unknown_representation,
    "[layout]\nkind='responsive'\nvariants=[{representation='large',minimum_width=1,layout={kind='text'}}]",
    "layout.variants[0].representation",
    "representation"
);
invalid!(
    duplicate_representation,
    "[layout]\nkind='responsive'\nvariants=[{representation='full',minimum_width=1,layout={kind='text'}},{representation='full',minimum_width=2,layout={kind='text'}}]",
    "layout.variants[1].representation",
    "duplicate"
);
invalid!(
    variants_only_on_responsive,
    "[layout]\nkind='text'\nvariants=[{representation='full',minimum_width=1,layout={kind='text'}}]",
    "layout.variants",
    "responsive"
);
invalid!(
    shader_requires_effect,
    "[layout]\nkind='shader-effect'",
    "layout.effect",
    "requires"
);
invalid!(
    effect_only_on_shader,
    "[layout]\nkind='text'\neffect={id=1,source='abc'}",
    "layout.effect",
    "shader"
);
invalid!(
    effect_id_positive,
    "[layout]\nkind='shader-effect'\neffect={id=0,source='abc'}",
    "layout.effect.id",
    "positive"
);
invalid!(
    effect_period_bounded,
    "[layout]\nkind='shader-effect'\neffect={id=1,source='abc',period_ms=1}",
    "layout.effect.period_ms",
    "period"
);
invalid!(
    motion_id_positive,
    "[layout]\nkind='text'\nmotion={id=0,duration_ms=100,easing='linear',playback='once',from={},to={}}",
    "layout.motion.id",
    "positive"
);
invalid!(
    motion_duration_bounded,
    "[layout]\nkind='text'\nmotion={id=1,duration_ms=0,easing='linear',playback='once',from={},to={}}",
    "layout.motion.duration_ms",
    "duration"
);
invalid!(
    motion_easing_known,
    "[layout]\nkind='text'\nmotion={id=1,duration_ms=100,easing='bounce',playback='once',from={},to={}}",
    "layout.motion.easing",
    "easing"
);
invalid!(
    motion_playback_known,
    "[layout]\nkind='text'\nmotion={id=1,duration_ms=100,easing='linear',playback='forever',from={},to={}}",
    "layout.motion.playback",
    "playback"
);
invalid!(
    key_requires_key,
    "[layout]\nkind='key'",
    "layout.key",
    "requires"
);
invalid!(
    key_row_requires_keys,
    "[layout]\nkind='key-row'",
    "layout.keys",
    "requires"
);
invalid!(
    keys_only_on_key_row,
    "[layout]\nkind='row'\nkeys=[{key='f1'}]",
    "layout.keys",
    "key-row"
);
invalid!(
    unknown_key,
    "[layout]\nkind='key'\nkey='f13'",
    "layout.key",
    "unknown variant"
);
invalid!(
    unknown_panel_reference,
    "[layout]\nkind='button'\nid='b'\nexpanded_panel='missing'",
    "layout.expanded_panel",
    "unknown panel"
);
invalid!(
    old_panel_items_rejected,
    "[layout]\nkind='text'\n[[panels]]\nid='p'\nitems=['volume']\nlayout={kind='row'}",
    "panels[0].items",
    "unknown field"
);
invalid!(
    old_panel_elements_rejected,
    "[layout]\nkind='text'\n[[panels]]\nid='p'\nlayout={kind='row'}\n[[panels.element]]\nkind='item'\nitem='volume'",
    "panels[0].element",
    "unknown field"
);
invalid!(
    panel_requires_local_layout,
    "[layout]\nkind='text'\n[[panels]]\nid='p'",
    "panels[0].layout",
    "missing field"
);
invalid!(
    duplicate_panel_id,
    "[layout]\nkind='text'\n[[panels]]\nid='p'\nlayout={kind='row'}\n[[panels]]\nid='p'\nlayout={kind='row'}",
    "panels[1].id",
    "duplicate"
);
invalid!(
    panel_widget_ids_share_owner_namespace,
    "[layout]\nkind='text'\nid='x'\n[[panels]]\nid='p'\nlayout={kind='text',id='x'}",
    "panels[0].layout.id",
    "duplicate"
);
invalid!(
    principal_widget_must_belong_to_panel,
    "[layout]\nkind='text'\nid='x'\n[[panels]]\nid='p'\nprincipal_item='x'\nlayout={kind='row'}",
    "panels[0].principal_item",
    "panel"
);
invalid!(
    invalid_binding_identifier,
    "[layout]\nkind='text'\ntext_from='../bad'",
    "layout.text_from",
    "identifier"
);

#[test]
fn manifest_parses_and_checks_api() {
    let mut source = MANIFEST.to_string();
    assert_eq!(plugin().sets[0].id, "main");
    source = source.replace("api='1'", "api='2'");
    let error = PluginManifest::from_toml("plugin.toml", &source).unwrap_err();
    assert_eq!(error.path, "api");
    assert_eq!(error.line, 3);
}
#[test]
fn manifest_unknown_field_has_exact_location() {
    let error = PluginManifest::from_toml("plugin.toml", &(MANIFEST.to_string() + "typo=true\n"))
        .unwrap_err();
    assert_eq!(error.path, "sets[0].typo");
    assert_eq!(error.line, 10);
}
#[test]
fn versions_identifiers_labels_and_persist_are_checked() {
    for (source, path) in [
        (
            set("[layout]\nkind='text'").replace("version = 1", "version = 2"),
            "version",
        ),
        (
            set("[layout]\nkind='text'").replace("id = 'main'", "id = '../x'"),
            "id",
        ),
        (
            set("[layout]\nkind='text'").replace("label = 'Main'", "label = ''"),
            "label",
        ),
        (
            set("persist=['x','x']\n[layout]\nkind='text'"),
            "persist[1]",
        ),
    ] {
        assert_eq!(
            SetFile::from_toml("main.toml", &source).unwrap_err().path,
            path
        );
    }
}
#[test]
fn diagnostic_tracks_second_array_table_and_unicode_lines() {
    let source = set(
        "# ж\n[layout]\nkind='row'\n[[layout.children]]\nkind='text'\nid='one'\n[[layout.children]]\nkind='text'\nid='../two'",
    );
    let error = SetFile::from_toml("a.toml", &source).unwrap_err();
    assert_eq!(error.path, "layout.children[1].id");
    assert_eq!(error.line, 12);
    assert!(
        error
            .to_string()
            .contains("a.toml:12: layout.children[1].id")
    );
}
#[test]
fn relative_units_and_interaction_metadata_reach_core_subset() {
    let doc = parse(
        "[units]\nu='2%h'\n[layout]\nkind='button'\nid='open'\nwidth='40%w'\nvisible_from='visible'\non_activate='toggle'\npress_and_hold_panel='detail'\nhold_ms=300\n[[panels]]\nid='detail'\nprincipal_item='level'\ndismiss_on_selection=false\n[panels.layout]\nkind='slider'\nid='level'\nvalue_from='value'\non_value_changed='change'",
    );
    let spec = doc.to_spec(&plugin()).unwrap();
    assert_eq!(spec.reference.to_string(), "owner/main");
    assert_eq!(spec.logic, Some("owner".parse().unwrap()));
    assert_eq!(spec.units, Some(doc.units.unwrap().u));
    assert_eq!(spec.widgets[0].width, None);
    assert_eq!(spec.widgets[0].hold_ms, Some(300));
    assert_eq!(spec.widgets[0].handlers[&ActionKind::Activated], "toggle");
    assert_eq!(
        spec.widgets[0].state_bindings[&StateBinding::Visible],
        "visible"
    );
    assert_eq!(
        spec.panels[0].principal_item.as_ref().unwrap().as_str(),
        "level"
    );
    assert!(!spec.panels[0].dismiss_on_selection);
    let catalog = Catalog {
        plugins: vec![PluginSpec {
            id: plugin().id,
            revision: "a".into(),
            enabled: true,
            has_logic: true,
            requires: vec![],
        }],
        sets: vec![spec],
        ..Default::default()
    };
    assert!(catalog.validate().is_empty(), "{:?}", catalog.validate());
}
#[test]
fn fkeys_document_supports_esc_by_uncommenting_one_line() {
    let docs = include_str!("../../../docs/architecture/04-plugins.md");
    let source = docs
        .split("# plugins/system/sets/fkeys.toml\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    let mut owner = plugin();
    owner.logic = None;
    for (text, count) in [
        (source.to_string(), 12),
        (source.replace("# { key", "{ key"), 13),
    ] {
        let file = SetFile::from_toml("fkeys.toml", &text).unwrap();
        let spec = file.to_spec(&owner).unwrap();
        assert_eq!(spec.widgets[0].children.len(), count);
        assert!(matches!(
            spec.widgets[0].children.last().unwrap().kind,
            WidgetKind::Key {
                key: SystemKey::F12
            }
        ));
    }
}
#[test]
fn set_file_round_trips_all_layout_kinds() {
    let nodes = [
        "row",
        "column",
        "stack",
        "button",
        "text",
        "icon",
        "image",
        "slider",
        "meter",
        "progress",
        "scrubber",
        "fixed-space",
        "flexible-space",
    ];
    for kind in nodes {
        let doc = parse(&format!(
            "[layout]\nkind='{kind}'\nwidth='12u'\npadding='1%h'"
        ));
        let encoded = toml::to_string(&doc).unwrap();
        assert_eq!(SetFile::from_toml("roundtrip.toml", &encoded).unwrap(), doc);
    }
    for body in [
        "[layout]\nkind='repeat'\nrepeat_from='rows'\nrepeat_key='id'\ntemplate={kind='text',text_from='record.label'}",
        "[layout]\nkind='responsive'\nvariants=[{representation='full',minimum_width=10,layout={kind='text'}}]",
        "[layout]\nkind='canvas'\ncanvas_from='drawing'",
        "[layout]\nkind='key'\nkey='f1'",
        "[layout]\nkind='key-row'\nkeys=[{key='esc',width=150},{key='f1'}]",
        "[layout]\nkind='shader-effect'\neffect={id=1,source='shader',parameters=[0.5,'amount'],period_ms=100,opacity=0.5}\nmotion={id=2,duration_ms=100,easing='linear',playback='once',from={x=0.0},to={opacity='opacity'}}",
    ] {
        let doc = parse(body);
        let encoded = toml::to_string(&doc).unwrap();
        assert_eq!(SetFile::from_toml("roundtrip.toml", &encoded).unwrap(), doc);
    }
}
#[test]
fn no_logic_rejects_state_handlers_and_persist() {
    let mut owner = plugin();
    owner.logic = None;
    for body in [
        "[layout]\nkind='text'\ntext_from='label'",
        "[layout]\nkind='button'\nid='b'\non_activate='go'",
        "persist=['value']\n[layout]\nkind='text'",
        "[layout]\nkind='text'\ncolor_from='color'",
    ] {
        assert!(
            parse(body)
                .to_spec(&owner)
                .unwrap_err()
                .message
                .contains("logic")
        );
    }
}
#[test]
fn auto_ids_do_not_collide_with_user_ids_and_are_stable() {
    let file = parse(
        "[layout]\nkind='row'\nchildren=[{kind='text'},{kind='text',id='auto-layout-0'},{kind='key-row',keys=[{key='f1'},{key='f1'}]}]",
    );
    let spec = file.to_spec(&plugin()).unwrap();
    let ids = spec
        .all_widgets()
        .iter()
        .map(|w| w.id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), spec.all_widgets().len());
    assert_eq!(spec, file.to_spec(&plugin()).unwrap());
}
#[test]
fn files_and_tree_and_text_budgets_are_bounded() {
    let oversized = " ".repeat(256 * 1024 + 1);
    assert!(
        SetFile::from_toml("big.toml", &oversized)
            .unwrap_err()
            .message
            .contains("256 KiB")
    );
    let huge_text = format!("[layout]\nkind='text'\ntext='{}'", "a".repeat(16385));
    reject(&huge_text, "layout.text", "text budget");
    let nodes = format!(
        "[layout]\nkind='row'\nchildren=[{}]",
        vec!["{kind='text'}"; 256].join(",")
    );
    assert!(
        SetFile::from_toml("many.toml", &set(&nodes))
            .unwrap_err()
            .message
            .contains("node budget")
    );
    let mut node = "{kind='text'}".to_string();
    for _ in 0..33 {
        node = format!("{{kind='row',children=[{node}]}}");
    }
    assert!(
        SetFile::from_toml("deep.toml", &set(&format!("layout={node}")))
            .unwrap_err()
            .message
            .contains("depth")
    );
}

#[test]
fn every_shipped_manifest_and_set_parses_and_round_trips() {
    let plugins = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
    let mut sets = 0;
    for entry in std::fs::read_dir(plugins).unwrap() {
        let dir = entry.unwrap().path();
        let path = dir.join("plugin.toml");
        if !path.exists() {
            continue;
        }
        let manifest =
            PluginManifest::from_toml(&path, &std::fs::read_to_string(&path).unwrap()).unwrap();
        for item in &manifest.sets {
            let path = dir.join(&item.file);
            let set = SetFile::from_toml(&path, &std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(set.id, item.id);
            let encoded = toml::to_string(&set).unwrap();
            assert_eq!(SetFile::from_toml("roundtrip.toml", &encoded).unwrap(), set);
            set.to_spec(&manifest).unwrap();
            sets += 1;
        }
    }
    assert_eq!(sets, 4, "controls and system sets must be shipped");
}
#[test]
fn label_and_text_may_bind_different_fields_on_one_node() {
    let file = parse("[layout]\nkind='text'\nlabel_from='title'\ntext_from='subtitle'");
    assert_eq!(file.layout.label_from.as_deref(), Some("title"));
    assert_eq!(file.layout.text_from.as_deref(), Some("subtitle"));
}
fn description() -> touchbar_model::LogicDescription {
    use touchbar_model::{ActionDescription, FieldKind, LogicDescription, ScalarKind, StateField};
    LogicDescription {
        actions: vec![
            (
                "go".into(),
                ActionDescription {
                    kinds: vec![ActionKind::Activated],
                    value: None,
                },
            ),
            (
                "change".into(),
                ActionDescription {
                    kinds: vec![ActionKind::ValueChanged],
                    value: Some(ScalarKind::Number),
                },
            ),
        ],
        state: vec![
            StateField {
                name: "value".into(),
                kind: FieldKind::Scalar(ScalarKind::Number),
                persist: true,
            },
            StateField {
                name: "label".into(),
                kind: FieldKind::Scalar(ScalarKind::Text),
                persist: false,
            },
            StateField {
                name: "enabled".into(),
                kind: FieldKind::Scalar(ScalarKind::Boolean),
                persist: false,
            },
            StateField {
                name: "drawing".into(),
                kind: FieldKind::Canvas,
                persist: false,
            },
            StateField {
                name: "rows".into(),
                kind: FieldKind::Records(vec![
                    ("id".into(), ScalarKind::Text),
                    ("label".into(), ScalarKind::Text),
                ]),
                persist: false,
            },
        ],
    }
}
macro_rules! bad_binding {
    ($name:ident,$body:expr,$path:expr) => {
        #[test]
        fn $name() {
            let error =
                SetFile::from_toml_with_description("main.toml", &set($body), &description())
                    .unwrap_err();
            assert_eq!(error.path, $path, "{error}");
            assert!(error.line >= 4, "{error}");
        }
    };
}
bad_binding!(
    unknown_state_field,
    "[layout]\nkind='text'\ntext_from='missing'",
    "layout.text_from"
);
bad_binding!(
    wrong_scalar_type,
    "[layout]\nkind='text'\ntext_from='value'",
    "layout.text_from"
);
bad_binding!(
    wrong_render_binding_type,
    "[layout]\nkind='text'\ncolor_from='value'",
    "layout.color_from"
);
bad_binding!(
    pressed_binding_requires_boolean,
    "[layout]\nkind='button'\npressed_from='label'",
    "layout.pressed_from"
);
bad_binding!(
    unknown_action,
    "[layout]\nkind='button'\nid='b'\non_activate='missing'",
    "layout.on_activate"
);
bad_binding!(
    action_event_not_supported,
    "[layout]\nkind='button'\nid='b'\non_pressed='go'",
    "layout.on_pressed"
);
bad_binding!(
    canvas_requires_canvas_type,
    "[layout]\nkind='canvas'\ncanvas_from='rows'",
    "layout.canvas_from"
);
bad_binding!(
    repeat_requires_record_list_type,
    "[layout]\nkind='repeat'\nrepeat_from='label'\nrepeat_key='id'\ntemplate={kind='text'}",
    "layout.repeat_from"
);
bad_binding!(
    repeat_key_must_be_declared,
    "[layout]\nkind='repeat'\nrepeat_from='rows'\nrepeat_key='missing'\ntemplate={kind='text'}",
    "layout.repeat_key"
);
bad_binding!(
    unknown_record_field,
    "[layout]\nkind='repeat'\nrepeat_from='rows'\nrepeat_key='id'\ntemplate={kind='text',text_from='record.missing'}",
    "layout.template.text_from"
);
bad_binding!(
    shader_parameter_requires_number,
    "[layout]\nkind='shader-effect'\neffect={id=1,source='test',parameters=['label']}",
    "layout.effect.parameters[0]"
);
bad_binding!(
    transform_binding_requires_number,
    "[layout]\nkind='text'\nmotion={id=1,duration_ms=100,easing='linear',playback='once',from={},to={x='label'}}",
    "layout.motion.to.x"
);
bad_binding!(
    persist_requires_declared_persist_flag,
    "persist=['label']\n[layout]\nkind='text'",
    "persist[0]"
);
#[test]
fn typed_bindings_accept_repeat_canvas_and_persist() {
    for body in [
        "persist=['value']\n[layout]\nkind='slider'\nid='slider'\nvalue_from='value'\non_value_changed='change'\nenabled_from='enabled'",
        "[layout]\nkind='canvas'\ncanvas_from='drawing'",
        "[layout]\nkind='repeat'\nrepeat_from='rows'\nrepeat_key='id'\ntemplate={kind='text',text_from='record.label'}",
        "[layout]\nkind='text'\ncolor_from='label'\npressed_from='enabled'",
    ] {
        SetFile::from_toml_with_description("main.toml", &set(body), &description()).unwrap();
    }
}

#[test]
fn maximum_layout_depth_is_accepted_and_extreme_toml_nesting_is_rejected() {
    let mut node = "{kind='text'}".to_string();
    for _ in 0..32 {
        node = format!("{{kind='row',children=[{node}]}}");
    }
    SetFile::from_toml("max-depth.toml", &set(&format!("layout={node}"))).unwrap();
    let nested = format!("extra={}0{}", "[".repeat(1000), "]".repeat(1000));
    assert!(SetFile::from_toml("very-deep.toml", &nested).is_err());
}

invalid!(
    negative_spacing,
    "[layout]\nkind='row'\nspacing=-1",
    "layout.spacing",
    "bounds"
);
invalid!(
    invalid_units,
    "[units]\nu='0%h'\n[layout]\nkind='text'",
    "units.u",
    "positive"
);
invalid!(
    unknown_units_field,
    "[units]\nu='1%h'\npixels=1\n[layout]\nkind='text'",
    "units.pixels",
    "unknown field"
);
invalid!(
    unknown_key_row_field,
    "[layout]\nkind='key-row'\nkeys=[{key='f1',typo=1}]",
    "layout.keys[0].typo",
    "unknown field"
);
invalid!(
    unknown_effect_field,
    "[layout]\nkind='shader-effect'\neffect={id=1,source='x',typo=1}",
    "layout.effect.typo",
    "unknown field"
);
invalid!(
    unknown_motion_field,
    "[layout]\nkind='text'\nmotion={id=1,duration_ms=100,easing='linear',playback='once',from={},to={},typo=1}",
    "layout.motion.typo",
    "unknown field"
);
invalid!(
    unknown_transform_field,
    "[layout]\nkind='text'\nmotion={id=1,duration_ms=100,easing='linear',playback='once',from={},to={typo=1}}",
    "layout.motion.to.typo",
    "unknown field"
);
invalid!(
    unknown_variant_field,
    "[layout]\nkind='responsive'\nvariants=[{representation='full',minimum_width=1,layout={kind='text'},typo=1}]",
    "layout.variants[0].typo",
    "unknown field"
);
invalid!(
    duplicate_ids_across_responsive_variants,
    "[layout]\nkind='responsive'\nvariants=[{representation='minimal',minimum_width=0,layout={kind='text',id='x'}},{representation='full',minimum_width=10,layout={kind='text',id='x'}}]",
    "layout.variants[1].layout.id",
    "duplicate"
);
invalid!(
    panel_width_ordering,
    "[layout]\nkind='text'\n[[panels]]\nid='p'\nminimum_width=200\npreferred_width=100\nlayout={kind='text'}",
    "panels[0].minimum_width",
    "minimum"
);
invalid!(
    shader_parameter_count,
    "[layout]\nkind='shader-effect'\neffect={id=1,source='x',parameters=[1,2,3,4,5,6,7,8,9]}",
    "layout.effect.parameters",
    "parameters"
);
invalid!(
    shader_opacity,
    "[layout]\nkind='shader-effect'\neffect={id=1,source='x',opacity=1.1}",
    "layout.effect.opacity",
    "opacity"
);
invalid!(
    date_values_must_be_quoted,
    "[layout]\nkind='text'\ntext=2026-10-12",
    "layout.text",
    "quoted"
);

#[test]
fn manifest_paths_dependencies_and_duplicate_sets_are_checked() {
    for (source, path) in [
        (
            MANIFEST.replace("logic='logic.wasm'", "logic='../logic.wasm'"),
            "logic",
        ),
        (
            MANIFEST.replace("file='sets/main.toml'", "file='/main.toml'"),
            "sets[0].file",
        ),
        (
            MANIFEST.replace(
                "logic='logic.wasm'",
                "logic='logic.wasm'\nrequires=['owner']",
            ),
            "requires[0]",
        ),
        (
            format!("{MANIFEST}[[sets]]\nid='main'\nfile='sets/other.toml'"),
            "sets[1].id",
        ),
        (
            format!("{MANIFEST}[[sets]]\nid='other'\nfile='sets/main.toml'"),
            "sets[1].file",
        ),
    ] {
        let error = PluginManifest::from_toml("plugin.toml", &source).unwrap_err();
        assert_eq!(error.path, path, "{error}");
    }
}
