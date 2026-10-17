use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use touchbar_core::{Core, CoreConfig};
use touchbar_loader::*;
use touchbar_logic_runtime::{MemoryStore, Runtime, RuntimeConfig};
static SERIAL: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "wstouchbar-loader-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn plugin(root: &Path, id: &str, label: &str) -> PathBuf {
    let p = root.join(id);
    fs::create_dir_all(p.join("sets")).unwrap();
    fs::write(p.join("plugin.toml"), format!("id='{id}'\nversion='1'\napi='1'\nname='{label}'\ndescription=''\n[[sets]]\nid='key'\nfile='sets/key.toml'\n")).unwrap();
    fs::write(
        p.join("sets/key.toml"),
        format!("version=1\nid='key'\nlabel='{label}'\n[layout]\nkind='key'\nkey='f1'\n"),
    )
    .unwrap();
    p
}
fn setup(t: &Temp) -> (LoadRequest, Runtime, Core, Loader) {
    let config = UserConfig::default();
    (
        LoadRequest {
            paths: vec![t.0.join("plugins")],
            config_file: t.0.join("config.toml"),
            bindings_file: t.0.join("bindings.toml"),
        },
        Runtime::new(RuntimeConfig::default(), Box::<MemoryStore>::default()).unwrap(),
        Core::new(CoreConfig::default()),
        Loader::new(config),
    )
}
fn load(loader: &mut Loader, req: &LoadRequest, runtime: &mut Runtime, core: &Core) -> LoadReport {
    let prepared = prepare(req.clone(), runtime.compiler());
    loader.accept(prepared, core, runtime)
}
#[test]
fn path_order_shadows_whole_plugin_and_revision_tracks_content() {
    let t = Temp::new();
    let (mut req, mut r, c, mut loader) = setup(&t);
    let first = plugin(&t.0.join("first"), "p", "first");
    plugin(&t.0.join("plugins"), "p", "second");
    req.paths.insert(0, t.0.join("first"));
    assert!(load(&mut loader, &req, &mut r, &c).accepted);
    let old = loader.current().unwrap().catalog.clone();
    assert_eq!(
        loader.current().unwrap().documents.sets[&"p/key".parse().unwrap()]
            .file
            .label,
        "first"
    );
    assert!(load(&mut loader, &req, &mut r, &c).accepted);
    assert_eq!(loader.current().unwrap().catalog, old);
    fs::write(
        first.join("sets/key.toml"),
        "version=1\nid='key'\nlabel='changed'\n[layout]\nkind='key'\nkey='f2'\n",
    )
    .unwrap();
    assert!(load(&mut loader, &req, &mut r, &c).accepted);
    assert_ne!(
        loader.current().unwrap().catalog.plugins[0].revision,
        old.plugins[0].revision
    );
}
#[test]
fn initial_bad_plugin_is_skipped_but_reload_is_atomic_for_all_documents_and_config() {
    let t = Temp::new();
    let (req, mut r, c, mut loader) = setup(&t);
    let good = plugin(&req.paths[0], "good", "old");
    let bad = plugin(&req.paths[0], "bad", "bad");
    fs::write(bad.join("sets/key.toml"), "broken").unwrap();
    let report = load(&mut loader, &req, &mut r, &c);
    assert!(report.accepted);
    assert!(!report.issues.is_empty());
    assert_eq!(loader.current().unwrap().catalog.plugins.len(), 1);
    let old = loader.current().unwrap().catalog.clone();
    let doc = loader.current().unwrap().documents.sets[&"good/key".parse().unwrap()].clone();
    fs::write(
        good.join("sets/key.toml"),
        "version=1\nid='key'\nlabel='new'\n[layout]\nkind='key'\nkey='f2'\n",
    )
    .unwrap();
    fs::write(&req.config_file, "desktop='null'\nlog='debug'\n").unwrap();
    assert!(!load(&mut loader, &req, &mut r, &c).accepted);
    assert_eq!(loader.current().unwrap().catalog, old);
    assert_eq!(loader.current().unwrap().config, UserConfig::default());
    assert!(std::sync::Arc::ptr_eq(
        &doc,
        &loader.current().unwrap().documents.sets[&"good/key".parse().unwrap()]
    ));
}
#[test]
fn bad_config_and_changed_hard_limit_reject_reload_and_allow_explicit_retry() {
    let t = Temp::new();
    let (req, mut r, c, mut loader) = setup(&t);
    plugin(&req.paths[0], "p", "ok");
    assert!(load(&mut loader, &req, &mut r, &c).accepted);
    for config in ["unknown=true", "[logic]\ntime-limit-ms=100"] {
        fs::write(&req.config_file, config).unwrap();
        assert!(!load(&mut loader, &req, &mut r, &c).accepted);
    }
    fs::write(&req.config_file, "desktop='null'\nlog='debug'").unwrap();
    assert!(load(&mut loader, &req, &mut r, &c).accepted);
}
#[test]
fn disabled_required_plugin_leaves_profile_slot_vacant() {
    let t = Temp::new();
    let (req, mut r, c, mut loader) = setup(&t);
    plugin(&req.paths[0], "p", "ok");
    let p = req.paths[0].join("profiles");
    fs::create_dir_all(&p).unwrap();
    fs::write(p.join("plugin.toml"),"id='profiles'\nversion='1'\napi='1'\nname='Profiles'\ndescription=''\nrequires=['p']\n[[profiles]]\nid='default'\nfile='default.toml'\n").unwrap();
    fs::write(p.join("default.toml"),"version=1\nid='default'\ndefault=true\n[[element]]\nkind='slot'\nid='key'\nitems=['p/key']").unwrap();
    fs::write(&req.config_file, "disabled=['p']").unwrap();
    assert!(load(&mut loader, &req, &mut r, &c).accepted);
    let cat = &loader.current().unwrap().catalog;
    assert!(cat.plugin(&"p".parse().unwrap()).is_none());
    assert!(!cat.set_available(&"p/key".parse().unwrap()));
    assert_eq!(cat.profiles.len(), 1);
}
#[test]
fn profile_conflict_is_diagnostic_and_explicit_choice_is_retained() {
    let t = Temp::new();
    let (req, mut r, c, mut loader) = setup(&t);
    for id in ["a", "b"] {
        let p = req.paths[0].join(id);
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("plugin.toml"),format!("id='{id}'\nversion='1'\napi='1'\nname='P'\ndescription=''\n[[profiles]]\nid='browser'\nfile='browser.toml'\n")).unwrap();
        fs::write(
            p.join("browser.toml"),
            "version=1\nid='browser'\nfor={category='browser'}",
        )
        .unwrap();
    }
    let report = load(&mut loader, &req, &mut r, &c);
    assert!(report.accepted);
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.message.contains("ProfileConflict"))
    );
    fs::write(&req.config_file,"[profile-for]\n'category:browser'='b/browser'\n[profile-for.'window:main']\n'category:browser'='a/browser'").unwrap();
    assert!(load(&mut loader, &req, &mut r, &c).accepted);
    assert_eq!(loader.current().unwrap().catalog.profile_for.len(), 2);
}
#[test]
fn prepared_modules_are_sent_from_worker_and_rejected_reload_retains_old_revision() {
    let t = Temp::new();
    let (req, mut r, c, mut loader) = setup(&t);
    let p = plugin(&req.paths[0], "echo", "ok");
    let manifest = fs::read_to_string(p.join("plugin.toml"))
        .unwrap()
        .replace("[[sets]]", "logic='logic.wasm'\n[[sets]]");
    fs::write(p.join("plugin.toml"), manifest).unwrap();
    fs::copy(touchbar_wasm_fixtures::ECHO, p.join("logic.wasm")).unwrap();
    let worker = Worker::new(r.compiler());
    worker.submit(req.clone()).unwrap();
    let prepared = worker.results.recv_blocking().unwrap();
    assert!(loader.accept(prepared, &c, &mut r).accepted);
    let rev = loader.current().unwrap().catalog.plugins[0]
        .revision
        .clone();
    fs::write(p.join("logic.wasm"), b"broken").unwrap();
    assert!(!load(&mut loader, &req, &mut r, &c).accepted);
    assert!(r.describe(&"echo".parse().unwrap(), &rev).is_ok());
}
#[test]
fn watcher_observes_new_nested_file_and_coalesces_changes() {
    let t = Temp::new();
    let (req, _, _, _) = setup(&t);
    let p = plugin(&req.paths[0], "p", "ok");
    let mut watcher = ReloadWatcher::new(&req).unwrap();
    fs::write(p.join("sets/key.toml"), "first").unwrap();
    assert!(!watcher.poll(Duration::ZERO).unwrap());
    assert!(!watcher.poll(Duration::from_millis(199)).unwrap());
    assert!(watcher.poll(Duration::from_millis(200)).unwrap());
    assert!(!watcher.poll(Duration::from_millis(201)).unwrap());
    fs::create_dir(p.join("new")).unwrap();
    assert!(!watcher.poll(Duration::from_millis(300)).unwrap());
    fs::write(p.join("new/asset.svg"), "x").unwrap();
    assert!(!watcher.poll(Duration::from_millis(400)).unwrap());
    assert!(!watcher.poll(Duration::from_millis(500)).unwrap());
    assert!(watcher.poll(Duration::from_millis(600)).unwrap());
}

#[test]
fn watcher_unreadable_descendant_keeps_catalog_and_recovers() {
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    if rustix::process::geteuid().is_root() {
        let cache = Temp::new();
        fs::set_permissions(&cache.0, fs::Permissions::from_mode(0o777)).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "watcher_unreadable_descendant_keeps_catalog_and_recovers",
            ])
            .env("XDG_CACHE_HOME", &cache.0)
            .uid(65534)
            .gid(65534)
            .status()
            .unwrap();
        assert!(
            status.success(),
            "unprivileged watcher test failed: {status}"
        );
        return;
    }
    let t = Temp::new();
    let (req, mut runtime, core, mut loader) = setup(&t);
    let p = plugin(&req.paths[0], "p", "ok");
    assert!(load(&mut loader, &req, &mut runtime, &core).accepted);
    let accepted = loader.current().unwrap().catalog.clone();
    let mut watcher = ReloadWatcher::new(&req).unwrap();
    let blocked = p.join("unused");
    fs::create_dir(&blocked).unwrap();
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o0)).unwrap();
    assert!(
        fs::read_dir(&blocked).is_err(),
        "test must run without DAC override"
    );
    assert!(!watcher.poll(Duration::ZERO).unwrap());
    assert!(!watcher.take_issues().is_empty());
    assert!(watcher.poll(Duration::from_millis(200)).unwrap());
    assert!(load(&mut loader, &req, &mut runtime, &core).accepted);
    assert_eq!(loader.current().unwrap().catalog, accepted);
    let startup = ReloadWatcher::new(&req);
    assert!(
        startup.is_ok(),
        "unreadable unused directory must not prevent startup"
    );
    fs::write(p.join("sets/key.toml"), "broken").unwrap();
    assert!(!watcher.poll(Duration::from_millis(300)).unwrap());
    assert!(watcher.poll(Duration::from_millis(500)).unwrap());
    assert!(!load(&mut loader, &req, &mut runtime, &core).accepted);
    assert_eq!(loader.current().unwrap().catalog, accepted);
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!watcher.poll(Duration::from_millis(600)).unwrap());
    assert!(watcher.poll(Duration::from_millis(800)).unwrap());
    fs::write(blocked.join("new.txt"), "new").unwrap();
    assert!(!watcher.poll(Duration::from_millis(900)).unwrap());
    assert!(watcher.poll(Duration::from_millis(1100)).unwrap());
}

#[test]
fn watcher_directory_budget_does_not_abort_startup() {
    let t = Temp::new();
    let (req, _, _, _) = setup(&t);
    let p = plugin(&req.paths[0], "p", "ok");
    for index in 0..4096 {
        fs::create_dir(p.join(format!("unused{index}"))).unwrap();
    }
    let mut watcher =
        ReloadWatcher::new(&req).expect("watch budget must not terminate the session");
    assert!(
        watcher
            .take_issues()
            .iter()
            .any(|issue| issue.contains("4096"))
    );
    assert_eq!(watcher.deadline(), Some(Duration::from_secs(1)));
    assert!(!watcher.poll(Duration::from_secs(1)).unwrap());
    assert_eq!(watcher.deadline(), Some(Duration::from_secs(2)));
}
#[test]
fn declared_assets_are_loaded_and_hashed_and_png_sizes_are_verified() {
    let t = Temp::new();
    let p = plugin(&t.0, "images", "images");
    let base = fs::read_to_string(p.join("plugin.toml")).unwrap();
    let asset =
        "\n[[assets]]\nid='icon'\nfile='icon.svg'\nkind='symbolic-svg'\nwidth=8\nheight=8\n";
    fs::write(p.join("plugin.toml"), format!("{base}{asset}")).unwrap();
    fs::write(p.join("icon.svg"),"<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'><rect width='8' height='8' fill='white'/></svg>").unwrap();
    let first = read_plugin(&p, None).unwrap();
    assert_eq!(first.assets["icon"].pixels.len(), 256);
    fs::write(p.join("icon.svg"),"<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'><circle cx='4' cy='4' r='3' fill='white'/></svg>").unwrap();
    assert_ne!(read_plugin(&p, None).unwrap().revision, first.revision);
    let mut bytes = vec![];
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[255, 0, 0, 255, 0, 255, 0, 128])
            .unwrap();
    }
    fs::write(p.join("icon.png"), bytes).unwrap();
    fs::write(
        p.join("plugin.toml"),
        format!(
            "{base}{}",
            asset
                .replace("icon.svg", "icon.png")
                .replace("symbolic-svg", "png")
                .replace("width=8", "width=2")
                .replace("height=8", "height=1")
        ),
    )
    .unwrap();
    assert_eq!(
        &*read_plugin(&p, None).unwrap().assets["icon"].pixels,
        &[255, 0, 0, 255, 0, 255, 0, 128]
    );
    fs::write(
        p.join("plugin.toml"),
        format!(
            "{base}{}",
            asset
                .replace("icon.svg", "icon.png")
                .replace("symbolic-svg", "png")
        ),
    )
    .unwrap();
    assert!(read_plugin(&p, None).unwrap_err().contains("dimensions"));
}
#[test]
fn config_is_strict_and_preserves_app_rules_for_later_stage() {
    let source = "desktop='null'\nlog='debug,touchbar_core=trace'\n[app.'org.example.App']\ncategories=['browser']\n[logic]\ntime-limit-ms=80";
    let config = UserConfig::from_toml("config.toml", source).unwrap();
    assert_eq!(config.app["org.example.App"].categories, ["browser"]);
    assert_eq!(config.logic.time_limit_ms, 80);
    for bad in [
        "typo=true",
        "[logic]\ntime-limit-ms=0",
        "desktop='bad'",
        "[app.foo]\nwrong=[]",
        "[profile-for]\n'unknown:foo'='p/set'",
        "log='foo=banana'",
    ] {
        assert!(UserConfig::from_toml("config.toml", bad).is_err(), "{bad}");
    }
}
#[test]
fn bad_selected_profile_on_first_load_keeps_default_config_and_healthy_plugins() {
    let t = Temp::new();
    let (req, mut r, c, mut loader) = setup(&t);
    plugin(&req.paths[0], "p", "ok");
    fs::write(
        &req.config_file,
        "[profile-for]\n'category:browser'='missing/profile'",
    )
    .unwrap();
    let report = load(&mut loader, &req, &mut r, &c);
    assert!(report.accepted);
    assert!(!report.issues.is_empty());
    assert_eq!(loader.current().unwrap().catalog.plugins.len(), 1);
    assert_eq!(loader.current().unwrap().config, UserConfig::default());
}
#[test]
fn invalid_bindings_plugin_does_not_drop_an_unrelated_valid_bindings_plugin() {
    let t = Temp::new();
    let (req, mut r, c, mut loader) = setup(&t);
    for (id, key) in [("a", "fn"), ("b", "fn"), ("c", "esc")] {
        let p = plugin(&req.paths[0], id, "ok");
        fs::write(p.join("bindings.toml"),format!("[[binding]]\nid='{id}'\ntrigger={{key='{key}',gesture='while-held'}}\naction={{layer='{id}/key'}}")).unwrap();
    }
    let report = load(&mut loader, &req, &mut r, &c);
    assert!(report.accepted);
    let ids: Vec<_> = loader
        .current()
        .unwrap()
        .catalog
        .plugins
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    assert_eq!(ids, ["a", "c"]);
}
#[test]
fn watcher_follows_plugin_symlink_and_directory_replacement() {
    let t = Temp::new();
    let (req, _, _, _) = setup(&t);
    fs::create_dir_all(&req.paths[0]).unwrap();
    let external = Temp::new();
    let target = plugin(&external.0, "p", "old");
    std::os::unix::fs::symlink(&target, req.paths[0].join("p")).unwrap();
    let mut watcher = ReloadWatcher::new(&req).unwrap();
    fs::write(target.join("sets/key.toml"), "old edit").unwrap();
    assert!(!watcher.poll(Duration::ZERO).unwrap());
    assert!(watcher.poll(Duration::from_millis(200)).unwrap());
    fs::remove_file(req.paths[0].join("p")).unwrap();
    let new = plugin(&req.paths[0], "p", "new");
    assert!(!watcher.poll(Duration::from_millis(300)).unwrap());
    assert!(watcher.poll(Duration::from_millis(500)).unwrap());
    fs::write(new.join("sets/key.toml"), "new edit").unwrap();
    assert!(!watcher.poll(Duration::from_millis(600)).unwrap());
    assert!(watcher.poll(Duration::from_millis(800)).unwrap());
}
#[test]
fn watcher_rebinds_a_directory_replaced_at_the_same_canonical_path() {
    let t = Temp::new();
    let (req, _, _, _) = setup(&t);
    let old = plugin(&req.paths[0], "p", "old");
    let mut watcher = ReloadWatcher::new(&req).unwrap();
    fs::rename(&old, t.0.join("retired")).unwrap();
    let new = plugin(&req.paths[0], "p", "new");
    assert!(!watcher.poll(Duration::ZERO).unwrap());
    assert!(watcher.poll(Duration::from_millis(200)).unwrap());
    fs::write(new.join("sets/key.toml"), "edited replacement").unwrap();
    assert!(!watcher.poll(Duration::from_millis(300)).unwrap());
    assert!(watcher.poll(Duration::from_millis(500)).unwrap());
}
#[test]
fn worker_submission_never_blocks_and_queued_work_is_bounded() {
    let t = Temp::new();
    let (req, r, _, _) = setup(&t);
    let worker = Worker::new(r.compiler());
    assert!((0..10_000).any(|_| worker.submit(req.clone()).is_err()));
}
#[test]
fn bounded_reader_rejects_fifo_without_waiting_for_a_writer() {
    let t = Temp::new();
    let path = t.0.join("fifo");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &path,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || tx.send(read_bounded(&path, 100)).unwrap());
    assert!(
        rx.recv_timeout(Duration::from_millis(300))
            .expect("bounded file read must not wait for FIFO peers")
            .is_err()
    );
}

#[test]
fn broken_config_at_start_keeps_startup_config_and_healthy_plugins() {
    let t = Temp::new();
    let (req, mut r, c, mut loader) = setup(&t);
    plugin(&req.paths[0], "p", "ok");
    fs::write(&req.config_file, "this is not toml").unwrap();
    let report = load(&mut loader, &req, &mut r, &c);
    assert!(report.accepted);
    assert!(!report.issues.is_empty());
    assert_eq!(loader.current().unwrap().config, UserConfig::default());
    assert_eq!(loader.current().unwrap().catalog.plugins.len(), 1);
}

#[test]
fn watcher_ignores_unrelated_ancestor_writes_and_follows_missing_paths() {
    let t = Temp::new();
    let req = LoadRequest {
        paths: vec![t.0.join("share/touchbar/plugins")],
        config_file: t.0.join("config/touchbar/config.toml"),
        bindings_file: t.0.join("config/touchbar/bindings.toml"),
    };
    fs::create_dir_all(t.0.join("config")).unwrap();
    fs::create_dir_all(t.0.join("share")).unwrap();
    let mut watcher = ReloadWatcher::new(&req).unwrap();
    fs::write(t.0.join("config/recently-used.xbel"), "unrelated").unwrap();
    fs::write(t.0.join("share/other-package"), "unrelated").unwrap();
    assert!(!watcher.poll(Duration::ZERO).unwrap());
    assert!(!watcher.poll(Duration::from_millis(250)).unwrap());
    assert_eq!(watcher.deadline(), None);
    fs::create_dir_all(req.paths[0].clone()).unwrap();
    fs::create_dir_all(req.config_file.parent().unwrap()).unwrap();
    assert!(!watcher.poll(Duration::from_millis(300)).unwrap());
    assert!(watcher.poll(Duration::from_millis(550)).unwrap());
    fs::write(&req.config_file, "config").unwrap();
    assert!(!watcher.poll(Duration::from_millis(600)).unwrap());
    assert!(watcher.poll(Duration::from_millis(850)).unwrap());
    fs::write(req.paths[0].join("plugin.toml"), "plugin").unwrap();
    assert!(!watcher.poll(Duration::from_millis(900)).unwrap());
    assert!(watcher.poll(Duration::from_millis(1150)).unwrap());
    fs::write(t.0.join("config/other"), "still unrelated").unwrap();
    assert!(!watcher.poll(Duration::from_millis(1200)).unwrap());
    assert!(!watcher.poll(Duration::from_millis(1450)).unwrap());
}
