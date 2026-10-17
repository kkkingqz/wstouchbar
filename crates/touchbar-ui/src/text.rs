use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

use cosmic_text::{
    Align, Attrs, Buffer, Color as CosmicColor, Ellipsize, EllipsizeHeightLimit, FontSystem,
    Metrics, Shaping, SwashCache, Weight, Wrap,
};

use crate::{Image, Size, TextAlign};
use anyhow::Result;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialOrd, PartialEq)]
struct TextKey {
    text: String,
    width: u32,
    height: u32,
    size_bits: u32,
    align: u8,
    overflow: TextRasterMode,
    bold: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialOrd, PartialEq)]
pub(crate) enum TextRasterMode {
    Ellipsis,
    Clip,
}

#[derive(Clone, Debug)]
struct CachedText {
    image: Image,
    measured: Size,
}

/// Unicode shaping and theme-neutral alpha-mask cache using injected fonts.
pub struct TextEngine {
    font_system: FontSystem,
    swash_cache: SwashCache,
    cache: BTreeMap<TextKey, CachedText>,
    insertion_order: VecDeque<TextKey>,
    capacity: usize,
    evicted_image_ids: Vec<u64>,
    next_image_id: u64,
}

impl TextEngine {
    pub fn new(font_system: FontSystem) -> Self {
        Self::with_capacity(font_system, 256)
    }

    pub fn with_capacity(font_system: FontSystem, capacity: usize) -> Self {
        Self {
            font_system,
            swash_cache: SwashCache::new(),
            cache: BTreeMap::new(),
            insertion_order: VecDeque::new(),
            capacity: capacity.max(1),
            evicted_image_ids: Vec::new(),
            next_image_id: 1 << 63,
        }
    }

    /// Shape and rasterize a single-line label into a white RGBA alpha mask. Width-constrained text is ellipsized at the visual end.
    pub fn rasterize_mask(
        &mut self,
        text: &str,
        width: u32,
        height: u32,
        size: f32,
        align: TextAlign,
    ) -> Result<&Image> {
        self.rasterize_with_mode(
            text,
            width,
            height,
            size,
            align,
            TextRasterMode::Ellipsis,
            false,
        )
    }

    /// Shape and rasterize a bold single-line label. The trusted fallback row
    /// uses this to match the platform function-key treatment.
    pub fn rasterize_bold_mask(
        &mut self,
        text: &str,
        width: u32,
        height: u32,
        size: f32,
        align: TextAlign,
    ) -> Result<&Image> {
        self.rasterize_with_mode(
            text,
            width,
            height,
            size,
            align,
            TextRasterMode::Ellipsis,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn rasterize_with_mode(
        &mut self,
        text: &str,
        width: u32,
        height: u32,
        size: f32,
        align: TextAlign,
        overflow: TextRasterMode,
        bold: bool,
    ) -> Result<&Image> {
        let width = width.max(1);
        let height = height.max(1);
        crate::display_list::raster_bytes(width, height)?;
        if !size.is_finite() || size <= 0.0 {
            anyhow::bail!("font size must be finite and positive");
        }
        let key = TextKey {
            text: text.into(),
            width,
            height,
            size_bits: size.to_bits(),
            align: match align {
                TextAlign::Leading => 0,
                TextAlign::Center => 1,
                TextAlign::Trailing => 2,
            },
            overflow,
            bold,
        };
        if !self.cache.contains_key(&key) {
            let cached = self.rasterize_uncached(&key, align);
            self.insert(key.clone(), cached);
        }
        Ok(&self.cache[&key].image)
    }

    /// Return the shaped visual size for the same constrained label without
    /// exposing font implementation details to widget code.
    pub fn measure(
        &mut self,
        text: &str,
        width: u32,
        height: u32,
        size: f32,
        align: TextAlign,
    ) -> Result<Size> {
        self.measure_with_mode(
            text,
            width,
            height,
            size,
            align,
            TextRasterMode::Ellipsis,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn measure_with_mode(
        &mut self,
        text: &str,
        width: u32,
        height: u32,
        size: f32,
        align: TextAlign,
        overflow: TextRasterMode,
        bold: bool,
    ) -> Result<Size> {
        let width = width.max(1);
        let height = height.max(1);
        crate::display_list::raster_bytes(width, height)?;
        if !size.is_finite() || size <= 0.0 {
            anyhow::bail!("font size must be finite and positive");
        }
        let key = TextKey {
            text: text.into(),
            width,
            height,
            size_bits: size.to_bits(),
            align: match align {
                TextAlign::Leading => 0,
                TextAlign::Center => 1,
                TextAlign::Trailing => 2,
            },
            overflow,
            bold,
        };
        if !self.cache.contains_key(&key) {
            let cached = self.rasterize_uncached(&key, align);
            self.insert(key.clone(), cached);
        }
        Ok(self.cache[&key].measured)
    }

    pub fn cached_run_count(&self) -> usize {
        self.cache.len()
    }

    pub fn clear_runs(&mut self) {
        self.evicted_image_ids
            .extend(self.cache.values().map(|cached| cached.image.id));
        self.cache.clear();
        self.insertion_order.clear();
    }

    /// Texture owners consume these IDs to release GPU objects paired with
    /// evicted CPU-side runs.
    pub fn take_evicted_image_ids(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.evicted_image_ids)
    }

    fn insert(&mut self, key: TextKey, cached: CachedText) {
        while self.cache.len() >= self.capacity {
            let Some(oldest) = self.insertion_order.pop_front() else {
                break;
            };
            if let Some(evicted) = self.cache.remove(&oldest) {
                self.evicted_image_ids.push(evicted.image.id);
            }
        }
        self.insertion_order.push_back(key.clone());
        self.cache.insert(key, cached);
    }

    // Float raster coordinates are deliberately quantized; dimensions are bounded.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn rasterize_uncached(&mut self, key: &TextKey, align: TextAlign) -> CachedText {
        let size = f32::from_bits(key.size_bits).max(1.0);
        let line_height = (size * 1.2).max(1.0);
        let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(size, line_height));
        buffer.set_size(Some(key.width as f32), Some(key.height as f32));
        buffer.set_wrap(Wrap::None);
        buffer.set_ellipsize(match key.overflow {
            TextRasterMode::Ellipsis => Ellipsize::End(EllipsizeHeightLimit::Lines(1)),
            TextRasterMode::Clip => Ellipsize::None,
        });
        buffer.set_text(
            &key.text,
            &if key.bold {
                Attrs::new().weight(Weight::BOLD)
            } else {
                Attrs::new()
            },
            Shaping::Advanced,
            Some(match align {
                TextAlign::Leading => Align::Left,
                TextAlign::Center => Align::Center,
                TextAlign::Trailing => Align::Right,
            }),
        );
        buffer.shape_until_scroll(&mut self.font_system, false);
        let measured = buffer
            .layout_runs()
            .fold(Size::default(), |size, run| Size {
                width: size.width.max(run.line_w),
                height: size.height.max(run.line_top + run.line_height),
            });
        let y_offset = ((key.height as f32 - measured.height) * 0.5).round() as i32;
        let mut pixels = vec![0_u8; key.width as usize * key.height as usize * 4];
        buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            CosmicColor::rgb(255, 255, 255),
            |x, y, glyph_width, glyph_height, source| {
                for offset_y in 0..glyph_height as i32 {
                    for offset_x in 0..glyph_width as i32 {
                        blend_pixel(
                            &mut pixels,
                            key.width,
                            key.height,
                            x + offset_x,
                            y + y_offset + offset_y,
                            [255, 255, 255, source.a()],
                        );
                    }
                }
            },
        );
        let image_id = self.next_image_id;
        self.next_image_id = self.next_image_id.wrapping_add(1).max(1 << 63);
        CachedText {
            image: Image {
                id: image_id,
                revision: 1,
                width: key.width,
                height: key.height,
                pixels: Arc::from(pixels),
            },
            measured,
        }
    }
}

// Float raster coordinates are deliberately quantized; dimensions are bounded.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn blend_pixel(pixels: &mut [u8], width: u32, height: u32, x: i32, y: i32, source: [u8; 4]) {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 || source[3] == 0 {
        return;
    }
    let index = (y as usize * width as usize + x as usize) * 4;
    let source_alpha = f32::from(source[3]) / 255.0;
    let destination_alpha = f32::from(pixels[index + 3]) / 255.0;
    let output_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
    for channel in 0..3 {
        let source_value = f32::from(source[channel]) / 255.0;
        let destination_value = f32::from(pixels[index + channel]) / 255.0;
        let output = if output_alpha > 0.0 {
            (source_value * source_alpha
                + destination_value * destination_alpha * (1.0 - source_alpha))
                / output_alpha
        } else {
            0.0
        };
        pixels[index + channel] = (output * 255.0).round() as u8;
    }
    pixels[index + 3] = (output_alpha * 255.0).round() as u8;
}

/// Explicit shell policy for systems without installed fonts. Tests may still inject a fixed database.
/// Rebuild the FontSystem after adding the fallback so its fallback indexes include the new face.
pub fn ensure_default_font(fonts: FontSystem) -> FontSystem {
    if fonts.db().faces().next().is_some() {
        return fonts;
    }
    let (locale, mut db) = fonts.into_locale_and_db();
    db.load_font_data(include_bytes!("../../../assets/fonts/test/InterVariable.ttf").to_vec());
    db.set_sans_serif_family("Inter Variable");
    db.set_serif_family("Inter Variable");
    db.set_monospace_family("Inter Variable");
    FontSystem::new_with_locale_and_db(locale, db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_unicode_and_reuses_the_cached_run() {
        let mut engine = TextEngine::new(crate::test_fonts());
        let first = engine
            .rasterize_mask("TouchBar مرحبا", 180, 32, 16.0, TextAlign::Center)
            .unwrap()
            .clone();
        let second = engine
            .rasterize_mask("TouchBar مرحبا", 180, 32, 16.0, TextAlign::Center)
            .unwrap()
            .clone();
        assert_eq!(first.id, second.id);
        assert_eq!(engine.cached_run_count(), 1);
        assert!(
            first
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] > 0)
        );
    }

    #[test]
    fn bold_and_regular_runs_have_distinct_cache_entries() {
        let mut engine = TextEngine::new(crate::test_fonts());
        let regular = engine
            .rasterize_mask("F12", 96, 42, 32.0, TextAlign::Center)
            .unwrap()
            .id;
        let bold = engine
            .rasterize_bold_mask("F12", 96, 42, 32.0, TextAlign::Center)
            .unwrap()
            .id;

        assert_ne!(regular, bold);
        assert_eq!(engine.cached_run_count(), 2);
    }

    #[test]
    fn shaped_measurement_respects_the_constraint() {
        let mut engine = TextEngine::new(crate::test_fonts());
        let measured = engine
            .measure(
                "A label that must ellipsize",
                72,
                24,
                14.0,
                TextAlign::Leading,
            )
            .unwrap();
        assert!(measured.width <= 72.5);
        assert!(measured.height > 0.0);
    }

    #[test]
    fn cache_is_bounded_and_reports_gpu_evictions() {
        let mut engine = TextEngine::with_capacity(crate::test_fonts(), 2);
        for label in ["one", "two", "three"] {
            engine
                .rasterize_mask(label, 80, 24, 14.0, TextAlign::Leading)
                .unwrap();
        }
        assert_eq!(engine.cached_run_count(), 2);
        assert_eq!(engine.take_evicted_image_ids().len(), 1);
        assert!(engine.take_evicted_image_ids().is_empty());
    }
}
