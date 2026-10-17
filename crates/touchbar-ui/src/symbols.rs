use anyhow::Result;

use crate::{ColorRole, Image, ImageFit, ImageTint, Node, SvgAsset, SvgRasterizer};

macro_rules! svg {
    ($body:expr) => {
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><style>path,rect,circle{fill:none;stroke:#fff;stroke-width:2.5;stroke-linecap:round;stroke-linejoin:round}.fill{fill:#fff;stroke:none}</style>"#,
            $body,
            "</svg>"
        )
    };
}

/// Theme-neutral built-in symbols. They are rasterized once and colored by
/// the GLES mask shader whenever the live theme changes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u64)]
pub enum Symbol {
    Volume = 1,
    Muted,
    Play,
    Pause,
    Previous,
    Next,
    Microphone,
    Brightness,
    Battery,
    Wifi,
    Bluetooth,
    Workspace,
    Capture,
    Timer,
    Terminal,
    Graph,
    Check,
    ChevronLeft,
    ChevronRight,
    Unknown,
}

impl Symbol {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Volume => "Volume",
            Self::Muted => "Muted",
            Self::Play => "Play",
            Self::Pause => "Pause",
            Self::Previous | Self::ChevronLeft => "Previous",
            Self::Next | Self::ChevronRight => "Next",
            Self::Microphone => "Microphone",
            Self::Brightness => "Brightness",
            Self::Battery => "Battery",
            Self::Wifi => "Wi-Fi",
            Self::Bluetooth => "Bluetooth",
            Self::Workspace => "Workspace",
            Self::Capture => "Capture",
            Self::Timer => "Timer",
            Self::Terminal => "Terminal",
            Self::Graph => "Graph",
            Self::Check => "Check",
            Self::Unknown => "Unknown symbol",
        }
    }

    const fn source(self) -> &'static str {
        match self {
            Self::Volume => svg!(
                r#"<path d="M5 13h5l6-5v16l-6-5H5z"/><path d="M20 11c2 2 2 8 0 10M23 8c5 5 5 13 0 18"/>"#
            ),
            Self::Muted => {
                svg!(r#"<path d="M5 13h5l6-5v16l-6-5H5z"/><path d="m21 13 7 7m0-7-7 7"/>"#)
            }
            Self::Play => svg!(r#"<path class="fill" d="m10 6 16 10-16 10z"/>"#),
            Self::Pause => svg!(r#"<path class="fill" d="M8 6h6v20H8zm10 0h6v20h-6z"/>"#),
            Self::Previous => svg!(r#"<path class="fill" d="M7 7h4v18H7zm19 0L12 16l14 9z"/>"#),
            Self::Next => svg!(r#"<path class="fill" d="M21 7h4v18h-4zM6 7l14 9-14 9z"/>"#),
            Self::Microphone => svg!(
                r#"<rect class="fill" x="11" y="4" width="10" height="17" rx="5"/><path d="M7 16a9 9 0 0 0 18 0M16 25v4m-5 0h10"/>"#
            ),
            Self::Brightness => svg!(
                r#"<circle cx="16" cy="16" r="6"/><path d="M16 2v5m0 18v5M2 16h5m18 0h5M6 6l4 4m12 12 4 4M26 6l-4 4M10 22l-4 4"/>"#
            ),
            Self::Battery => svg!(
                r#"<rect x="3" y="9" width="24" height="14" rx="3"/><path d="M29 13v6"/><path class="fill" d="M7 13h13v6H7z"/>"#
            ),
            Self::Wifi => svg!(
                r#"<path d="M3 12c8-8 18-8 26 0M8 17c5-5 11-5 16 0m-11 5c2-2 4-2 6 0"/><circle class="fill" cx="16" cy="27" r="2"/>"#
            ),
            Self::Bluetooth => svg!(r#"<path d="m11 8 12 16-7 5V3l7 5-12 16"/>"#),
            Self::Workspace => svg!(
                r#"<rect x="3" y="5" width="11" height="9" rx="2"/><rect x="18" y="5" width="11" height="9" rx="2"/><rect x="3" y="18" width="11" height="9" rx="2"/><rect x="18" y="18" width="11" height="9" rx="2"/>"#
            ),
            Self::Capture => svg!(
                r#"<path d="M10 5H5v5m17-5h5v5M10 27H5v-5m17 5h5v-5"/><circle cx="16" cy="16" r="6"/>"#
            ),
            Self::Timer => {
                svg!(r#"<circle cx="16" cy="18" r="11"/><path d="M16 7V3m-4 0h8m-4 15 5-4"/>"#)
            }
            Self::Terminal => svg!(
                r#"<rect x="3" y="5" width="26" height="22" rx="3"/><path d="m8 11 5 5-5 5m8 1h8"/>"#
            ),
            Self::Check => svg!(r#"<path d="m6 16 7 7L27 8"/>"#),
            Self::ChevronLeft => svg!(r#"<path d="m21 6-10 10 10 10"/>"#),
            Self::ChevronRight => svg!(r#"<path d="m11 6 10 10-10 10"/>"#),
            Self::Unknown => svg!(
                r#"<rect x="4" y="4" width="24" height="24" rx="4"/><path d="M12 11c0-6 12-6 8 1l-4 4m0 6v1"/>"#
            ),
            Self::Graph => svg!(r#"<path d="M4 27V5m0 22h24M7 22l6-7 5 4 9-11"/>"#),
        }
    }
}

pub struct SymbolCatalog {
    rasterizer: SvgRasterizer,
    warned: std::collections::BTreeSet<String>,
    warnings: Vec<String>,
}

impl Default for SymbolCatalog {
    fn default() -> Self {
        Self::new(64)
    }
}

impl SymbolCatalog {
    pub fn new(capacity: usize) -> Self {
        Self {
            rasterizer: SvgRasterizer::new(capacity),
            warned: Default::default(),
            warnings: Vec::new(),
        }
    }

    pub fn image(&mut self, symbol: Symbol, pixels: u32) -> Result<Image> {
        let asset = SvgAsset::new(0x5359_4d42_4f4c_0000 | symbol as u64, 1, symbol.source())?;
        self.rasterizer.rasterize(&asset, pixels, pixels)
    }

    pub fn node(
        &mut self,
        symbol: Symbol,
        pixels: u32,
        color: ColorRole,
        label: impl Into<String>,
    ) -> Result<Node> {
        Ok(Node::Image {
            image: self.image(symbol, pixels)?,
            opacity: 1.0,
            fit: ImageFit::Contain,
            tint: ImageTint::Mask(color),
            label: label.into(),
        })
    }

    pub fn named_image(&mut self, name: &str, pixels: u32) -> Result<Image> {
        let symbol = match name {
            "volume" => Symbol::Volume,
            "muted" | "volume-muted" => Symbol::Muted,
            "play" => Symbol::Play,
            "pause" => Symbol::Pause,
            "previous" => Symbol::Previous,
            "next" => Symbol::Next,
            "microphone" | "mic" => Symbol::Microphone,
            "brightness" => Symbol::Brightness,
            "battery" => Symbol::Battery,
            "wifi" | "wi-fi" => Symbol::Wifi,
            "bluetooth" => Symbol::Bluetooth,
            "workspace" => Symbol::Workspace,
            "capture" => Symbol::Capture,
            "timer" => Symbol::Timer,
            "terminal" => Symbol::Terminal,
            "graph" => Symbol::Graph,
            "check" => Symbol::Check,
            "chevron-left" => Symbol::ChevronLeft,
            "chevron-right" => Symbol::ChevronRight,
            _ => {
                if self.warned.insert(name.into()) {
                    self.warnings
                        .push(format!("unknown symbol {name:?}; using placeholder"));
                }
                Symbol::Unknown
            }
        };
        self.image(symbol, pixels)
    }
    /// Drain diagnostic messages for the caller's logger, once per unknown name.
    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    pub fn cached_count(&self) -> usize {
        self.rasterizer.cached_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_symbol_rasterizes_and_uses_the_cache() {
        let symbols = [
            Symbol::Volume,
            Symbol::Muted,
            Symbol::Play,
            Symbol::Pause,
            Symbol::Previous,
            Symbol::Next,
            Symbol::Microphone,
            Symbol::Brightness,
            Symbol::Battery,
            Symbol::Wifi,
            Symbol::Bluetooth,
            Symbol::Workspace,
            Symbol::Capture,
            Symbol::Timer,
            Symbol::Terminal,
            Symbol::Graph,
        ];
        let mut catalog = SymbolCatalog::default();
        for symbol in symbols {
            let image = catalog.image(symbol, 24).unwrap();
            assert!(
                image
                    .pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel[3] > 0)
            );
        }
        assert_eq!(catalog.cached_count(), symbols.len());
        catalog.image(Symbol::Volume, 24).unwrap();
        assert_eq!(catalog.cached_count(), symbols.len());
    }
}
