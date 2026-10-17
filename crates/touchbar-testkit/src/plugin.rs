use crate::*;
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
    let manifest_source = touchbar_loader::read_text(&path.join("plugin.toml"))?;
    let manifest = PluginManifest::from_toml(path.join("plugin.toml"), &manifest_source)
        .map_err(|e| e.to_string())?;
    let fallback = if manifest
        .logic
        .as_ref()
        .is_some_and(|file| !path.join(file).exists())
    {
        fixture(manifest.id.as_str())
            .map(|file| {
                touchbar_loader::read_bounded(Path::new(file), touchbar_loader::MAX_COMPONENT_BYTES)
            })
            .transpose()?
    } else {
        None
    };
    let files = touchbar_loader::read_plugin(path, fallback.as_deref())?;
    let specs = files
        .sets
        .iter()
        .map(|set| set.to_spec(&files.manifest).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let revision = files.revision;
    let component = files.component;
    let sets = files.sets;
    let sources = files.sources;
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
