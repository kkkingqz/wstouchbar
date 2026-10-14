//! Bounded plugin I/O and atomic admission. Compilation runs in the worker; guest probes in the caller.
mod config;
mod files;
mod watch;
pub use config::*;
pub use files::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};
use touchbar_bindings::config::{BindingFile, LayerActionFile, merge_bindings};
use touchbar_core::Core;
use touchbar_logic_runtime::{Compiler, LoadError, PreparedModule, Runtime};
use touchbar_model::*;
use touchbar_profiles::resolve_profiles;
use touchbar_scene::{Documents, SetDocument};
use touchbar_sets::SetFile;
pub use watch::*;
#[derive(Clone, Debug)]
pub struct LoadRequest {
    pub paths: Vec<PathBuf>,
    pub config_file: PathBuf,
    pub bindings_file: PathBuf,
}
impl LoadRequest {
    pub fn standard(extra: Option<PathBuf>) -> Result<Self, String> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .ok_or("HOME must be absolute")?;
        let xdg = |name: &str, fallback: PathBuf| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or(fallback)
        };
        let config = xdg("XDG_CONFIG_HOME", home.join(".config")).join("touchbar");
        let mut paths = Vec::from_iter(extra);
        paths.extend([
            xdg("XDG_DATA_HOME", home.join(".local/share")).join("touchbar/plugins"),
            "/usr/local/share/touchbar/plugins".into(),
            "/usr/share/touchbar/plugins".into(),
        ]);
        Ok(Self {
            paths,
            config_file: config.join("config.toml"),
            bindings_file: config.join("bindings.toml"),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LoadIssue {
    pub plugin: Option<PluginId>,
    pub message: String,
}
fn issue(plugin: Option<PluginId>, message: impl Into<String>) -> LoadIssue {
    LoadIssue {
        plugin,
        message: message.into(),
    }
}
struct PreparedPlugin {
    files: PluginFiles,
    module: Option<Result<PreparedModule, LoadError>>,
}
pub struct PreparedLoad {
    config: Result<UserConfig, String>,
    plugins: Vec<PreparedPlugin>,
    bindings: Option<BindingFile<LayerActionFile>>,
    issues: Vec<LoadIssue>,
}
/// Blocking worker entry point. Safe to send both compiler and result between threads.
pub fn prepare(request: LoadRequest, compiler: Compiler) -> PreparedLoad {
    let config = read_config(&request.config_file);
    let mut issues = vec![];
    let bindings = match optional_bindings(&request.bindings_file) {
        Ok(b) => b,
        Err(e) => {
            issues.push(issue(None, e));
            None
        }
    };
    let mut seen = BTreeSet::new();
    let mut plugins = vec![];
    for path in &request.paths {
        let directories = match discover(path) {
            Ok(d) => d,
            Err(e) => {
                issues.push(issue(None, e));
                continue;
            }
        };
        for directory in directories {
            let manifest_source = match read_text(&directory.join("plugin.toml")) {
                Ok(s) => s,
                Err(e) => {
                    issues.push(issue(None, e));
                    continue;
                }
            };
            // Reserve a parseable identity even if another manifest field is invalid: no fallback to a shadowed plugin.
            #[derive(serde::Deserialize)]
            struct Identity {
                id: PluginId,
            }
            let identity = toml::from_str::<Identity>(&manifest_source)
                .ok()
                .map(|m| m.id);
            if let Some(id) = &identity {
                if !seen.insert(id.clone()) {
                    continue;
                }
                if config.as_ref().is_ok_and(|c| c.disabled.contains(id)) {
                    continue;
                }
            }
            let files = match read_plugin(&directory, None) {
                Ok(f) => f,
                Err(e) => {
                    issues.push(issue(identity, e));
                    continue;
                }
            };
            let module = files
                .component
                .as_ref()
                .map(|bytes| compiler.prepare(bytes));
            plugins.push(PreparedPlugin { files, module });
        }
    }
    PreparedLoad {
        config,
        plugins,
        bindings,
        issues,
    }
}
pub fn discover(root: &std::path::Path) -> Result<Vec<PathBuf>, String> {
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(format!("{}: {e}", root.display())),
    };
    let mut out = vec![];
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_dir()
            && path
                .join("plugin.toml")
                .try_exists()
                .map_err(|e| e.to_string())?
        {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}
fn optional_bindings(
    path: &std::path::Path,
) -> Result<Option<BindingFile<LayerActionFile>>, String> {
    if !path.try_exists().map_err(|e| e.to_string())? {
        return Ok(None);
    }
    let source = read_text(path)?;
    BindingFile::from_toml(path, &source)
        .map(Some)
        .map_err(|e| e.to_string())
}
pub struct Worker {
    sender: Option<std::sync::mpsc::SyncSender<LoadRequest>>,
    pub results: async_channel::Receiver<PreparedLoad>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Worker {
    pub fn new(compiler: Compiler) -> Self {
        let (sender, requests) = std::sync::mpsc::sync_channel(1);
        let (results, receiver) = async_channel::bounded(1);
        let thread = std::thread::spawn(move || {
            while let Ok(request) = requests.recv() {
                if results
                    .send_blocking(prepare(request, compiler.clone()))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            sender: Some(sender),
            results: receiver,
            thread: Some(thread),
        }
    }
    pub fn submit(&self, request: LoadRequest) -> Result<(), String> {
        self.sender
            .as_ref()
            .ok_or("loader worker closed")?
            .try_send(request)
            .map_err(|e| e.to_string())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.sender.take();
        self.results.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
#[derive(Clone, Debug)]
pub struct Accepted {
    pub catalog: Catalog,
    pub documents: Documents,
    pub config: UserConfig,
    pub issues: Vec<LoadIssue>,
}
#[derive(Clone, Debug)]
pub struct LoadReport {
    pub accepted: bool,
    pub issues: Vec<LoadIssue>,
}
pub struct Loader {
    startup: UserConfig,
    current: Option<Accepted>,
}
impl Loader {
    pub fn new(startup: UserConfig) -> Self {
        Self {
            startup,
            current: None,
        }
    }
    pub fn current(&self) -> Option<&Accepted> {
        self.current.as_ref()
    }
    /// Caller publishes the returned accepted Catalog and Documents together in one loop iteration.
    pub fn accept(
        &mut self,
        prepared: PreparedLoad,
        core: &Core,
        runtime: &mut Runtime,
    ) -> LoadReport {
        let initial = self.current.is_none();
        let mut issues = prepared.issues;
        let mut config = match prepared.config {
            Ok(c) => c,
            Err(e) => {
                issues.push(issue(None, e));
                self.current
                    .as_ref()
                    .map_or_else(|| self.startup.clone(), |a| a.config.clone())
            }
        };
        if config.logic != self.startup.logic {
            issues.push(issue(None, "logic.time-limit-ms changed; restart required"));
            return self.reject(runtime, issues);
        }
        if !initial && !issues.is_empty() {
            return self.reject(runtime, issues);
        }
        let mut good = vec![];
        for prepared in prepared.plugins {
            let files = prepared.files;
            let id = files.manifest.id.clone();
            let checked = (|| {
                if let Some(module) = prepared.module {
                    runtime
                        .install(id.clone(), &files.revision, module)
                        .map_err(|e| e.to_string())?;
                    let description = runtime
                        .describe(&id, &files.revision)
                        .map_err(|e| e.to_string())?;
                    for (path, source) in &files.sources {
                        SetFile::from_toml_with_description(path, source, &description)
                            .map_err(|e| e.to_string())?;
                    }
                }
                Ok::<_, String>(())
            })();
            if let Err(e) = checked {
                issues.push(issue(Some(id), e));
            } else {
                good.push(files);
            }
        }
        if !initial && !issues.is_empty() {
            return self.reject(runtime, issues);
        }
        let mut user_bindings = prepared.bindings;
        let mut retried_user = false;
        // On startup remove an invalid owner as a unit and recheck remaining dependencies.
        let accepted = loop {
            match assemble(&good, &config, user_bindings.as_ref(), core) {
                Ok((catalog, documents, diagnostics)) => {
                    issues.extend(diagnostics);
                    break Some(Accepted {
                        catalog,
                        documents,
                        config: config.clone(),
                        issues: issues.clone(),
                    });
                }
                Err(failure) => {
                    if !initial {
                        issues.extend(failure);
                        break None;
                    }
                    let owners: BTreeSet<_> =
                        failure.iter().filter_map(|i| i.plugin.clone()).collect();
                    issues.extend(failure);
                    let before = good.len();
                    good.retain(|p| !owners.contains(&p.manifest.id));
                    if good.len() == before {
                        if !retried_user && config.disabled == self.startup.disabled {
                            config = self.startup.clone();
                            user_bindings = None;
                            retried_user = true;
                            continue;
                        }
                        break None;
                    }
                }
            }
        };
        let Some(accepted) = accepted else {
            return self.reject(runtime, issues);
        };
        self.current = Some(accepted);
        self.retain(runtime);
        LoadReport {
            accepted: true,
            issues,
        }
    }
    fn reject(&self, runtime: &mut Runtime, issues: Vec<LoadIssue>) -> LoadReport {
        self.retain(runtime);
        LoadReport {
            accepted: false,
            issues,
        }
    }
    fn retain(&self, runtime: &mut Runtime) {
        runtime.retain_modules(|id, rev| {
            self.current.as_ref().is_some_and(|a| {
                a.catalog
                    .plugins
                    .iter()
                    .any(|p| &p.id == id && p.revision == rev)
            })
        });
    }
}
fn assemble(
    plugins: &[PluginFiles],
    config: &UserConfig,
    user: Option<&BindingFile<LayerActionFile>>,
    core: &Core,
) -> Result<(Catalog, Documents, Vec<LoadIssue>), Vec<LoadIssue>> {
    let mut catalog = Catalog::default();
    let mut documents = Documents::default();
    let mut profiles = BTreeMap::new();
    let mut bindings = vec![];
    for plugin in plugins {
        let manifest = &plugin.manifest;
        let owner = Some(manifest.id.clone());
        catalog.plugins.push(PluginSpec {
            id: manifest.id.clone(),
            revision: plugin.revision.clone(),
            enabled: true,
            has_logic: manifest.logic.is_some(),
            requires: manifest.requires.clone(),
        });
        for file in &plugin.sets {
            let set = file
                .to_spec(manifest)
                .map_err(|e| vec![issue(owner.clone(), e.to_string())])?;
            documents.sets.insert(
                set.reference.clone(),
                Arc::new(SetDocument {
                    file: file.clone(),
                    assets: plugin.assets.clone(),
                    asset_kinds: manifest
                        .assets
                        .iter()
                        .map(|a| (a.id.clone(), a.kind))
                        .collect(),
                }),
            );
            catalog.sets.push(set);
        }
        profiles.extend(plugin.profiles.clone());
        if let Some(b) = &plugin.bindings {
            bindings.push(b.clone());
            merge_bindings(&bindings, None)
                .map_err(|e| vec![issue(owner.clone(), e.to_string())])?;
        }
    }
    documents.profiles = resolve_profiles(&profiles).map_err(|e| {
        let owner = plugins
            .iter()
            .find(|p| e.file.starts_with(&p.root))
            .map(|p| p.manifest.id.clone());
        vec![issue(owner, e.to_string())]
    })?;
    catalog.profiles = documents
        .profiles
        .values()
        .map(|p| p.spec.clone())
        .collect();
    catalog.bindings =
        merge_bindings(&bindings, user).map_err(|e| vec![issue(None, e.to_string())])?;
    catalog.profile_for = config
        .overrides()
        .map_err(|e| vec![issue(None, e.to_string())])?;
    core.check_catalog(&catalog).map_err(|errors| {
        errors
            .into_iter()
            .map(|e| {
                let owner = plugins
                    .iter()
                    .find(|p| {
                        e.location.starts_with(&format!("{}/", p.manifest.id))
                            || p.bindings
                                .as_ref()
                                .is_some_and(|b| b.bindings.iter().any(|b| b.id == e.location))
                    })
                    .map(|p| p.manifest.id.clone());
                issue(owner, format!("{:?}: {}: {}", e.code, e.location, e.detail))
            })
            .collect::<Vec<_>>()
    })?;
    let mut diagnostics: Vec<_> = catalog
        .validate()
        .into_iter()
        .filter(|e| e.code == CatalogIssueCode::ProfileConflict)
        .map(|e| {
            issue(
                None,
                format!("ProfileConflict: {}: {}", e.location, e.detail),
            )
        })
        .collect();
    for plugin in plugins {
        for required in &plugin.manifest.requires {
            if catalog.plugin(required).is_none() {
                diagnostics.push(issue(Some(plugin.manifest.id.clone()),format!("required plugin {required} is absent or disabled; dependent slots remain vacant")));
            }
        }
    }
    Ok((catalog, documents, diagnostics))
}

/// Resolve one explicitly requested set for offline inspection, without loading guest logic.
pub fn offline_set(paths: &[PathBuf], reference: &SetRef) -> Result<(SetSpec, Documents), String> {
    for root in paths {
        for path in discover(root)? {
            #[derive(serde::Deserialize)]
            struct Identity {
                id: PluginId,
            }
            let source = read_text(&path.join("plugin.toml"))?;
            if toml::from_str::<Identity>(&source).is_ok_and(|p| p.id == reference.plugin) {
                let plugin = read_plugin_documents(&path)?;
                let file = plugin
                    .sets
                    .iter()
                    .find(|s| s.id == reference.id())
                    .ok_or_else(|| format!("set {reference} is absent from first matching plugin"))?
                    .clone();
                let spec = file.to_spec(&plugin.manifest).map_err(|e| e.to_string())?;
                let mut documents = Documents::default();
                documents.sets.insert(
                    reference.clone(),
                    Arc::new(SetDocument {
                        file,
                        assets: plugin.assets,
                        asset_kinds: plugin
                            .manifest
                            .assets
                            .iter()
                            .map(|a| (a.id.clone(), a.kind))
                            .collect(),
                    }),
                );
                return Ok((spec, documents));
            }
        }
    }
    Err(format!("plugin {} was not found", reference.plugin))
}
