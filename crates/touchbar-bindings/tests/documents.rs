use touchbar_bindings::config::*;
use touchbar_model::{KeyGesture, LayerOperation, LayerSurfaces, Trigger};
fn file(trigger: &str, action: &str) -> String {
    format!("[[binding]]\nid='fn-layer'\ntrigger={trigger}\naction={action}\n")
}
fn layer(source: &str) -> BindingFile<LayerActionFile> {
    BindingFile::from_toml("bindings.toml", source).unwrap()
}
#[test]
fn documented_triggers_and_layer_actions_project_to_engine_types() {
    for (trigger, gesture) in [
        ("{key='fn',gesture='while-held'}", KeyGesture::WhileHeld),
        (
            "{keys=['fn','esc'],gesture='hold',ms=3000}",
            KeyGesture::Hold { ms: 3000 },
        ),
        (
            "{key='fn',gesture='multi-tap',count=2,within-ms=300}",
            KeyGesture::MultiTap {
                count: 2,
                within_ms: 300,
                tap_ms: 300,
            },
        ),
        ("{key='fn',gesture='tap'}", KeyGesture::Tap { ms: 300 }),
        ("{key='fn',gesture='press'}", KeyGesture::Press),
        ("{key='fn',gesture='release'}", KeyGesture::Release),
    ] {
        let f = layer(&file(trigger, "{layer='system/fkeys'}"));
        let bindings = f.to_bindings().unwrap();
        assert!(matches!(&bindings[0].trigger,Trigger::Key(t) if t.gesture==gesture));
        assert_eq!(
            bindings[0].action.operation,
            LayerOperation::Layer("system/fkeys".parse().unwrap())
        );
        assert_eq!(bindings[0].action.surfaces, LayerSurfaces::Source);
        let json = serde_json::to_vec(&f).unwrap();
        let restored: BindingFile<LayerActionFile> = serde_json::from_slice(&json).unwrap();
        assert_eq!(restored, f);
        assert_eq!(layer(&toml::to_string(&f).unwrap()), f);
    }
    let f = layer(&file(
        "{sequence=[{key='fn',gesture='tap'},{key='esc',gesture='tap'}],within-ms=800}",
        "{toggle-layer='system/fkeys',surfaces='all'}",
    ));
    assert!(
        matches!(&f.to_bindings().unwrap()[0].trigger,Trigger::Sequence {steps,within_ms:800} if steps.len()==2)
    );
    assert_eq!(
        f.to_bindings().unwrap()[0].action.surfaces,
        LayerSurfaces::All
    );
    let f = layer(&file(
        "{key='fn',gesture='press'}",
        "{clear-layers=true,surfaces=['touchbar','window:tools']}",
    ));
    assert!(matches!(
        f.to_bindings().unwrap()[0].action.surfaces,
        LayerSurfaces::Names(_)
    ));
}
#[test]
fn invalid_trigger_shapes_timing_keys_and_parameters_are_rejected() {
    for trigger in [
        "{key='fn',keys=['esc'],gesture='press'}",
        "{keys=[],gesture='press'}",
        "{keys=['fn','fn'],gesture='press'}",
        "{key='fn'}",
        "{key='unknown',gesture='press'}",
        "{key='fn',gesture='hold'}",
        "{key='fn',gesture='hold',ms=0}",
        "{key='fn',gesture='tap',ms=0}",
        "{key='fn',gesture='multi-tap',count=1,within-ms=300}",
        "{key='fn',gesture='multi-tap',count=2,within-ms=0}",
        "{key='fn',gesture='press',ms=1}",
        "{key='fn',gesture='tap',count=2}",
        "{sequence=[],within-ms=300}",
        "{sequence=[{key='fn',gesture='while-held'}],within-ms=300}",
        "{sequence=[{key='fn',gesture='tap'}]}",
        "{sequence=[{key='fn',gesture='tap'}],within-ms=300,key='fn'}",
        "{sequence=[{sequence=[{key='fn',gesture='tap'}],within-ms=300}],within-ms=300}",
        "{key='fn',gesture='press',oops=true}",
    ] {
        let e = BindingFile::<LayerActionFile>::from_toml(
            "bindings.toml",
            &file(trigger, "{layer='system/fkeys'}"),
        )
        .unwrap_err();
        assert_eq!(e.file.to_str(), Some("bindings.toml"));
        assert!(e.path.starts_with("binding[0].trigger"), "{e}");
    }
}
#[test]
fn actions_and_disabled_overrides_are_strict() {
    for action in [
        "{}",
        "{layer='system/fkeys',clear-layers=true}",
        "{clear-layers=false}",
        "{layer='bad'}",
        "{layer='system/fkeys',surfaces='one'}",
        "{layer='system/fkeys',surfaces=[]}",
        "{layer='system/fkeys',surfaces=['touchbar','touchbar']}",
        "{layer='system/fkeys',oops=true}",
    ] {
        assert!(
            BindingFile::<LayerActionFile>::from_toml(
                "bindings.toml",
                &file("{key='fn',gesture='press'}", action)
            )
            .is_err()
        );
    }
    assert!(
        BindingFile::<LayerActionFile>::from_toml(
            "bindings.toml",
            "[[binding]]\nid='x'\ndisabled=true"
        )
        .unwrap()
        .to_bindings()
        .unwrap()
        .is_empty()
    );
    assert!(
        BindingFile::<LayerActionFile>::from_toml(
            "bindings.toml",
            "[[binding]]\nid='x'\ndisabled=true\ntrigger={key='fn',gesture='press'}"
        )
        .is_err()
    );
    assert!(
        BindingFile::<LayerActionFile>::from_toml(
            "bindings.toml",
            "[[binding]]\nid=''\ndisabled=true"
        )
        .is_err()
    );
}
#[test]
fn user_overrides_replace_at_the_original_position_disable_and_append() {
    let a = layer(&file(
        "{key='fn',gesture='while-held'}",
        "{layer='system/fkeys'}",
    ));
    let b = layer(
        &file("{key='esc',gesture='tap'}", "{clear-layers=true}").replace("fn-layer", "esc-clear"),
    );
    let user = layer(
        &(file(
            "{key='fn',gesture='hold',ms=2000}",
            "{toggle-layer='system/fkeys'}",
        ) + "[[binding]]\nid='esc-clear'\ndisabled=true\n"
            + &file("{key='f1',gesture='press'}", "{clear-layers=true}")
                .replace("fn-layer", "new")),
    );
    let merged = merge_bindings(&[a, b], Some(&user)).unwrap();
    assert_eq!(
        merged.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(),
        ["fn-layer", "new"]
    );
    assert!(matches!(
        merged[0].action.operation,
        LayerOperation::ToggleLayer(_)
    ));
}
#[test]
fn parser_is_shared_with_an_independent_action_type() {
    #[derive(Clone, serde::Deserialize)]
    #[serde(rename_all = "kebab-case")]
    enum Recovery {
        ToggleRecovery,
    }
    impl ActionDocument for Recovery {
        type Action = u8;
        fn to_action(&self) -> Result<u8, String> {
            Ok(1)
        }
    }
    let f = BindingFile::<Recovery>::from_toml(
        "touchbard.toml",
        &file("{key='fn',gesture='hold',ms=8000}", "'toggle-recovery'"),
    )
    .unwrap();
    assert_eq!(f.to_bindings().unwrap()[0].action, 1);
}
#[test]
fn conditions_and_binding_conflicts_use_the_common_contract() {
    let source =
        file("{key='fn',gesture='press'}", "{layer='system/fkeys'}") + "when={exists='app.id'}\n";
    assert!(layer(&source).to_bindings().unwrap()[0].when.is_some());
    let duplicate = source.clone() + &source;
    assert!(BindingFile::<LayerActionFile>::from_toml("bindings.toml", &duplicate).is_err());
    let conflict = source
        + &file("{key='fn',gesture='press'}", "{clear-layers=true}").replace("fn-layer", "other")
        + "when={exists='app.id'}\n";
    assert!(BindingFile::<LayerActionFile>::from_toml("bindings.toml", &conflict).is_err());
}
#[test]
fn shipped_system_binding_is_valid_and_observes_fn_while_held() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/system/bindings.toml");
    let f =
        BindingFile::<LayerActionFile>::from_toml(&path, &std::fs::read_to_string(&path).unwrap())
            .unwrap();
    assert_eq!(f.to_bindings().unwrap()[0].id, "fn-layer");
}
#[test]
fn conflicts_after_disabled_rows_report_the_original_row() {
    let source = "[[binding]]\nid='disabled'\ndisabled=true\n".to_owned()
        + &file("{key='fn',gesture='press'}", "{layer='system/fkeys'}")
        + &file("{key='fn',gesture='press'}", "{clear-layers=true}").replace("fn-layer", "other");
    let e = BindingFile::<LayerActionFile>::from_toml("bindings.toml", &source).unwrap_err();
    assert_eq!(e.path, "binding[2]");
    assert_eq!(e.line, 8);
}
