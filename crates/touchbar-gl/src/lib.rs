//! Thread-owned offscreen EGL/GLES renderer for complete display packets.
mod device;
mod effect_glsl;
mod egl;
mod native;
mod shaders;
use std::path::{Path, PathBuf};
use touchbar_model::SurfaceId;
use touchbar_ui::DisplayList;
#[derive(Clone, Debug, Default)]
pub enum Backend {
    #[default]
    Auto,
    RenderNode(PathBuf),
    Surfaceless,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orientation {
    #[default]
    Landscape,
    Clockwise,
    CounterClockwise,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug)]
pub struct RenderedFrame {
    pub width: u32,
    pub height: u32,
    /// Top-down premultiplied RGBA.
    pub pixels: Vec<u8>,
    pub damage: Vec<DamageRect>,
}
impl RenderedFrame {
    pub fn save_png(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.width > 0 && self.height > 0,
            "PNG dimensions must be positive"
        );
        let bytes = u64::from(self.width)
            .checked_mul(u64::from(self.height))
            .and_then(|n| n.checked_mul(4))
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| anyhow::anyhow!("PNG dimensions overflow"))?;
        anyhow::ensure!(
            self.pixels.len() == bytes,
            "PNG RGBA payload length mismatch"
        );
        let mut straight = self.pixels.clone();
        for pixel in straight.as_chunks_mut::<4>().0 {
            let alpha = u32::from(pixel[3]);
            for channel in &mut pixel[..3] {
                *channel = u8::try_from(
                    (u32::from(*channel) * 255 + alpha / 2)
                        .checked_div(alpha)
                        .unwrap_or(0)
                        .min(255),
                )?;
            }
        }
        let writer = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut encoder = png::Encoder::new(writer, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&straight)?;
        writer.finish()?;
        Ok(())
    }
}
pub struct Renderer(native::Native);
impl Renderer {
    pub fn new(backend: Backend) -> anyhow::Result<Self> {
        Ok(Self(native::Native::new(&backend)?))
    }
    pub fn backend_description(&self) -> &str {
        self.0.description()
    }
    pub fn render(
        &mut self,
        surface: SurfaceId,
        size: (u32, u32),
        orientation: Orientation,
        list: &DisplayList,
        force_full: bool,
    ) -> anyhow::Result<Option<RenderedFrame>> {
        self.0.render(surface, size, orientation, list, force_full)
    }
    pub fn remove_surface(&mut self, surface: SurfaceId) {
        self.0.remove_surface(surface);
    }
}
