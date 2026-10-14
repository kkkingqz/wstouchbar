//! CPU-only retained UI, shaping, assets and portable display packets.

use std::{sync::Arc, time::Duration};

mod asset;
mod controls;
mod display_list;
mod effect;
mod foundation;
mod painter;
mod symbols;
mod text;
pub use display_list::{DisplayList, DrawCommand, EffectUniforms, Resource};
pub use painter::CpuPainter;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrubberItemState {
    pub selected: bool,
    pub highlighted: bool,
}

pub use asset::{SvgAsset, SvgRasterizer};
pub use controls::{ContinuousValue, MeterStyle, ScrubberCellStyle, SliderStyle, TinyGraphStyle};
pub use effect::{
    EffectProgram, MAX_EFFECT_NODES, MAX_EFFECT_PARAMETERS, MAX_EFFECT_PROGRAMS,
    MAX_EFFECT_SOURCE_BYTES, ShaderEffect,
};
pub use foundation::{
    CanvasColor, CanvasCommand, CanvasLinearGradient, CanvasPaint, ColorRole, CrossAxisAlignment,
    Flex, FlexItem, FrameScheduler, Icon, ImageFit, ImageTint, InspectorSnapshot, LayoutMeasurer,
    Node, PressableStyle, ProgressValue, Representation, ResolvedUi, ResponsiveVariant, RetainedUi,
    SemanticNode, SemanticRole, TextAlign, TextMeasurement, resolve_flex_row,
};
pub use symbols::{Symbol, SymbolCatalog};
pub use text::TextEngine;

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MotionId(pub u64);

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct VisualTransform {
    pub translation: Point,
    pub scale: f32,
    pub opacity: f32,
}

impl VisualTransform {
    pub const IDENTITY: Self = Self {
        translation: Point::new(0.0, 0.0),
        scale: 1.0,
        opacity: 1.0,
    };
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Easing {
    Linear,
    #[default]
    EaseInOut,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MotionPolicy {
    Full,
    Reduced,
    Disabled,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MotionPlayback {
    #[default]
    Once,
    Loop,
    Alternate,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub id: MotionId,
    pub from: VisualTransform,
    pub to: VisualTransform,
    pub started: Duration,
    pub duration: Duration,
    pub easing: Easing,
    pub playback: MotionPlayback,
}

impl Motion {
    pub fn sample(self, now: Duration, policy: MotionPolicy) -> VisualTransform {
        if policy != MotionPolicy::Full || self.duration.is_zero() {
            return self.to;
        }
        let elapsed = now.saturating_sub(self.started).as_secs_f32();
        let duration = self.duration.as_secs_f32();
        let linear = match self.playback {
            MotionPlayback::Once => (elapsed / duration).clamp(0.0, 1.0),
            MotionPlayback::Loop => (elapsed / duration).rem_euclid(1.0),
            MotionPlayback::Alternate => {
                let cycle = elapsed / duration;
                let fraction = cycle.rem_euclid(1.0);
                if cycle.floor().rem_euclid(2.0) == 0.0 {
                    fraction
                } else {
                    1.0 - fraction
                }
            }
        };
        let amount = match self.easing {
            Easing::Linear => linear,
            Easing::EaseInOut => linear * linear * (3.0 - 2.0 * linear),
        };
        VisualTransform {
            translation: Point::new(
                self.from.translation.x
                    + (self.to.translation.x - self.from.translation.x) * amount,
                self.from.translation.y
                    + (self.to.translation.y - self.from.translation.y) * amount,
            ),
            scale: self.from.scale + (self.to.scale - self.from.scale) * amount,
            opacity: self.from.opacity + (self.to.opacity - self.from.opacity) * amount,
        }
    }

    pub fn is_active(self, now: Duration, policy: MotionPolicy) -> bool {
        if policy != MotionPolicy::Full || self.duration.is_zero() {
            return false;
        }
        match self.playback {
            MotionPlayback::Once => now < self.started.saturating_add(self.duration),
            MotionPlayback::Loop | MotionPlayback::Alternate => true,
        }
    }
}

impl Size {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn contains(self, point: Point) -> bool {
        point.x >= self.x
            && point.x <= self.x + self.width
            && point.y >= self.y
            && point.y <= self.y + self.height
    }

    pub fn inset(self, amount: f32) -> Self {
        Self::new(
            self.x + amount,
            self.y + amount,
            (self.width - amount * 2.0).max(0.0),
            (self.height - amount * 2.0).max(0.0),
        )
    }

    pub const fn size(self) -> Size {
        Size::new(self.width, self.height)
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

impl Color {
    pub const TRANSPARENT: Self = Self::rgba(0.0, 0.0, 0.0, 0.0);
    pub const BLACK: Self = Self::rgb(0.0, 0.0, 0.0);
    pub const WHITE: Self = Self::rgb(1.0, 1.0, 1.0);

    pub const fn rgb(red: f32, green: f32, blue: f32) -> Self {
        Self {
            red,
            green,
            blue,
            alpha: 1.0,
        }
    }

    pub const fn rgba(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    pub fn mix(self, other: Self, amount: f32) -> Self {
        let t = amount.clamp(0.0, 1.0);
        Self {
            red: self.red + (other.red - self.red) * t,
            green: self.green + (other.green - self.green) * t,
            blue: self.blue + (other.blue - self.blue) * t,
            alpha: self.alpha + (other.alpha - self.alpha) * t,
        }
    }

    pub fn premultiplied(self) -> Self {
        let alpha = self.alpha.clamp(0.0, 1.0);
        Self {
            red: self.red.clamp(0.0, 1.0) * alpha,
            green: self.green.clamp(0.0, 1.0) * alpha,
            blue: self.blue.clamp(0.0, 1.0) * alpha,
            alpha,
        }
    }

    /// Choose black or white text using WCAG relative luminance contrast.
    pub fn contrasting_foreground(self) -> Self {
        let channel = |value: f32| {
            let value = value.clamp(0.0, 1.0);
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        let luminance =
            0.2126 * channel(self.red) + 0.7152 * channel(self.green) + 0.0722 * channel(self.blue);
        let white_contrast = 1.05 / (luminance + 0.05);
        let black_contrast = (luminance + 0.05) / 0.05;
        if black_contrast >= white_contrast {
            Self::BLACK
        } else {
            Self::WHITE
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Primitive {
    RoundedRect {
        rect: Rect,
        radius: f32,
        color: Color,
    },
    Text {
        rect: Rect,
        text: String,
        size: f32,
        color: Color,
        align: TextAlign,
        overflow: TextOverflow,
    },
    Icon {
        rect: Rect,
        icon: Icon,
        color: Color,
    },
    Image {
        rect: Rect,
        image: Image,
        opacity: f32,
        coloring: ImageColoring,
    },
    PushOpacity {
        opacity: f32,
    },
    PopOpacity,
    PushMotion {
        origin: Point,
        motion: Motion,
    },
    PopMotion,
    ShaderEffect {
        effect: ShaderEffect,
        rect: Rect,
        theme: Theme,
    },
    Line {
        start: Point,
        end: Point,
        width: f32,
        color: Color,
    },
    LinearGradientRect {
        rect: Rect,
        radius: f32,
        start: Point,
        end: Point,
        start_color: Color,
        end_color: Color,
    },
    TriangleMesh {
        triangles: Arc<[Point]>,
        color: Color,
    },
    PushClip {
        rect: Rect,
    },
    PopClip,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum TextOverflow {
    #[default]
    Ellipsis,
    Clip,
    /// Scroll overflowing single-line text without changing layout.
    Marquee {
        speed: f32,
        gap: f32,
    },
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum ImageColoring {
    Original,
    Multiply(Color),
    Mask(Color),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub id: u64,
    pub revision: u64,
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}

impl Image {
    pub fn rgba8(
        id: u64,
        revision: u64,
        width: u32,
        height: u32,
        pixels: impl Into<Arc<[u8]>>,
    ) -> anyhow::Result<Self> {
        let pixels = pixels.into();
        let expected = display_list::raster_bytes(width, height)?;
        if width == 0 || height == 0 || pixels.len() != expected {
            anyhow::bail!(
                "RGBA image dimensions require {expected} bytes, received {}",
                pixels.len()
            );
        }
        Ok(Self {
            id,
            revision,
            width,
            height,
            pixels,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub clear: Color,
    pub primitives: Vec<Primitive>,
}

impl Scene {
    pub fn new(clear: Color) -> Self {
        Self {
            clear,
            primitives: Vec::new(),
        }
    }

    pub fn rounded_rect(&mut self, rect: Rect, radius: f32, color: Color) {
        if rect.width > 0.0 && rect.height > 0.0 && color.alpha > 0.0 {
            self.primitives.push(Primitive::RoundedRect {
                rect,
                radius: radius.max(0.0).min(rect.width.min(rect.height) * 0.5),
                color,
            });
        }
    }

    pub fn text(
        &mut self,
        rect: Rect,
        text: impl Into<String>,
        size: f32,
        color: Color,
        align: TextAlign,
    ) {
        self.styled_text(rect, text, size, color, align, TextOverflow::Ellipsis);
    }

    pub fn styled_text(
        &mut self,
        rect: Rect,
        text: impl Into<String>,
        size: f32,
        color: Color,
        align: TextAlign,
        overflow: TextOverflow,
    ) {
        let text = text.into();
        if rect.width > 0.0
            && rect.height > 0.0
            && size > 0.0
            && color.alpha > 0.0
            && !text.is_empty()
        {
            self.primitives.push(Primitive::Text {
                rect,
                text,
                size,
                color,
                align,
                overflow,
            });
        }
    }

    pub fn icon(&mut self, rect: Rect, icon: Icon, color: Color) {
        if rect.width > 0.0 && rect.height > 0.0 && color.alpha > 0.0 {
            self.primitives.push(Primitive::Icon { rect, icon, color });
        }
    }

    pub fn image(&mut self, rect: Rect, image: Image, opacity: f32) {
        self.colored_image(rect, image, opacity, ImageColoring::Original);
    }

    pub fn colored_image(
        &mut self,
        rect: Rect,
        image: Image,
        opacity: f32,
        coloring: ImageColoring,
    ) {
        if rect.width > 0.0 && rect.height > 0.0 && opacity > 0.0 {
            self.primitives.push(Primitive::Image {
                rect,
                image,
                opacity: opacity.clamp(0.0, 1.0),
                coloring,
            });
        }
    }

    pub fn push_opacity(&mut self, opacity: f32) {
        self.primitives.push(Primitive::PushOpacity {
            opacity: opacity.clamp(0.0, 1.0),
        });
    }

    pub fn pop_opacity(&mut self) {
        self.primitives.push(Primitive::PopOpacity);
    }

    pub fn push_motion(&mut self, origin: Point, motion: Motion) {
        self.primitives
            .push(Primitive::PushMotion { origin, motion });
    }

    pub fn has_active_motion(&self, now: Duration, policy: MotionPolicy) -> bool {
        self.primitives.iter().any(|primitive| {
            matches!(primitive, Primitive::PushMotion { motion, .. } if motion.is_active(now, policy))
                || matches!(primitive, Primitive::ShaderEffect { effect, .. } if effect.is_active(policy))
        })
    }

    pub fn pop_motion(&mut self) {
        self.primitives.push(Primitive::PopMotion);
    }

    pub fn shader_effect(&mut self, effect: ShaderEffect, rect: Rect, theme: Theme) {
        if rect.width > 0.0 && rect.height > 0.0 && effect.opacity > 0.0 {
            self.primitives.push(Primitive::ShaderEffect {
                effect,
                rect,
                theme,
            });
        }
    }

    pub fn line(&mut self, start: Point, end: Point, width: f32, color: Color) {
        if width > 0.0 && color.alpha > 0.0 && start != end {
            self.primitives.push(Primitive::Line {
                start,
                end,
                width,
                color,
            });
        }
    }

    pub fn linear_gradient_rect(
        &mut self,
        rect: Rect,
        radius: f32,
        start: Point,
        end: Point,
        start_color: Color,
        end_color: Color,
    ) {
        if rect.width > 0.0
            && rect.height > 0.0
            && start != end
            && (start_color.alpha > 0.0 || end_color.alpha > 0.0)
        {
            self.primitives.push(Primitive::LinearGradientRect {
                rect,
                radius,
                start,
                end,
                start_color,
                end_color,
            });
        }
    }

    pub fn triangle_mesh(&mut self, triangles: Arc<[Point]>, color: Color) {
        if triangles.len() >= 3 && triangles.len().is_multiple_of(3) && color.alpha > 0.0 {
            self.primitives
                .push(Primitive::TriangleMesh { triangles, color });
        }
    }

    pub fn push_clip(&mut self, rect: Rect) {
        self.primitives.push(Primitive::PushClip { rect });
    }

    pub fn pop_clip(&mut self) {
        self.primitives.push(Primitive::PopClip);
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct WidgetId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractionKind {
    Button { hold: Option<Duration> },
    Slider,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitTarget {
    pub id: WidgetId,
    pub bounds: Rect,
    pub kind: InteractionKind,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InteractionMap {
    targets: Vec<HitTarget>,
}

impl InteractionMap {
    pub fn add(&mut self, target: HitTarget) {
        self.targets.push(target);
    }

    pub fn button(&mut self, id: WidgetId, bounds: Rect) {
        self.add(HitTarget {
            id,
            bounds,
            kind: InteractionKind::Button { hold: None },
        });
    }

    pub fn press_and_hold(&mut self, id: WidgetId, bounds: Rect, threshold: Duration) {
        self.add(HitTarget {
            id,
            bounds,
            kind: InteractionKind::Button {
                hold: Some(threshold),
            },
        });
    }

    pub fn slider(&mut self, id: WidgetId, bounds: Rect) {
        self.add(HitTarget {
            id,
            bounds,
            kind: InteractionKind::Slider,
        });
    }

    pub fn target(&self, id: WidgetId) -> Option<HitTarget> {
        self.targets
            .iter()
            .rev()
            .find(|target| target.id == id)
            .copied()
    }

    /// Geometry only; capture and recognition belong to touchbar-core.
    pub fn targets(&self) -> &[HitTarget] {
        &self.targets
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub background: Color,
    pub control: Color,
    pub control_pressed: Color,
    pub accent: Color,
    pub track: Color,
    pub foreground: Color,
    pub muted: Color,
    pub destructive: Color,
    pub corner_radius: f32,
}

impl Theme {
    pub fn light() -> Self {
        Self {
            background: Color::rgb(0.96, 0.96, 0.97),
            control: Color::rgb(0.87, 0.87, 0.89),
            control_pressed: Color::rgb(0.75, 0.75, 0.78),
            foreground: Color::rgb(0.08, 0.08, 0.1),
            muted: Color::rgb(0.4, 0.4, 0.43),
            track: Color::rgb(0.72, 0.72, 0.75),
            ..Self::default()
        }
    }
    pub fn dark() -> Self {
        Self::default()
    }
    pub fn with_accent(self, accent: Color) -> Self {
        Self { accent, ..self }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: Color::rgb(0.005, 0.008, 0.006),
            control: Color::rgb(0.13, 0.13, 0.15),
            control_pressed: Color::rgb(0.24, 0.24, 0.28),
            accent: Color::rgb(0.42, 0.95, 0.25),
            track: Color::rgb(0.22, 0.22, 0.25),
            foreground: Color::WHITE,
            muted: Color::rgb(0.55, 0.55, 0.58),
            destructive: Color::rgb(0.95, 0.25, 0.25),
            corner_radius: 9.0,
        }
    }
}

pub fn button(
    scene: &mut Scene,
    interactions: &mut InteractionMap,
    id: WidgetId,
    bounds: Rect,
    pressed: bool,
    hold: Option<Duration>,
    theme: Theme,
) {
    scene.rounded_rect(
        bounds,
        theme.corner_radius,
        if pressed {
            theme.control_pressed
        } else {
            theme.control
        },
    );
    match hold {
        Some(threshold) => interactions.press_and_hold(id, bounds, threshold),
        None => interactions.button(id, bounds),
    }
}

pub fn slider(
    scene: &mut Scene,
    interactions: &mut InteractionMap,
    id: WidgetId,
    bounds: Rect,
    value: f32,
    theme: Theme,
) {
    let value = value.clamp(0.0, 1.0);
    let track = Rect::new(
        bounds.x,
        bounds.y + bounds.height * 0.4,
        bounds.width,
        bounds.height * 0.2,
    );
    scene.rounded_rect(track, track.height * 0.5, theme.track);
    scene.rounded_rect(
        Rect::new(track.x, track.y, track.width * value, track.height),
        track.height * 0.5,
        theme.accent,
    );
    let knob_radius = bounds.height * 0.24;
    let knob_x = bounds.x + bounds.width * value;
    scene.rounded_rect(
        Rect::new(
            knob_x - knob_radius,
            bounds.y + bounds.height * 0.5 - knob_radius,
            knob_radius * 2.0,
            knob_radius * 2.0,
        ),
        knob_radius,
        Color::WHITE,
    );
    interactions.slider(id, bounds);
}

/// Return equal-width horizontal cells inside a rectangle.
pub fn equal_row(bounds: Rect, count: usize, gap: f32) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let total_gap = gap.max(0.0) * count.saturating_sub(1) as f32;
    let width = ((bounds.width - total_gap) / count as f32).max(0.0);
    (0..count)
        .map(|index| {
            Rect::new(
                bounds.x + index as f32 * (width + gap.max(0.0)),
                bounds.y,
                width,
                bounds.height,
            )
        })
        .collect()
}

/// Measure the dependency-free bootstrap font used by [`Scene::text`].
///
/// The first renderer intentionally supports compact ASCII labels. A future
/// shaper can implement the same measurement boundary for localized text.
pub fn measure_text(text: &str, size: f32) -> Size {
    if text.is_empty() || size <= 0.0 {
        return Size::default();
    }
    let cell = (size / 7.0).max(0.75);
    Size::new(cell * (text.chars().count() as f32 * 6.0 - 1.0), cell * 7.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tween {
    pub from: f32,
    pub to: f32,
    pub started: Duration,
    pub duration: Duration,
}

impl Tween {
    pub fn sample(self, now: Duration) -> f32 {
        if self.duration.is_zero() {
            return self.to;
        }
        let linear = now.saturating_sub(self.started).as_secs_f32() / self.duration.as_secs_f32();
        let t = linear.clamp(0.0, 1.0);
        // Smoothstep has zero velocity at both ends and is sufficient for the
        // first UI slice without imposing a global animation system.
        let eased = t * t * (3.0 - 2.0 * t);
        self.from + (self.to - self.from) * eased
    }

    pub fn finished(self, now: Duration) -> bool {
        now.saturating_sub(self.started) >= self.duration
    }
}

#[cfg(test)]
fn test_fonts() -> cosmic_text::FontSystem {
    let mut db = cosmic_text::fontdb::Database::new();
    db.load_font_data(include_bytes!("../../../assets/fonts/test/InterVariable.ttf").to_vec());
    db.set_sans_serif_family("Inter Variable");
    cosmic_text::FontSystem::new_with_locale_and_db("en-US".into(), db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_premultiply_for_wayland_argb_composition() {
        assert_eq!(
            Color::rgba(0.8, 0.4, 0.2, 0.5).premultiplied(),
            Color::rgba(0.4, 0.2, 0.1, 0.5)
        );
        assert_eq!(Color::TRANSPARENT.premultiplied(), Color::TRANSPARENT);
    }

    #[test]
    fn contrasting_foreground_tracks_light_and_dark_theme_colors() {
        assert_eq!(
            Color::rgb(0.95, 0.9, 0.2).contrasting_foreground(),
            Color::BLACK
        );
        assert_eq!(
            Color::rgb(0.05, 0.08, 0.12).contrasting_foreground(),
            Color::WHITE
        );
    }

    #[test]
    fn text_measurement_and_image_validation_are_deterministic() {
        assert_eq!(measure_text("VOL", 14.0), Size::new(34.0, 14.0));
        assert!(Image::rgba8(1, 1, 2, 2, vec![0; 16]).is_ok());
        assert!(Image::rgba8(1, 1, 2, 2, vec![0; 15]).is_err());
    }

    #[test]
    fn row_layout_is_deterministic() {
        assert_eq!(
            equal_row(Rect::new(0.0, 0.0, 100.0, 20.0), 3, 5.0),
            [
                Rect::new(0.0, 0.0, 30.0, 20.0),
                Rect::new(35.0, 0.0, 30.0, 20.0),
                Rect::new(70.0, 0.0, 30.0, 20.0),
            ]
        );
    }

    #[test]
    fn tween_uses_smooth_endpoints() {
        let tween = Tween {
            from: 10.0,
            to: 20.0,
            started: Duration::from_secs(1),
            duration: Duration::from_secs(2),
        };
        assert_eq!(tween.sample(Duration::from_secs(1)), 10.0);
        assert_eq!(tween.sample(Duration::from_secs(2)), 15.0);
        assert_eq!(tween.sample(Duration::from_secs(3)), 20.0);
        assert!(tween.finished(Duration::from_secs(3)));
    }

    #[test]
    fn motion_samples_at_render_time_and_reduced_motion_settles_immediately() {
        let motion = Motion {
            id: MotionId(3),
            from: VisualTransform {
                translation: Point::new(10.0, -4.0),
                scale: 0.8,
                opacity: 0.0,
            },
            to: VisualTransform::IDENTITY,
            started: Duration::from_secs(1),
            duration: Duration::from_secs(2),
            easing: Easing::Linear,
            playback: MotionPlayback::Once,
        };
        assert_eq!(
            motion.sample(Duration::from_secs(2), MotionPolicy::Full),
            VisualTransform {
                translation: Point::new(5.0, -2.0),
                scale: 0.9,
                opacity: 0.5,
            }
        );
        assert_eq!(
            motion.sample(Duration::from_secs(1), MotionPolicy::Reduced),
            VisualTransform::IDENTITY
        );
        assert_eq!(
            motion.sample(Duration::from_secs(1), MotionPolicy::Disabled),
            VisualTransform::IDENTITY
        );
        assert!(!motion.is_active(Duration::from_secs(3), MotionPolicy::Full));
        assert!(!motion.is_active(Duration::from_secs(2), MotionPolicy::Reduced));

        let repeating = Motion {
            playback: MotionPlayback::Loop,
            started: Duration::ZERO,
            duration: Duration::from_secs(1),
            from: VisualTransform {
                opacity: 0.0,
                ..VisualTransform::IDENTITY
            },
            to: VisualTransform::IDENTITY,
            ..motion
        };
        assert_eq!(
            repeating
                .sample(Duration::from_millis(1_250), MotionPolicy::Full)
                .opacity,
            0.25
        );
        assert!(repeating.is_active(Duration::from_secs(100), MotionPolicy::Full));

        let alternating = Motion {
            playback: MotionPlayback::Alternate,
            ..repeating
        };
        assert_eq!(
            alternating
                .sample(Duration::from_millis(1_250), MotionPolicy::Full)
                .opacity,
            0.75
        );
    }
}
