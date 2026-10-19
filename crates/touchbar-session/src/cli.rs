use std::{collections::BTreeSet, path::PathBuf};
use touchbar_loader::DesktopChoice;
use touchbar_model::*;
#[derive(Clone, Debug, Default)]
pub struct Cli {
    pub help: bool,
    pub plugins: Option<PathBuf>,
    pub simulator: bool,
    pub surfaces: Vec<SurfaceInfo>,
    pub headless: Option<SurfaceInfo>,
    pub desktop: Option<DesktopChoice>,
    pub fake_effects: bool,
    pub touchbard_socket: Option<PathBuf>,
}
pub const USAGE: &str = "touchbar-session [--plugins DIR] [--touchbard-socket PATH] [--simulator --surface NAME=WxH@ppmm ...] [--headless WxH@ppmm] [--desktop=null|fake|auto|gnome] [--effects=fake]";
impl Cli {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut cli = Self::default();
        let mut args = args.into_iter();
        let mut seen = BTreeSet::new();
        while let Some(arg) = args.next() {
            let (key, inline) = arg
                .split_once('=')
                .map_or((arg.as_str(), None), |(k, v)| (k, Some(v.to_owned())));
            if key != "--surface" && !seen.insert(key.to_owned()) {
                return Err(format!("duplicate option {key}"));
            }
            match key {
                "--help" | "-h" if inline.is_none() => cli.help = true,
                "--simulator" if inline.is_none() => cli.simulator = true,
                "--plugins" | "--surface" | "--headless" | "--desktop" | "--effects"
                | "--touchbard-socket" => {
                    let value = inline
                        .or_else(|| args.next())
                        .ok_or_else(|| format!("{key} requires a value"))?;
                    match key {
                        "--touchbard-socket" => {
                            let path = PathBuf::from(value);
                            if !path.is_absolute() || path.file_name().is_none() {
                                return Err("touchbard socket must be an absolute file path".into());
                            }
                            cli.touchbard_socket = Some(path);
                        }
                        "--plugins" => cli.plugins = Some(value.into()),
                        "--surface" => {
                            let (name, geometry) = value
                                .split_once('=')
                                .unwrap_or(("touchbar", value.as_str()));
                            let name: SurfaceName =
                                name.parse().map_err(|e: ModelError| e.to_string())?;
                            let kind = if name.as_str() == "touchbar" {
                                SurfaceKind::Touchbar
                            } else if name.as_str().starts_with("window:") {
                                SurfaceKind::Window
                            } else {
                                return Err(
                                    "simulator surfaces must be touchbar or window:NAME".into()
                                );
                            };
                            if cli.surfaces.len() >= 8
                                || cli.surfaces.iter().any(|s| s.name == name)
                            {
                                return Err("duplicate simulator name or surface limit".into());
                            }
                            let mut info = parse_surface(geometry, kind)?;
                            info.id = SurfaceId(
                                u64::try_from(cli.surfaces.len()).expect("bounded surface count")
                                    + 1,
                            );
                            info.name = name;
                            info.caps = SurfaceCaps {
                                watch_keys: kind == SurfaceKind::Touchbar,
                                inject_keys: kind == SurfaceKind::Touchbar,
                                damage: true,
                                ..SurfaceCaps::default()
                            };
                            cli.surfaces.push(info);
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
        if !cli.surfaces.is_empty() && !cli.simulator {
            return Err("--surface requires --simulator".into());
        }
        if cli.simulator && cli.surfaces.is_empty() {
            let mut info = parse_surface("2008x60@6.2ppmm", SurfaceKind::Touchbar)?;
            info.caps.inject_keys = true;
            info.caps.damage = true;
            cli.surfaces.push(info);
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
