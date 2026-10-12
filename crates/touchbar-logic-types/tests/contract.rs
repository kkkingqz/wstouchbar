use std::path::Path;
use wit_parser::{Resolve, WorldItem, WorldKey};

#[test]
fn v1_contract_has_one_versioned_api_and_only_logging_capability() {
    let mut resolve = Resolve::default();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../wit");
    let parsed = resolve.push_path(path);
    assert!(parsed.is_ok(), "contract must parse: {parsed:?}");
    let (package, _) = parsed.unwrap();
    assert_eq!(
        resolve.packages[package].name.to_string(),
        "wstouchbar:logic@1.0.0"
    );
    let world = &resolve.worlds[resolve.packages[package].worlds["logic"]];
    let callable_imports: Vec<_> = world
        .imports
        .iter()
        .filter_map(|(key, item)| match item {
            WorldItem::Function(_) => Some(resolve.name_world_key(key)),
            WorldItem::Type { .. } => None,
            WorldItem::Interface { .. } => panic!("unexpected interface capability"),
        })
        .collect();
    assert_eq!(callable_imports, ["log"]);
    assert_eq!(world.exports.len(), 1);
    let (WorldKey::Interface(api), WorldItem::Interface { .. }) = world.exports.first().unwrap()
    else {
        panic!("API must be a namespaced interface");
    };
    assert_eq!(
        resolve.id_of(*api).as_deref(),
        Some("wstouchbar:logic/api@1.0.0")
    );
    let api = &resolve.interfaces[*api];
    assert_eq!(
        api.functions.keys().map(String::as_str).collect::<Vec<_>>(),
        ["describe", "init", "handle"]
    );
    assert_eq!(api.functions["init"].params.len(), 3);
    assert_eq!(api.functions["handle"].params.len(), 2);
}
