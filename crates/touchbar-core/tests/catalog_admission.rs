use touchbar_core::{Core, CoreConfig, Input};
use touchbar_model::*;
#[test]
fn catalog_admission_is_pure_and_matches_input_admission() {
    let mut core = Core::new(CoreConfig {
        max_update_bytes: 20,
        ..Default::default()
    });
    let before = serde_json::to_value(core.checkpoint()).unwrap();
    let mut catalog = Catalog::default();
    catalog.plugins.push(PluginSpec {
        id: "p".parse().unwrap(),
        revision: "r".into(),
        enabled: true,
        has_logic: false,
        requires: vec![],
    });
    let mut set = SetSpec::default_for("p/set".parse().unwrap());
    set.logic_config = "x".repeat(30);
    catalog.sets.push(set);
    assert!(core.check_catalog(&catalog).is_err());
    assert_eq!(serde_json::to_value(core.checkpoint()).unwrap(), before);
    core.handle(Timestamp(0), Input::Catalog(catalog));
    assert_eq!(serde_json::to_value(core.checkpoint()).unwrap(), before);
    assert!(core.check_catalog(&Catalog::default()).is_ok());
}
