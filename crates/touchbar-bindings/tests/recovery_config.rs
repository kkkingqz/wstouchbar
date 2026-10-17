use touchbar_bindings::config::{BindingFile, RecoveryAction};
use touchbar_model::{FactName, FactValue};
const PREFIX: &str = "[[binding]]\nid='recovery'\ntrigger={key='fn',gesture='hold',ms=8000}\n";
#[test]
fn recovery_actions_and_default_trigger_use_shared_binding_grammar() {
    for (text, action) in [
        ("native", RecoveryAction::Native),
        ("display", RecoveryAction::Display),
        ("toggle-recovery", RecoveryAction::ToggleRecovery),
    ] {
        let source = format!("{PREFIX}action='{text}'\nwhen={{mode='native',recovery=true}}");
        let file = BindingFile::<RecoveryAction>::from_toml("touchbard.toml", &source).unwrap();
        let bindings = file.to_bindings().unwrap();
        assert_eq!(bindings[0].action, action);
        let facts = [
            (FactName::from("mode"), FactValue::Text("native".into())),
            (FactName::from("recovery"), FactValue::Boolean(true)),
        ]
        .into();
        assert!(bindings[0].when.as_ref().unwrap().evaluate(&facts));
    }
}
#[test]
fn daemon_conditions_reject_foreign_facts_even_when_nested_or_exists() {
    for condition in [
        "{app='terminal'}",
        "{exists='session.locked'}",
        "{not={any=[{mode='native'},{'surface.kind'='touchbar'}]}}",
        "{mode=true}",
        "{mode='typo'}",
        "{recovery='true'}",
        "{recovery={in=[true,'false']}}",
    ] {
        let source = format!("{PREFIX}action='native'\nwhen={condition}");
        let error =
            BindingFile::<RecoveryAction>::from_toml("touchbard.toml", &source).unwrap_err();
        assert_eq!(error.file, std::path::Path::new("touchbard.toml"));
        assert!(error.path.contains("when"), "{error:?}");
    }
    let source = format!(
        "{PREFIX}action='native'\nwhen={{all=[{{exists='mode'}},{{not={{recovery=false}}}}]}}"
    );
    assert!(BindingFile::<RecoveryAction>::from_toml("touchbard.toml", &source).is_ok());
}
