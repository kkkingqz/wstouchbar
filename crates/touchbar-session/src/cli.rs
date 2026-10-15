use std::{collections::BTreeSet, path::PathBuf};
use touchbar_loader::DesktopChoice;
use touchbar_model::*;
#[derive(Clone, Debug, Default)]
pub struct Cli {
    pub help: bool,
    pub plugins: Option<PathBuf>,
    pub simulator: bool,
    pub surface: Option<SurfaceInfo>,
    pub headless: Option<SurfaceInfo>,
    pub desktop: Option<DesktopChoice>,
    pub fake_effects: bool,
}
pub const USAGE: &str = "touchbar-session [--plugins DIR] [--simulator --surface WxH@ppmm] [--headless WxH@ppmm] [--desktop=null|fake|auto|gnome] [--effects=fake]";
impl Cli {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut cli = Self::default();
        let mut args = args.into_iter();
        let mut seen = BTreeSet::new();
        while let Some(arg) = args.next() {
            let (key, inline) = arg
                .split_once('=')
                .map_or((arg.as_str(), None), |(k, v)| (k, Some(v.to_owned())));
            if !seen.insert(key.to_owned()) {
                return Err(format!("duplicate option {key}"));
            }
            match key {
                "--help" | "-h" if inline.is_none() => cli.help = true,
                "--simulator" if inline.is_none() => cli.simulator = true,
                "--plugins" | "--surface" | "--headless" | "--desktop" | "--effects" => {
                    let value = inline
                        .or_else(|| args.next())
                        .ok_or_else(|| format!("{key} requires a value"))?;
                    match key {
                        "--plugins" => cli.plugins = Some(value.into()),
                        "--surface" => {
                            cli.surface = Some(parse_surface(&value, SurfaceKind::Window)?)
                        }
                        "--headless" => {
                            cli.headless = Some(parse_surface(&value, SurfaceKind::Headless)?)
                        }
                        "--desktop" => {
                            cli.desktop = Some(match value.as_str() {
                                "null" => DesktopChoice::Null,
                                "fake" => DesktopChoice::Fake,
                                "auto" => DesktopChoice::Auto,
                                "gnome" => DesktopChoice::Gnome,
                                _ => return Err("invalid desktop backend".into()),
                            })
                        }
                        "--effects" => match value.as_str() {
                            "fake" => cli.fake_effects = true,
                            "native" => cli.fake_effects = false,
                            _ => return Err("effects must be native or fake".into()),
                        },
                        _ => unreachable!(),
                    }
                }
                _ => return Err(format!("unknown option {arg}")),
            }
        }
        if cli.headless.is_some() && cli.simulator {
            return Err("choose headless or simulator".into());
        }
        if cli.surface.is_some() && !cli.simulator {
            return Err("--surface requires --simulator".into());
        }
        Ok(cli)
    }
}
pub fn parse_surface(value: &str, kind: SurfaceKind) -> Result<SurfaceInfo, String> {
    let (size, density) = value.split_once('@').ok_or("surface must be WxH@ppmm")?;
    let (width, height) = size.split_once('x').ok_or("surface must be WxH@ppmm")?;
    let width = width.parse::<u32>().map_err(|_| "invalid surface width")?;
    let height = height
        .parse::<u32>()
        .map_err(|_| "invalid surface height")?;
    let density = density
        .strip_suffix("ppmm")
        .unwrap_or(density)
        .parse::<f32>()
        .map_err(|_| "invalid surface density")?;
    if width > 8192 || height > 8192 || u64::from(width) * u64::from(height) > 4 * 1024 * 1024 {
        return Err("surface exceeds renderer dimensions budget".into());
    }
    let info = SurfaceInfo {
        id: SurfaceId(1),
        name: match kind {
            SurfaceKind::Headless => "headless:main",
            SurfaceKind::Window => "window:simulator",
            SurfaceKind::Touchbar => "touchbar",
        }
        .parse()
        .map_err(|e: ModelError| e.to_string())?,
        kind,
        width_px: width,
        height_px: height,
        px_per_mm: density,
        caps: SurfaceCaps {
            watch_keys: true,
            inject_keys: kind == SurfaceKind::Headless,
            ..SurfaceCaps::default()
        },
    };
    info.validate().map_err(|e| e.to_string())?;
    Ok(info)
}
