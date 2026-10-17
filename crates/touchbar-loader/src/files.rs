use crate::*;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
use touchbar_bindings::config::{BindingFile, LayerActionFile};
use touchbar_model::*;
use touchbar_profiles::ProfileDocument;
use touchbar_sets::{AssetKind, PluginManifest, SetFile};
use touchbar_ui::{Image, SvgAsset, SvgRasterizer};
pub const MAX_TEXT_BYTES: usize = 256 * 1024;
pub const MAX_COMPONENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_PLUGIN_BYTES: usize = 64 * 1024 * 1024;
/// Read a finite regular file, including a second size check after the read.
pub fn read_bounded(path: &Path, max: usize) -> Result<Vec<u8>, String> {
    let descriptor = rustix::fs::openat(
        rustix::fs::CWD,
        path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    )
    .map_err(|e| format!("{}: {e}", path.display()))?;
    let f = File::from(descriptor);
    let meta = f.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > max as u64 {
        return Err(format!(
            "{}: not a regular file within {max} bytes",
            path.display()
        ));
    }
    let mut out = Vec::new();
    f.take(max as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if out.len() > max {
        return Err(format!("{}: file exceeds {max} bytes", path.display()));
    }
    Ok(out)
}
pub fn read_text(path: &Path) -> Result<String, String> {
    String::from_utf8(read_bounded(path, MAX_TEXT_BYTES)?)
        .map_err(|e| format!("{}: {e}", path.display()))
}
pub fn read_config(path: &Path) -> Result<UserConfig, String> {
    match read_text(path) {
        Ok(s) => UserConfig::from_toml(path, &s).map_err(|e| e.to_string()),
        Err(_) if !path.try_exists().map_err(|e| e.to_string())? => Ok(UserConfig::default()),
        Err(e) => Err(e),
    }
}
/// Stable identity over sorted relative paths and exact file bytes. Includes length delimiters.
pub fn content_revision(files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut hash = Sha256::new();
    for (name, bytes) in files {
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name);
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    format!("{:x}", hash.finalize())
}
#[derive(Clone, Debug)]
pub struct PluginFiles {
    pub manifest: PluginManifest,
    pub sets: Vec<SetFile>,
    pub profiles: BTreeMap<ProfileRef, ProfileDocument>,
    pub bindings: Option<BindingFile<LayerActionFile>>,
    pub assets: BTreeMap<String, Image>,
    pub sources: Vec<(PathBuf, String)>,
    pub component: Option<Vec<u8>>,
    pub revision: String,
    pub root: PathBuf,
}
struct Reader {
    root: PathBuf,
    files: BTreeMap<String, Vec<u8>>,
    total: usize,
}
impl Reader {
    fn read(&mut self, relative: &str, max: usize) -> Result<Vec<u8>, String> {
        let file = self.root.join(relative);
        let canonical = file
            .canonicalize()
            .map_err(|e| format!("{}: {e}", file.display()))?;
        if !canonical.starts_with(&self.root) {
            return Err(format!("{}: file escapes plugin directory", file.display()));
        }
        let bytes = read_bounded(&canonical, max)?;
        self.total = self
            .total
            .checked_add(bytes.len())
            .ok_or("plugin size overflow")?;
        if self.total > MAX_PLUGIN_BYTES || self.files.len() >= 1024 {
            return Err("plugin file budget exceeded".into());
        }
        self.files.insert(relative.into(), bytes.clone());
        Ok(bytes)
    }
    fn text(&mut self, relative: &str) -> Result<String, String> {
        String::from_utf8(self.read(relative, MAX_TEXT_BYTES)?).map_err(|e| e.to_string())
    }
}
/// Shared reader. Tests can supply a missing component fixture; production requires a real file.
pub fn read_plugin(path: impl AsRef<Path>, fallback: Option<&[u8]>) -> Result<PluginFiles, String> {
    read_plugin_impl(path.as_ref(), fallback, false)
}
/// Layout-only reader used by offline render; no wasm is read or compiled.
pub fn read_plugin_documents(path: impl AsRef<Path>) -> Result<PluginFiles, String> {
    read_plugin_impl(path.as_ref(), None, true)
}
fn read_plugin_impl(
    path: &Path,
    fallback: Option<&[u8]>,
    documents_only: bool,
) -> Result<PluginFiles, String> {
    let root = path
        .canonicalize()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut reader = Reader {
        root: root.clone(),
        files: BTreeMap::new(),
        total: 0,
    };
    let source = reader.text("plugin.toml")?;
    let manifest =
        PluginManifest::from_toml(root.join("plugin.toml"), &source).map_err(|e| e.to_string())?;
    let component = if let Some(logic) = &manifest.logic {
        if documents_only {
            None
        } else {
            let bytes = match reader.read(logic, MAX_COMPONENT_BYTES) {
                Ok(b) => b,
                Err(_)
                    if fallback.is_some()
                        && !root.join(logic).try_exists().map_err(|e| e.to_string())? =>
                {
                    let b = fallback.unwrap();
                    if b.len() > MAX_COMPONENT_BYTES {
                        return Err("fixture exceeds component limit".into());
                    }
                    reader.files.insert(logic.clone(), b.to_vec());
                    b.to_vec()
                }
                Err(e) => return Err(e),
            };
            Some(bytes)
        }
    } else {
        None
    };
    let mut sets = vec![];
    let mut sources = vec![];
    for entry in &manifest.sets {
        let source = reader.text(&entry.file)?;
        let path = root.join(&entry.file);
        let set = SetFile::from_toml(&path, &source).map_err(|e| e.to_string())?;
        if set.id != entry.id {
            return Err(format!("{}: set id differs from manifest", path.display()));
        }
        set.to_spec(&manifest).map_err(|e| e.to_string())?;
        sources.push((path, source));
        sets.push(set);
    }
    let mut profiles = BTreeMap::new();
    for entry in &manifest.profiles {
        let source = reader.text(&entry.file)?;
        let doc = ProfileDocument::from_toml(root.join(&entry.file), &source)
            .map_err(|e| e.to_string())?;
        if doc.profile.id != entry.id {
            return Err("profile id differs from manifest".into());
        }
        profiles.insert(
            format!("{}/{}", manifest.id, entry.id)
                .parse()
                .map_err(|e: ModelError| e.to_string())?,
            doc,
        );
    }
    let bindings = if root
        .join("bindings.toml")
        .try_exists()
        .map_err(|e| e.to_string())?
    {
        let source = reader.text("bindings.toml")?;
        Some(
            BindingFile::from_toml(root.join("bindings.toml"), &source)
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    let mut assets = BTreeMap::new();
    let mut svg = SvgRasterizer::default();
    for (index, asset) in manifest.assets.iter().enumerate() {
        let bytes = reader.read(
            &asset.file,
            if asset.kind == AssetKind::Png {
                4 * 1024 * 1024
            } else {
                MAX_TEXT_BYTES
            },
        )?;
        let rev = u64::from_le_bytes(Sha256::digest(&bytes)[..8].try_into().expect("hash"));
        let image = match asset.kind {
            AssetKind::SymbolicSvg => svg
                .rasterize(
                    &SvgAsset::new(
                        index as u64,
                        rev,
                        String::from_utf8(bytes).map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?,
                    asset.width,
                    asset.height,
                )
                .map_err(|e| e.to_string())?,
            AssetKind::Png => decode_png(index as u64, rev, &bytes, asset.width, asset.height)?,
        };
        assets.insert(asset.id.clone(), image);
    }
    let ids = assets.keys().map(String::as_str).collect();
    for set in &sets {
        set.validate_assets(&ids).map_err(|e| e.to_string())?;
    }
    Ok(PluginFiles {
        manifest,
        sets,
        profiles,
        bindings,
        assets,
        sources,
        component,
        revision: content_revision(&reader.files),
        root,
    })
}
fn decode_png(
    id: u64,
    revision: u64,
    bytes: &[u8],
    width: u32,
    height: u32,
) -> Result<Image, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    if reader.info().width != width || reader.info().height != height {
        return Err("PNG dimensions differ from declared raster size".into());
    }
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("PNG output too large")?];
    let info = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    match info.color_type {
        png::ColorType::Rgba => pixels.extend_from_slice(&buffer[..info.buffer_size()]),
        png::ColorType::Rgb => {
            for p in buffer[..info.buffer_size()].as_chunks::<3>().0 {
                pixels.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        png::ColorType::Grayscale => {
            for v in &buffer[..info.buffer_size()] {
                pixels.extend_from_slice(&[*v, *v, *v, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for p in buffer[..info.buffer_size()].as_chunks::<2>().0 {
                pixels.extend_from_slice(&[p[0], p[0], p[0], p[1]]);
            }
        }
        png::ColorType::Indexed => return Err("unexpanded PNG palette".into()),
    }
    Image::rgba8(id, revision, width, height, pixels).map_err(|e| e.to_string())
}
