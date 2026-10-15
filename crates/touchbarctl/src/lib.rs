//! Offline rendering and private session control.
use std::{collections::BTreeSet, path::PathBuf};
use touchbar_core::{InstanceStatus, Scene, SceneItem};
use touchbar_loader::{LoadRequest, offline_set, read_bounded};
use touchbar_model::*;
use touchbar_scene::{Frame, SceneRenderer};
use touchbar_ui::{CpuPainter, MotionPolicy, Theme};
#[derive(Clone, Debug)]
pub struct RenderOptions {
    pub set: SetRef,
    pub state: Option<PathBuf>,
    pub surface: SurfaceInfo,
    pub theme: Theme,
    pub plugins: Option<PathBuf>,
    pub output: PathBuf,
}
impl RenderOptions {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut args = args.into_iter();
        let set = args
            .next()
            .ok_or("render requires PLUGIN/SET")?
            .parse()
            .map_err(|e: ModelError| e.to_string())?;
        let mut state = None;
        let mut surface = parse_surface("2008x60@6.2ppmm")?;
        let mut theme = Theme::dark();
        let mut plugins = None;
        let mut output = None;
        let mut seen = BTreeSet::new();
        while let Some(option) = args.next() {
            if !seen.insert(option.clone()) {
                return Err(format!("duplicate argument {option}"));
            }
            let value = args
                .next()
                .ok_or_else(|| format!("{option} requires a value"))?;
            match option.as_str() {
                "--state" => state = Some(value.into()),
                "--surface" => surface = parse_surface(&value)?,
                "--theme" => {
                    theme = match value.as_str() {
                        "dark" => Theme::dark(),
                        "light" => Theme::light(),
                        _ => return Err("theme must be dark or light".into()),
                    }
                }
                "--plugins" => plugins = Some(value.into()),
                "-o" => output = Some(PathBuf::from(value)),
                _ => return Err(format!("unknown render argument {option}")),
            }
        }
        Ok(Self {
            set,
            state,
            surface,
            theme,
            plugins,
            output: output.ok_or("render requires -o FILE")?,
        })
    }
}
pub fn parse_surface(text: &str) -> Result<SurfaceInfo, String> {
    let (dimensions, density) = text.split_once('@').unwrap_or((text, "6.2ppmm"));
    let (w, h) = dimensions
        .split_once('x')
        .ok_or("surface requires WxH[@Nppmm]")?;
    let width_px = w.parse().map_err(|_| "invalid surface width")?;
    let height_px = h.parse().map_err(|_| "invalid surface height")?;
    let px_per_mm = density
        .strip_suffix("ppmm")
        .unwrap_or(density)
        .parse()
        .map_err(|_| "invalid surface density")?;
    let surface = SurfaceInfo {
        id: SurfaceId(1),
        name: "headless:render".parse().expect("static name"),
        kind: SurfaceKind::Headless,
        width_px,
        height_px,
        px_per_mm,
        caps: SurfaceCaps::default(),
    };
    surface.validate().map_err(|e| e.to_string())?;
    if width_px > 8192
        || height_px > 8192
        || u64::from(width_px) * u64::from(height_px) > 4 * 1024 * 1024
    {
        return Err("surface exceeds renderer limits".into());
    }
    Ok(surface)
}
pub fn render_frame(options: &RenderOptions, painter: &mut CpuPainter) -> Result<Frame, String> {
    let request = LoadRequest::standard(options.plugins.clone())?;
    let (spec, documents) = offline_set(&request.paths, &options.set)?;
    let state: StatePatch = if let Some(file) = &options.state {
        serde_json::from_slice(&read_bounded(file, 256 * 1024)?).map_err(|e| e.to_string())?
    } else {
        vec![]
    };
    if state.len() > 256
        || state
            .iter()
            .any(|(name, _)| name.is_empty() || name.len() > 128)
        || state.iter().map(|(n, _)| n).collect::<BTreeSet<_>>().len() != state.len()
    {
        return Err("invalid state field names or count".into());
    }
    let units = UnitSize::resolve(&options.surface, spec.units, None);
    let scene = Scene {
        items: vec![SceneItem {
            instance: InstanceId {
                number: 1,
                generation: Generation(1),
            },
            slot: "slot".parse().expect("static slot"),
            set: spec,
            state,
            plane: ScenePlane::Profile,
            units,
            status: InstanceStatus::Visible,
            interactions: vec![],
        }],
        ..Default::default()
    };
    let frame = SceneRenderer::default()
        .frame(
            &options.surface,
            &scene,
            &documents,
            options.theme,
            painter,
            Timestamp(0),
            MotionPolicy::Full,
        )
        .map_err(|e| e.to_string())?;
    if !frame.faults.is_empty() {
        return Err(format!("render failed: {:?}", frame.faults));
    }
    Ok(frame)
}
pub fn render_png(options: &RenderOptions) -> Result<(), String> {
    let mut painter = CpuPainter::new(touchbar_ui::ensure_default_font(
        cosmic_text::FontSystem::new(),
    ));
    let frame = render_frame(options, &mut painter)?;
    let mut renderer =
        touchbar_gl::Renderer::new(touchbar_gl::Backend::Auto).map_err(|e| e.to_string())?;
    let output = renderer
        .render(
            options.surface.id,
            (options.surface.width_px, options.surface.height_px),
            touchbar_gl::Orientation::Landscape,
            &frame.display_list,
            true,
        )
        .map_err(|e| e.to_string())?
        .ok_or("renderer produced no frame")?;
    output.save_png(&options.output).map_err(|e| e.to_string())
}

mod control;
pub use control::*;
