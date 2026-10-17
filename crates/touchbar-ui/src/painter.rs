use crate::{
    DisplayList, Icon, ImageColoring, LayoutMeasurer, MotionPolicy, Primitive, Rect, Scene, Size,
    Symbol, SymbolCatalog, TextAlign, TextEngine, TextOverflow, text::TextRasterMode,
};
use anyhow::Result;
use cosmic_text::FontSystem;
use std::{cell::RefCell, time::Duration};

/// Session-owned CPU engines. Font discovery is an explicit caller decision;
/// tests can inject a database containing only the pinned test font.
pub struct CpuPainter {
    text: RefCell<TextEngine>,
    symbols: SymbolCatalog,
}
impl CpuPainter {
    pub fn new(font_system: FontSystem) -> Self {
        Self {
            text: RefCell::new(TextEngine::new(font_system)),
            symbols: SymbolCatalog::default(),
        }
    }
    fn with_text<T>(&self, work: impl FnOnce(&mut TextEngine) -> T) -> T {
        let mut engine = self.text.borrow_mut();
        let result = work(&mut engine);
        // GL owns a content-addressed resource cache. The legacy texture-ID
        // notifications have no consumer here, even when an operation fails.
        engine.take_evicted_image_ids();
        result
    }
    pub fn display_list(
        &mut self,
        scene: &Scene,
        now: Duration,
        policy: MotionPolicy,
    ) -> Result<DisplayList> {
        let scene = self.materialize(scene, now, policy)?;
        DisplayList::from_scene(&scene, now, policy)
    }
    // Float raster coordinates are deliberately quantized; dimensions are bounded.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn materialize(
        &mut self,
        scene: &Scene,
        now: Duration,
        policy: MotionPolicy,
    ) -> Result<Scene> {
        let mut result = Scene::new(scene.clear);
        for primitive in &scene.primitives {
            match primitive {
                Primitive::Text {
                    rect,
                    text,
                    size,
                    color,
                    align,
                    overflow,
                } => {
                    let rect = crate::display_list::physical_rect(*rect)?;
                    if rect.width == 0.0 || rect.height == 0.0 {
                        continue;
                    }
                    let height = rect.height as u32;
                    let width = if matches!(overflow, TextOverflow::Marquee { .. }) {
                        8192
                    } else {
                        rect.width as u32
                    };
                    crate::display_list::raster_bytes(width, height)?;
                    let (mode, align) = match overflow {
                        TextOverflow::Ellipsis => (TextRasterMode::Ellipsis, *align),
                        TextOverflow::Clip => (TextRasterMode::Clip, *align),
                        TextOverflow::Marquee { .. } => (TextRasterMode::Clip, TextAlign::Leading),
                    };
                    let (measured, image) = self.with_text(|engine| -> Result<_> {
                        let measured = engine
                            .measure_with_mode(text, width, height, *size, align, mode, false)?;
                        let raster_width = if matches!(overflow, TextOverflow::Marquee { .. }) {
                            measured.width.ceil().max(1.0) as u32
                        } else {
                            width
                        };
                        let image = engine
                            .rasterize_with_mode(
                                text,
                                raster_width,
                                height,
                                *size,
                                align,
                                mode,
                                false,
                            )?
                            .clone();
                        Ok((measured, image))
                    })?;
                    result.push_clip(rect);
                    match overflow {
                        TextOverflow::Marquee { speed, gap } if measured.width > rect.width => {
                            let cycle = image.width as f32 + gap.max(1.0);
                            let offset = if policy == MotionPolicy::Full {
                                (now.as_secs_f32() * speed.max(0.0)) % cycle
                            } else {
                                0.0
                            };
                            for x in [rect.x - offset, rect.x - offset + cycle] {
                                result.colored_image(
                                    Rect::new(x, rect.y, image.width as f32, rect.height),
                                    image.clone(),
                                    1.0,
                                    ImageColoring::Mask(*color),
                                );
                            }
                        }
                        _ => result.colored_image(
                            Rect::new(rect.x, rect.y, image.width as f32, rect.height),
                            image,
                            1.0,
                            ImageColoring::Mask(*color),
                        ),
                    }
                    result.pop_clip();
                }
                Primitive::Icon { rect, icon, color } => {
                    let rect = crate::display_list::physical_rect(*rect)?;
                    let pixels = rect.width.min(rect.height).max(1.0) as u32;
                    let symbol = match icon {
                        Icon::Volume => Symbol::Volume,
                        Icon::Muted => Symbol::Muted,
                        Icon::Play => Symbol::Play,
                        Icon::Pause => Symbol::Pause,
                        Icon::Check => Symbol::Check,
                        Icon::ChevronLeft => Symbol::ChevronLeft,
                        Icon::ChevronRight => Symbol::ChevronRight,
                    };
                    let image = self.symbols.image(symbol, pixels)?;
                    let side = pixels as f32;
                    result.colored_image(
                        Rect::new(
                            rect.x + (rect.width - side) * 0.5,
                            rect.y + (rect.height - side) * 0.5,
                            side,
                            side,
                        ),
                        image,
                        1.0,
                        ImageColoring::Mask(*color),
                    );
                }
                _ => result.primitives.push(primitive.clone()),
            }
        }
        Ok(result)
    }
}
impl LayoutMeasurer for CpuPainter {
    // Float raster coordinates are deliberately quantized; dimensions are bounded.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn measure_text(&self, text: &str, size: f32, maximum: Size) -> Size {
        let width = maximum.width.clamp(1.0, 8192.0) as u32;
        let height = maximum.height.clamp(1.0, 512.0) as u32;
        self.with_text(|engine| engine.measure(text, width, height, size, TextAlign::Leading))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_painter() -> CpuPainter {
        let mut db = cosmic_text::fontdb::Database::new();
        db.load_font_data(include_bytes!("../../../assets/fonts/test/InterVariable.ttf").to_vec());
        db.set_sans_serif_family("Inter Variable");
        CpuPainter {
            text: RefCell::new(TextEngine::with_capacity(
                FontSystem::new_with_locale_and_db("en-US".into(), db),
                2,
            )),
            symbols: SymbolCatalog::default(),
        }
    }

    #[test]
    fn measurement_evictions_leave_no_unused_id_history() {
        let mut painter = small_painter();
        for number in 0..100 {
            let size = painter.measure_text(&number.to_string(), 12.0, Size::new(80.0, 20.0));
            assert!(size.width > 0.0);
            let text = painter.text.get_mut();
            assert!(text.cached_run_count() <= 2);
            assert!(
                text.take_evicted_image_ids().is_empty(),
                "CPU owner must consume obsolete texture IDs"
            );
        }
    }

    #[test]
    fn raster_evictions_are_drained_even_if_later_primitive_fails() {
        let mut painter = small_painter();
        for number in 0..100 {
            let mut scene = Scene::new(crate::Color::TRANSPARENT);
            scene.text(
                Rect::new(0.0, 0.0, 80.0, 20.0),
                number.to_string(),
                12.0,
                crate::Color::WHITE,
                TextAlign::Leading,
            );
            scene.text(
                Rect::new(f32::NAN, 0.0, 80.0, 20.0),
                "bad",
                12.0,
                crate::Color::WHITE,
                TextAlign::Leading,
            );
            assert!(
                painter
                    .materialize(&scene, Duration::ZERO, MotionPolicy::Full)
                    .is_err()
            );
            let text = painter.text.get_mut();
            assert!(text.cached_run_count() <= 2);
            assert!(
                text.take_evicted_image_ids().is_empty(),
                "failed frames must not accumulate eviction IDs"
            );
        }
    }
}
