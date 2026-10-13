use crate::*;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use touchbar_model::*;
use touchbar_sets::{PluginManifest, SetFile};
#[derive(Debug)]
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub sets: Vec<SetFile>,
    pub catalog: Catalog,
    pub revision: String,
    pub component: Option<Vec<u8>>,
    pub(crate) sources: Vec<(PathBuf, String)>,
}
fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}
fn hash_file(hash: &mut Sha256, name: &str, bytes: &[u8]) {
    hash.update((name.len() as u64).to_le_bytes());
    hash.update(name.as_bytes());
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}
fn fixture(id: &str) -> Option<&'static str> {
    use touchbar_wasm_fixtures::*;
    Some(match id {
        "controls" => CONTROLS,
        "echo" => ECHO,
        "trap" => TRAP,
        "hang" => HANG,
        "oom" => OOM,
        "err" => ERR,
        "budget" => BUDGET,
        "log" => LOG,
        "persist" => PERSIST,
        "version" => VERSION,
        "wasi" => WASI,
        _ => return None,
    })
}
/// Load TOML and the component bytes used by tests. Profiles are supplied by the scenario.
pub fn plugin_dir(path: impl AsRef<Path>) -> Result<LoadedPlugin, String> {
    let path = path.as_ref();
    let source = read(&path.join("plugin.toml"))?;
    let manifest =
        PluginManifest::from_toml(path.join("plugin.toml"), &source).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    hash_file(&mut hash, "plugin.toml", source.as_bytes());
    let component = if let Some(logic) = &manifest.logic {
        let file = path.join(logic);
        let bytes = match std::fs::read(&file) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let source = fixture(manifest.id.as_str()).ok_or_else(|| {
                    format!("{}: no fixture for plugin {}", file.display(), manifest.id)
                })?;
                std::fs::read(source).map_err(|e| format!("{source}: {e}"))?
            }
            Err(e) => return Err(format!("{}: {e}", file.display())),
        };
        hash_file(&mut hash, logic, &bytes);
        Some(bytes)
    } else {
        None
    };
    let mut sets = vec![];
    let mut sources = vec![];
    let mut specs = vec![];
    for entry in &manifest.sets {
        let file = path.join(&entry.file);
        let source = read(&file)?;
        hash_file(&mut hash, &entry.file, source.as_bytes());
        let set = SetFile::from_toml(&file, &source).map_err(|e| e.to_string())?;
        if set.id != entry.id {
            return Err(format!(
                "{}: set id {} does not match manifest {}",
                file.display(),
                set.id,
                entry.id
            ));
        }
        specs.push(set.to_spec(&manifest).map_err(|e| e.to_string())?);
        sets.push(set);
        sources.push((file, source));
    }
    let revision = format!("{:x}", hash.finalize());
    let catalog = Catalog {
        plugins: vec![PluginSpec {
            id: manifest.id.clone(),
            revision: revision.clone(),
            enabled: true,
            has_logic: manifest.logic.is_some(),
            requires: manifest.requires.clone(),
        }],
        sets: specs,
        ..Default::default()
    };
    let errors = catalog.validate();
    if !errors.is_empty() {
        return Err(format!(
            "{}: invalid plugin catalog: {errors:?}",
            path.display()
        ));
    }
    Ok(LoadedPlugin {
        manifest,
        sets,
        catalog,
        revision,
        component,
        sources,
    })
}
impl WasmLogic {
    pub fn register_plugin(&mut self, plugin: &LoadedPlugin) -> Result<(), String> {
        if let Some(bytes) = &plugin.component {
            self.register_bytes(plugin.manifest.id.as_str(), &plugin.revision, bytes)?;
            let description = self.probe(&plugin.manifest.id, &plugin.revision)?;
            for (file, source) in &plugin.sources {
                SetFile::from_toml_with_description(file, source, &description)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}
