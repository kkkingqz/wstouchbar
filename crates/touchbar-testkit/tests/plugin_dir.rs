use std::path::PathBuf;
use touchbar_testkit::*;
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "touchbar-testkit-plugin-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(p.join("sets")).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn fixture() -> Temp {
    let d = Temp::new();
    std::fs::write(d.0.join("plugin.toml"),"id='echo'\nversion='1.0.0'\napi='1'\nname='Echo'\ndescription='Fixture'\nlogic='logic.wasm'\n[[sets]]\nid='main'\nfile='sets/main.toml'").unwrap();
    std::fs::write(
        d.0.join("sets/main.toml"),
        "version=1\nid='main'\nlabel='Echo'\n[layout]\nkind='text'\ntext_from='last'",
    )
    .unwrap();
    d
}
#[test]
fn revisions_hash_manifest_sets_and_component_content() {
    let d = fixture();
    let first = plugin_dir(&d.0).unwrap();
    assert_eq!(first.catalog.sets.len(), 1);
    assert_eq!(first.revision, plugin_dir(&d.0).unwrap().revision);
    let path = d.0.join("sets/main.toml");
    let old = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, old.replace("Echo", "Changed")).unwrap();
    assert_ne!(first.revision, plugin_dir(&d.0).unwrap().revision);
    std::fs::write(&path, old).unwrap();
    assert_eq!(first.revision, plugin_dir(&d.0).unwrap().revision);
    std::fs::write(
        d.0.join("logic.wasm"),
        std::fs::read(touchbar_wasm_fixtures::PERSIST).unwrap(),
    )
    .unwrap();
    assert_ne!(first.revision, plugin_dir(&d.0).unwrap().revision);
}
#[test]
fn loader_checks_manifest_set_id_and_full_description_bindings() {
    let d = fixture();
    let loaded = plugin_dir(&d.0).unwrap();
    let mut host = WasmLogic::new();
    host.register_plugin(&loaded).unwrap();
    let path = d.0.join("sets/main.toml");
    let old = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        old.replace("text_from='last'", "color_from='missing'"),
    )
    .unwrap();
    let loaded = plugin_dir(&d.0).unwrap();
    let error = host.register_plugin(&loaded).unwrap_err();
    assert!(
        error.contains("sets/main.toml:6: layout.color_from"),
        "{error}"
    );
    std::fs::write(&path, old.replace("id='main'", "id='other'")).unwrap();
    assert!(plugin_dir(&d.0).unwrap_err().contains("set id"));
}
#[test]
fn declarative_system_plugin_loads_without_wasm() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/system");
    let p = plugin_dir(path).unwrap();
    assert!(p.component.is_none());
    assert!(p.catalog.sets[0].logic.is_none());
}
