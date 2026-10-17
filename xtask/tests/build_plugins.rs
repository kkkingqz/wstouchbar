use std::{
    fs,
    path::{Path, PathBuf},
};
use touchbar_model::*;
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}
#[test]
fn assembled_tree_loads_all_plugins_and_fn_shows_fkeys_in_default_profile() {
    use touchbar_core::{Core, CoreConfig, Input, KeySource};
    use touchbar_loader::*;
    use touchbar_logic_runtime::{MemoryStore, Runtime, RuntimeConfig};
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("plugins");
    xtask::assemble_plugins(
        &root(),
        &output,
        Path::new(touchbar_wasm_fixtures::CONTROLS),
    )
    .unwrap();
    assert!(output.join("controls/logic.wasm").is_file());
    assert!(output.join("system/bindings.toml").is_file());
    let mut runtime =
        Runtime::new(RuntimeConfig::default(), Box::<MemoryStore>::default()).unwrap();
    let mut core = Core::new(CoreConfig::default());
    let mut loader = Loader::new(UserConfig::default());
    let request = LoadRequest {
        paths: vec![output],
        config_file: dir.path().join("config.toml"),
        bindings_file: dir.path().join("bindings.toml"),
    };
    let report = loader.accept(prepare(request, runtime.compiler()), &core, &mut runtime);
    assert!(report.accepted && report.issues.is_empty(), "{report:?}");
    let catalog = loader.current().unwrap().catalog.clone();
    assert_eq!(catalog.plugins.len(), 3);
    core.handle(Timestamp(0), Input::Catalog(catalog));
    core.handle(
        Timestamp(0),
        Input::SurfaceAdded(SurfaceInfo {
            id: SurfaceId(1),
            name: "touchbar".parse().unwrap(),
            kind: SurfaceKind::Touchbar,
            width_px: 2008,
            height_px: 60,
            px_per_mm: 6.2,
            caps: SurfaceCaps {
                watch_keys: true,
                inject_keys: true,
                ..SurfaceCaps::default()
            },
        }),
    );
    assert_eq!(
        core.snapshot().surfaces[0].scene.profile,
        Some("profile-default/default".parse().unwrap())
    );
    core.handle(
        Timestamp(1),
        Input::Key {
            source: KeySource::Surface(SurfaceId(1)),
            code: WatchKey::Fn,
            phase: KeyPhase::Pressed,
        },
    );
    assert_eq!(
        core.snapshot().surfaces[0].layers[0].set,
        "system/fkeys".parse().unwrap()
    );
    core.handle(
        Timestamp(2),
        Input::Key {
            source: KeySource::Surface(SurfaceId(1)),
            code: WatchKey::Fn,
            phase: KeyPhase::Released,
        },
    );
    assert!(core.snapshot().surfaces[0].layers.is_empty());
}
#[test]
fn rebuild_removes_stale_plugins_but_refuses_source_and_unowned_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("plugins");
    let wasm = Path::new(touchbar_wasm_fixtures::CONTROLS);
    xtask::assemble_plugins(&root(), &output, wasm).unwrap();
    fs::create_dir(output.join("stale")).unwrap();
    fs::write(output.join("stale/plugin.toml"), "old").unwrap();
    xtask::assemble_plugins(&root(), &output, wasm).unwrap();
    assert!(!output.join("stale").exists());
    let foreign = dir.path().join("foreign");
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join("mine"), "keep").unwrap();
    assert!(xtask::assemble_plugins(&root(), &foreign, wasm).is_err());
    assert_eq!(fs::read(foreign.join("mine")).unwrap(), b"keep");
    for output in [
        root(),
        root().join("plugins"),
        root().join("plugins/nested"),
    ] {
        assert!(xtask::assemble_plugins(&root(), &output, wasm).is_err());
    }
}
