use touchbar_wasm_fixtures as fixtures;
use wit_component::{DecodedWasm, decode};
use wit_parser::{WorldItem, WorldKey};

#[test]
fn fixtures_are_components_with_versioned_api_and_expected_capabilities() {
    for (name, path) in [
        ("controls", fixtures::CONTROLS),
        ("echo", fixtures::ECHO),
        ("trap", fixtures::TRAP),
        ("hang", fixtures::HANG),
        ("oom", fixtures::OOM),
        ("err", fixtures::ERR),
        ("budget", fixtures::BUDGET),
        ("log", fixtures::LOG),
        ("persist", fixtures::PERSIST),
        ("version", fixtures::VERSION),
        ("wasi", fixtures::WASI),
    ] {
        let bytes =
            std::fs::read(path).unwrap_or_else(|e| panic!("{name} component must exist: {e}"));
        let DecodedWasm::Component(resolve, world) = decode(&bytes).unwrap() else {
            panic!("{name} must be a component");
        };
        let world = &resolve.worlds[world];
        let exported_apis: Vec<_> = world
            .exports
            .keys()
            .filter_map(|key| {
                if let WorldKey::Interface(id) = key {
                    resolve.id_of(*id)
                } else {
                    None
                }
            })
            .collect();
        let expected = if name == "version" {
            "wstouchbar:logic/api@9.0.0"
        } else {
            "wstouchbar:logic/api@1.0.0"
        };
        assert_eq!(exported_apis, [expected], "{name}");
        let capabilities: Vec<_> = world
            .imports
            .iter()
            .filter_map(|(key, item)| match item {
                WorldItem::Function(_) | WorldItem::Interface { .. } => {
                    Some(resolve.name_world_key(key))
                }
                WorldItem::Type { .. } => None,
            })
            .collect();
        if name == "wasi" {
            assert!(capabilities.iter().any(|name| name.starts_with("wasi:")));
        } else {
            assert!(
                capabilities.iter().all(|name| name == "log"),
                "{name}: {capabilities:?}"
            );
        }
    }
}

#[path = "../build_support.rs"]
mod build_support;

#[test]
fn nested_build_does_not_inherit_host_rustflags_or_cfg() {
    if std::env::var_os("TOUCHBAR_WASM_FIXTURES").is_some() {
        eprintln!("prebuilt fixtures: nested cargo check skipped");
        return;
    }
    if std::env::var_os("TOUCHBAR_BUILD_ENV_TEST_CHILD").is_some() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let output = root
            .join("target")
            .join(format!("fixture-env-test-{}", std::process::id()));
        std::fs::create_dir_all(&output).unwrap();
        let result = build_support::build(&root, &output);
        let paths = result.unwrap();
        assert_eq!(paths.len(), 11);
        for (_, path) in paths {
            assert!(matches!(
                decode(&std::fs::read(path).unwrap()).unwrap(),
                DecodedWasm::Component(..)
            ));
        }
        std::fs::remove_dir_all(output).unwrap();
        return;
    }
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "nested_build_does_not_inherit_host_rustflags_or_cfg",
            "--nocapture",
        ])
        .env("TOUCHBAR_BUILD_ENV_TEST_CHILD", "1")
        .env("RUSTFLAGS", "--this-is-an-invalid-host-only-rustflag")
        .env("CARGO_ENCODED_RUSTFLAGS", "--also-an-invalid-rustflag")
        .env("CARGO_CFG_TARGET_ARCH", "not-a-target")
        .env("CARGO_CFG_TARGET_OS", "not-an-os")
        .env("CARGO_FEATURE_HOST_ONLY", "1")
        .env("CARGO_MAKEFLAGS", "invalid-jobserver-options")
        .status()
        .unwrap();
    assert!(result.success());
}
