use std::{collections::BTreeMap, sync::Arc, time::Duration};

use taffy::{
    geometry::{Rect as TaffyRect, Size as TaffySize},
    prelude::{AlignItems, AvailableSpace, FlexDirection, Style, TaffyTree, auto, length},
};

use crate::{
    Color, ContinuousValue, Image, ImageColoring, InteractionMap, MeterStyle, Motion, Point, Rect,
    Scene, ScrubberCellStyle, ScrubberItemState, ShaderEffect, Size, SliderStyle, Theme,
    TinyGraphStyle, WidgetId, button, slider,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextAlign {
    Leading,
    Center,
    Trailing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Icon {
    Volume,
    Muted,
    Play,
    Pause,
    Check,
    ChevronLeft,
    ChevronRight,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ImageFit {
    /// Preserve the complete image and letterbox it inside the assigned bounds.
    #[default]
    Contain,
    /// Preserve aspect ratio while filling and clipping to the assigned bounds.
    Cover,
    /// Fill the assigned bounds without preserving aspect ratio.
    Stretch,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ImageTint {
    /// Preserve the original image colors.
    #[default]
    None,
    /// Multiply the original image colors by a live theme color.
    Multiply(ColorRole),
    /// Treat image alpha as a mask filled by a live theme color.
    Mask(ColorRole),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PressableStyle {
    pub background: Option<ColorRole>,
    pub pressed_background: Option<ColorRole>,
    /// `None` follows the current theme's corner radius.
    pub corner_radius: Option<f32>,
    pub padding: f32,
    pub minimum_size: Size,
    /// Overrides `Foreground` for the complete child subtree. This lets a
    /// retained control derive readable content from its live background.
    pub content_foreground: Option<ColorRole>,
}

impl PressableStyle {
    pub const fn control() -> Self {
        Self {
            background: Some(ColorRole::Control),
            pressed_background: Some(ColorRole::ControlPressed),
            corner_radius: None,
            padding: 8.0,
            minimum_size: Size::new(44.0, 44.0),
            content_foreground: None,
        }
    }

    pub const fn plain() -> Self {
        Self {
            background: None,
            pressed_background: Some(ColorRole::ControlPressed),
            corner_radius: None,
            padding: 0.0,
            minimum_size: Size::new(0.0, 0.0),
            content_foreground: None,
        }
    }

    pub const fn accent() -> Self {
        Self {
            background: Some(ColorRole::Accent),
            pressed_background: Some(ColorRole::Accent),
            corner_radius: None,
            padding: 8.0,
            minimum_size: Size::new(44.0, 44.0),
            content_foreground: Some(ColorRole::OnAccent),
        }
    }
}

impl Default for PressableStyle {
    fn default() -> Self {
        Self::control()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProgressValue {
    Determinate(f32),
    /// A normalized animation phase supplied by the plugin's own scheduler.
    Indeterminate {
        phase: f32,
    },
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Representation {
    Hidden,
    Minimal,
    Compact,
    Full,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResponsiveVariant {
    pub representation: Representation,
    pub minimum_width: f32,
    pub node: Box<Node>,
}

impl ResponsiveVariant {
    pub fn new(representation: Representation, minimum_width: f32, node: Node) -> Self {
        Self {
            representation,
            minimum_width: minimum_width.max(0.0),
            node: Box::new(node),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flex {
    pub minimum: f32,
    pub basis: f32,
    pub maximum: f32,
    pub grow: f32,
    pub shrink: f32,
    pub visibility_priority: i32,
    pub required: bool,
    /// Let measured child content provide the flex basis instead of `basis`.
    pub intrinsic: bool,
}

impl Flex {
    pub fn fixed(width: f32) -> Self {
        let width = width.max(0.0);
        Self {
            minimum: width,
            basis: width,
            maximum: width,
            grow: 0.0,
            shrink: 0.0,
            visibility_priority: 0,
            required: false,
            intrinsic: false,
        }
    }

    pub fn flexible(minimum: f32, basis: f32, maximum: f32) -> Self {
        let minimum = minimum.max(0.0);
        let maximum = maximum.max(minimum);
        Self {
            minimum,
            basis: basis.clamp(minimum, maximum),
            maximum,
            grow: 1.0,
            shrink: 1.0,
            visibility_priority: 0,
            required: false,
            intrinsic: false,
        }
    }

    /// Size from shaped text, image dimensions, or nested child content.
    pub fn content(minimum: f32, maximum: f32) -> Self {
        let minimum = minimum.max(0.0);
        let maximum = maximum.max(minimum);
        Self {
            minimum,
            basis: minimum,
            maximum,
            grow: 0.0,
            shrink: 1.0,
            visibility_priority: 0,
            required: false,
            intrinsic: true,
        }
    }

    pub fn grow(mut self, weight: f32) -> Self {
        self.grow = weight.max(0.0);
        self
    }

    pub fn shrink(mut self, weight: f32) -> Self {
        self.shrink = weight.max(0.0);
        self
    }

    pub fn priority(mut self, priority: i32) -> Self {
        self.visibility_priority = priority;
        self
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlexItem {
    pub flex: Flex,
    pub node: Node,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CrossAxisAlignment {
    Start,
    Center,
    End,
    #[default]
    Stretch,
}

impl FlexItem {
    pub fn new(flex: Flex, node: Node) -> Self {
        Self { flex, node }
    }
}

/// Resolve a horizontal group. Optional rectangles are hidden children.
/// Low-priority, trailing children are hidden first when even minimum widths
/// cannot fit; required children are never hidden.
pub fn resolve_flex_row(bounds: Rect, items: &[Flex], gap: f32) -> Vec<Option<Rect>> {
    let measured = vec![crate::Size::default(); items.len()];
    resolve_taffy_flex(
        bounds,
        items,
        &measured,
        gap,
        0.0,
        FlexDirection::Row,
        CrossAxisAlignment::Stretch,
    )
}

fn minimum_total(items: &[Flex], visible: &[bool], gap: f32) -> f32 {
    let count = visible.iter().filter(|shown| **shown).count();
    items
        .iter()
        .zip(visible)
        .filter(|(_, shown)| **shown)
        .map(|(item, _)| item.minimum)
        .sum::<f32>()
        + gap * count.saturating_sub(1) as f32
}

fn resolve_taffy_flex(
    bounds: Rect,
    items: &[Flex],
    measured: &[crate::Size],
    gap: f32,
    padding: f32,
    direction: FlexDirection,
    alignment: CrossAxisAlignment,
) -> Vec<Option<Rect>> {
    if items.is_empty() {
        return Vec::new();
    }
    let gap = gap.max(0.0);
    let padding = padding.max(0.0);
    let row = matches!(direction, FlexDirection::Row | FlexDirection::RowReverse);
    let primary = if row { bounds.width } else { bounds.height };
    let inner_primary = (primary - padding * 2.0).max(0.0);
    let mut visible = vec![true; items.len()];
    while minimum_total(items, &visible, gap) > inner_primary {
        let candidate = items
            .iter()
            .enumerate()
            .filter(|(index, item)| visible[*index] && !item.required)
            .min_by_key(|(index, item)| (item.visibility_priority, std::cmp::Reverse(*index)))
            .map(|(index, _)| index);
        let Some(index) = candidate else {
            break;
        };
        visible[index] = false;
    }

    let mut tree = TaffyTree::<crate::Size>::new();
    tree.disable_rounding();
    let mut child_ids = vec![None; items.len()];
    let mut children = Vec::new();
    for (index, item) in items
        .iter()
        .enumerate()
        .filter(|(index, _)| visible[*index])
    {
        let mut style = Style {
            flex_basis: if item.intrinsic {
                auto()
            } else {
                length(item.basis)
            },
            flex_grow: item.grow,
            flex_shrink: item.shrink,
            ..Style::default()
        };
        if row {
            style.min_size.width = length(item.minimum);
            style.max_size.width = if item.maximum.is_finite() {
                length(item.maximum)
            } else {
                auto()
            };
        } else {
            style.min_size.height = length(item.minimum);
            style.max_size.height = if item.maximum.is_finite() {
                length(item.maximum)
            } else {
                auto()
            };
        }
        let id = tree
            .new_leaf_with_context(style, measured.get(index).copied().unwrap_or_default())
            .expect("a small in-memory Taffy tree is valid");
        child_ids[index] = Some(id);
        children.push(id);
    }
    let root_style = Style {
        display: taffy::style::Display::Flex,
        flex_direction: direction,
        size: TaffySize {
            width: length(bounds.width.max(0.0)),
            height: length(bounds.height.max(0.0)),
        },
        padding: TaffyRect {
            left: length(padding),
            right: length(padding),
            top: length(padding),
            bottom: length(padding),
        },
        gap: if row {
            TaffySize {
                width: length(gap),
                height: length(0.0),
            }
        } else {
            TaffySize {
                width: length(0.0),
                height: length(gap),
            }
        },
        align_items: Some(match alignment {
            CrossAxisAlignment::Start => AlignItems::START,
            CrossAxisAlignment::Center => AlignItems::CENTER,
            CrossAxisAlignment::End => AlignItems::END,
            CrossAxisAlignment::Stretch => AlignItems::STRETCH,
        }),
        ..Style::default()
    };
    let root = tree
        .new_with_children(root_style, &children)
        .expect("a small in-memory Taffy tree is valid");
    tree.compute_layout_with_measure(
        root,
        TaffySize {
            width: AvailableSpace::Definite(bounds.width.max(0.0)),
            height: AvailableSpace::Definite(bounds.height.max(0.0)),
        },
        |inputs, _node, context, style| {
            taffy::compute_leaf_layout(
                inputs,
                style,
                |_, _| 0.0,
                |known, available| {
                    let measured = context.as_deref().copied().unwrap_or_default();
                    TaffySize {
                        width: known.width.unwrap_or_else(|| {
                            constrain_measurement(measured.width, available.width)
                        }),
                        height: known.height.unwrap_or_else(|| {
                            constrain_measurement(measured.height, available.height)
                        }),
                    }
                },
            )
        },
    )
    .expect("a small in-memory Taffy layout is valid");

    child_ids
        .into_iter()
        .map(|id| {
            id.map(|id| {
                let layout = tree.layout(id).expect("computed child layout exists");
                Rect::new(
                    bounds.x + layout.location.x,
                    bounds.y + layout.location.y,
                    layout.size.width,
                    layout.size.height,
                )
            })
        })
        .collect()
}

fn constrain_measurement(value: f32, available: AvailableSpace) -> f32 {
    match available {
        AvailableSpace::Definite(limit) => value.min(limit),
        AvailableSpace::MinContent => 0.0,
        AvailableSpace::MaxContent => value,
    }
    .max(0.0)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticRole {
    Group,
    Label,
    Image,
    Button,
    Toggle,
    Slider,
    Progress,
    Canvas,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemanticNode {
    pub id: Option<WidgetId>,
    pub role: SemanticRole,
    pub label: String,
    pub value: Option<String>,
    pub hint: Option<String>,
    pub bounds: Rect,
    pub enabled: bool,
    pub selected: bool,
    pub children: Vec<SemanticNode>,
}

/// Supplies intrinsic leaf measurements without coupling layout to a renderer.
/// CpuPainter implements this with the same Cosmic Text cache it uses
/// for painting; tests and renderer-independent callers use a deterministic
/// fallback through [`RetainedUi::resolve`].
pub trait LayoutMeasurer {
    fn measure_text(&self, text: &str, size: f32, maximum: crate::Size) -> crate::Size;
}

struct ApproximateMeasurer;

impl LayoutMeasurer for ApproximateMeasurer {
    fn measure_text(&self, text: &str, size: f32, maximum: crate::Size) -> crate::Size {
        let measured = crate::measure_text(text, size);
        crate::Size::new(
            measured.width.min(maximum.width),
            measured.height.min(maximum.height),
        )
    }
}

impl SemanticNode {
    fn leaf(
        id: Option<WidgetId>,
        role: SemanticRole,
        label: impl Into<String>,
        bounds: Rect,
    ) -> Self {
        Self {
            id,
            role,
            label: label.into(),
            value: None,
            hint: None,
            bounds,
            enabled: true,
            selected: false,
            children: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Empty,
    Layer(Vec<Node>),
    /// Place a child in local coordinates. The enclosing node still provides
    /// clipping, making this useful for virtualized and scrolling content.
    Positioned {
        frame: Rect,
        child: Box<Node>,
    },
    Row {
        gap: f32,
        padding: f32,
        align: CrossAxisAlignment,
        children: Vec<FlexItem>,
    },
    Column {
        gap: f32,
        padding: f32,
        align: CrossAxisAlignment,
        children: Vec<FlexItem>,
    },
    Panel {
        radius: f32,
        color: ColorRole,
        padding: f32,
        child: Box<Node>,
    },
    Label {
        text: String,
        size: f32,
        color: ColorRole,
        align: TextAlign,
        overflow: crate::TextOverflow,
        measurement: TextMeasurement,
    },
    Icon {
        icon: Icon,
        color: ColorRole,
        label: String,
    },
    Image {
        image: Image,
        opacity: f32,
        fit: ImageFit,
        tint: ImageTint,
        label: String,
    },
    /// Make any visual subtree interactive while preserving its own layout
    /// and semantic descendants.
    Pressable {
        id: WidgetId,
        label: String,
        pressed: bool,
        hold: Option<Duration>,
        selected: Option<bool>,
        style: PressableStyle,
        child: Box<Node>,
    },
    Button {
        id: WidgetId,
        label: String,
        icon: Option<Icon>,
        pressed: bool,
        hold: Option<Duration>,
    },
    Toggle {
        id: WidgetId,
        label: String,
        icon: Option<Icon>,
        selected: bool,
        pressed: bool,
    },
    Slider {
        id: WidgetId,
        label: String,
        value: f32,
    },
    StyledSlider {
        id: WidgetId,
        label: String,
        value: ContinuousValue,
        style: SliderStyle,
    },
    Meter {
        label: String,
        value: ContinuousValue,
        peak: Option<f32>,
        style: MeterStyle,
    },
    TinyGraph {
        label: String,
        samples: Arc<[f32]>,
        minimum: f32,
        maximum: f32,
        style: TinyGraphStyle,
    },
    Progress {
        label: String,
        value: ProgressValue,
        track: ColorRole,
        fill: ColorRole,
    },
    /// Apply visual opacity without changing layout, interaction, or semantic
    /// identity. Nested opacity values multiply.
    Opacity {
        opacity: f32,
        child: Box<Node>,
    },
    /// Animate paint-time translation, uniform scale, and opacity around the
    /// node's center without invalidating layout or hit geometry.
    Motion {
        motion: Motion,
        child: Box<Node>,
    },
    /// A capability-free, host-rendered vector scene for sandboxed component
    /// plugins. Commands use local view-box coordinates and are clipped to the
    /// resolved node bounds.
    Canvas {
        label: String,
        viewbox: Size,
        commands: Arc<[CanvasCommand]>,
    },
    /// A statically validated, bounded fragment effect. It fills its assigned
    /// layout bounds and receives live semantic theme uniforms.
    ShaderEffect {
        label: String,
        preferred_size: Size,
        effect: ShaderEffect,
    },
    Responsive {
        id: WidgetId,
        variants: Vec<ResponsiveVariant>,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum TextMeasurement {
    #[default]
    Content,
    /// Reserve the width of this sample while displaying the label text.
    Reserve(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorRole {
    Background,
    Control,
    ControlPressed,
    Track,
    Foreground,
    Muted,
    Accent,
    Destructive,
    /// Automatically chooses black or white for readable accent content.
    OnAccent,
    /// Automatically chooses black or white for destructive content.
    OnDestructive,
}

/// A Canvas2D paint can either follow a live semantic theme role or preserve a
/// literal data color. Alpha is straight in this public model and is
/// premultiplied only by the GLES renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CanvasColor {
    Role(ColorRole),
    Rgba(Color),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasPaint {
    pub color: CanvasColor,
    pub opacity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasLinearGradient {
    pub start: Point,
    pub end: Point,
    pub start_color: CanvasPaint,
    pub end_color: CanvasPaint,
}

impl CanvasPaint {
    pub const fn role(role: ColorRole) -> Self {
        Self {
            color: CanvasColor::Role(role),
            opacity: 1.0,
        }
    }

    pub const fn rgba(color: Color) -> Self {
        Self {
            color: CanvasColor::Rgba(color),
            opacity: 1.0,
        }
    }

    pub const fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    pub fn resolve(self, theme: Theme) -> Color {
        let mut color = match self.color {
            CanvasColor::Role(role) => role.resolve(theme),
            CanvasColor::Rgba(color) => color,
        };
        color.alpha *= self.opacity.clamp(0.0, 1.0);
        color
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CanvasCommand {
    FillRect {
        rect: Rect,
        radius: f32,
        paint: CanvasPaint,
    },
    Line {
        start: Point,
        end: Point,
        width: f32,
        paint: CanvasPaint,
    },
    Polyline {
        points: Arc<[Point]>,
        width: f32,
        paint: CanvasPaint,
    },
    FillCircle {
        center: Point,
        radius: f32,
        paint: CanvasPaint,
    },
    Text {
        rect: Rect,
        text: String,
        size: f32,
        paint: CanvasPaint,
        align: TextAlign,
    },
    FillLinearGradientRect {
        rect: Rect,
        radius: f32,
        gradient: CanvasLinearGradient,
    },
    StrokePath {
        subpaths: Arc<[Arc<[Point]>]>,
        width: f32,
        paint: CanvasPaint,
    },
    FillPath {
        triangles: Arc<[Point]>,
        paint: CanvasPaint,
    },
}

impl Node {
    pub fn positioned(frame: Rect, child: Node) -> Self {
        Self::Positioned {
            frame,
            child: Box::new(child),
        }
    }

    pub fn row(gap: f32, padding: f32, children: Vec<FlexItem>) -> Self {
        Self::Row {
            gap,
            padding,
            align: CrossAxisAlignment::Stretch,
            children,
        }
    }

    pub fn row_aligned(
        gap: f32,
        padding: f32,
        align: CrossAxisAlignment,
        children: Vec<FlexItem>,
    ) -> Self {
        Self::Row {
            gap,
            padding,
            align,
            children,
        }
    }

    pub fn column(gap: f32, padding: f32, children: Vec<FlexItem>) -> Self {
        Self::Column {
            gap,
            padding,
            align: CrossAxisAlignment::Stretch,
            children,
        }
    }

    pub fn column_aligned(
        gap: f32,
        padding: f32,
        align: CrossAxisAlignment,
        children: Vec<FlexItem>,
    ) -> Self {
        Self::Column {
            gap,
            padding,
            align,
            children,
        }
    }

    pub fn label(text: impl Into<String>, size: f32) -> Self {
        Self::Label {
            text: text.into(),
            size,
            color: ColorRole::Foreground,
            align: TextAlign::Center,
            overflow: crate::TextOverflow::Ellipsis,
            measurement: TextMeasurement::Content,
        }
    }

    pub fn marquee_label(text: impl Into<String>, size: f32, speed: f32, gap: f32) -> Self {
        Self::Label {
            text: text.into(),
            size,
            color: ColorRole::Foreground,
            align: TextAlign::Leading,
            overflow: crate::TextOverflow::Marquee { speed, gap },
            measurement: TextMeasurement::Content,
        }
    }

    pub fn stable_label(text: impl Into<String>, reserve: impl Into<String>, size: f32) -> Self {
        Self::Label {
            text: text.into(),
            size,
            color: ColorRole::Foreground,
            align: TextAlign::Center,
            overflow: crate::TextOverflow::Ellipsis,
            measurement: TextMeasurement::Reserve(reserve.into()),
        }
    }

    pub fn button(
        id: WidgetId,
        label: impl Into<String>,
        icon: Option<Icon>,
        pressed: bool,
    ) -> Self {
        Self::Button {
            id,
            label: label.into(),
            icon,
            pressed,
            hold: None,
        }
    }

    pub fn image(image: Image, label: impl Into<String>) -> Self {
        Self::Image {
            image,
            opacity: 1.0,
            fit: ImageFit::Contain,
            tint: ImageTint::None,
            label: label.into(),
        }
    }

    pub fn pressable(id: WidgetId, label: impl Into<String>, pressed: bool, child: Node) -> Self {
        Self::Pressable {
            id,
            label: label.into(),
            pressed,
            hold: None,
            selected: None,
            style: PressableStyle::default(),
            child: Box::new(child),
        }
    }

    pub fn progress(label: impl Into<String>, value: f32) -> Self {
        Self::Progress {
            label: label.into(),
            value: ProgressValue::Determinate(value),
            track: ColorRole::Track,
            fill: ColorRole::Accent,
        }
    }

    pub fn opacity(opacity: f32, child: Node) -> Self {
        Self::Opacity {
            opacity,
            child: Box::new(child),
        }
    }

    pub fn motion(motion: Motion, child: Node) -> Self {
        Self::Motion {
            motion,
            child: Box::new(child),
        }
    }

    pub fn canvas(
        label: impl Into<String>,
        viewbox: Size,
        commands: impl Into<Arc<[CanvasCommand]>>,
    ) -> Self {
        Self::Canvas {
            label: label.into(),
            viewbox,
            commands: commands.into(),
        }
    }

    pub fn shader_effect(
        label: impl Into<String>,
        preferred_size: Size,
        effect: ShaderEffect,
    ) -> Self {
        Self::ShaderEffect {
            label: label.into(),
            preferred_size,
            effect,
        }
    }

    pub fn slider(id: WidgetId, label: impl Into<String>, value: f32) -> Self {
        Self::Slider {
            id,
            label: label.into(),
            value,
        }
    }

    pub fn styled_slider(
        id: WidgetId,
        label: impl Into<String>,
        value: ContinuousValue,
        style: SliderStyle,
    ) -> Self {
        Self::StyledSlider {
            id,
            label: label.into(),
            value,
            style,
        }
    }

    pub fn meter(
        label: impl Into<String>,
        value: ContinuousValue,
        peak: Option<f32>,
        style: MeterStyle,
    ) -> Self {
        Self::Meter {
            label: label.into(),
            value,
            peak,
            style,
        }
    }

    pub fn tiny_graph(
        label: impl Into<String>,
        samples: impl Into<Arc<[f32]>>,
        minimum: f32,
        maximum: f32,
        style: TinyGraphStyle,
    ) -> Self {
        Self::TinyGraph {
            label: label.into(),
            samples: samples.into(),
            minimum,
            maximum,
            style,
        }
    }

    pub fn scrubber_cell(
        text: impl Into<String>,
        state: ScrubberItemState,
        style: ScrubberCellStyle,
    ) -> Self {
        let background = if state.selected {
            style.selected_background
        } else if state.highlighted {
            style.highlighted_background
        } else {
            style.background
        };
        let foreground = if state.selected {
            style.selected_foreground
        } else {
            style.foreground
        };
        Self::Panel {
            radius: style.corner_radius,
            color: background,
            padding: 4.0,
            child: Box::new(Self::Label {
                text: text.into(),
                size: 11.0,
                color: foreground,
                align: TextAlign::Center,
                overflow: crate::TextOverflow::Ellipsis,
                measurement: TextMeasurement::Content,
            }),
        }
    }

    pub fn responsive(id: WidgetId, variants: Vec<ResponsiveVariant>) -> Self {
        Self::Responsive { id, variants }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InspectorSnapshot {
    pub revision: u64,
    pub representations: BTreeMap<WidgetId, Representation>,
    pub semantics: SemanticNode,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedUi {
    pub scene: Scene,
    pub interactions: InteractionMap,
    pub inspector: InspectorSnapshot,
}

#[derive(Clone, Debug)]
pub struct RetainedUi {
    root: Node,
    revision: u64,
    dirty: bool,
    visible: bool,
    last_inspector: Option<InspectorSnapshot>,
}

impl RetainedUi {
    pub fn new(root: Node) -> Self {
        Self {
            root,
            revision: 1,
            dirty: true,
            visible: true,
            last_inspector: None,
        }
    }

    pub fn update(&mut self, update: impl FnOnce(&mut Node)) {
        update(&mut self.root);
        self.revision = self.revision.wrapping_add(1).max(1);
        self.dirty = true;
    }

    pub fn replace(&mut self, root: Node) {
        self.root = root;
        self.revision = self.revision.wrapping_add(1).max(1);
        self.dirty = true;
    }

    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    pub fn set_visible(&mut self, visible: bool) {
        if visible && !self.visible {
            self.dirty = true;
        }
        self.visible = visible;
    }

    pub fn needs_render(&self) -> bool {
        self.visible && self.dirty
    }

    pub fn inspector(&self) -> Option<&InspectorSnapshot> {
        self.last_inspector.as_ref()
    }

    pub fn resolve(&mut self, bounds: Rect, theme: Theme) -> ResolvedUi {
        self.resolve_with_measurer(bounds, theme, &ApproximateMeasurer)
    }

    /// Resolve using renderer-quality intrinsic measurements. Geometry,
    /// painting, hit targets, and semantics are produced from this one pass.
    pub fn resolve_with_measurer(
        &mut self,
        bounds: Rect,
        theme: Theme,
        measurer: &impl LayoutMeasurer,
    ) -> ResolvedUi {
        let mut output = ResolveOutput {
            scene: Scene::new(Color::TRANSPARENT),
            interactions: InteractionMap::default(),
            representations: BTreeMap::new(),
        };
        let semantics = resolve_node(&self.root, bounds, theme, measurer, &mut output)
            .unwrap_or_else(|| {
                SemanticNode::leaf(None, SemanticRole::Group, "Touch Bar plugin", bounds)
            });
        let inspector = InspectorSnapshot {
            revision: self.revision,
            representations: output.representations,
            semantics,
        };
        self.last_inspector = Some(inspector.clone());
        self.dirty = false;
        ResolvedUi {
            scene: output.scene,
            interactions: output.interactions,
            inspector,
        }
    }
}

struct ResolveOutput {
    scene: Scene,
    interactions: InteractionMap,
    representations: BTreeMap<WidgetId, Representation>,
}

fn resolve_node(
    node: &Node,
    bounds: Rect,
    theme: Theme,
    measurer: &impl LayoutMeasurer,
    output: &mut ResolveOutput,
) -> Option<SemanticNode> {
    match node {
        Node::Empty => None,
        Node::Layer(children) => {
            output.scene.push_clip(bounds);
            let semantic_children = children
                .iter()
                .filter_map(|child| resolve_node(child, bounds, theme, measurer, output))
                .collect();
            output.scene.pop_clip();
            Some(group_semantics(bounds, semantic_children))
        }
        Node::Positioned { frame, child } => resolve_node(
            child,
            Rect::new(
                bounds.x + frame.x,
                bounds.y + frame.y,
                frame.width,
                frame.height,
            ),
            theme,
            measurer,
            output,
        ),
        Node::Row {
            gap,
            padding,
            align,
            children,
        } => {
            let flexes: Vec<_> = children.iter().map(|child| child.flex).collect();
            let measured = children
                .iter()
                .map(|child| intrinsic_size(&child.node, bounds.size(), theme, measurer))
                .collect::<Vec<_>>();
            let placements = resolve_taffy_flex(
                bounds,
                &flexes,
                &measured,
                *gap,
                *padding,
                FlexDirection::Row,
                *align,
            );
            output.scene.push_clip(bounds);
            let semantic_children = children
                .iter()
                .zip(placements)
                .filter_map(|(child, placement)| {
                    placement
                        .and_then(|rect| resolve_node(&child.node, rect, theme, measurer, output))
                })
                .collect();
            output.scene.pop_clip();
            Some(group_semantics(bounds, semantic_children))
        }
        Node::Column {
            gap,
            padding,
            align,
            children,
        } => {
            let flexes: Vec<_> = children.iter().map(|child| child.flex).collect();
            let measured = children
                .iter()
                .map(|child| intrinsic_size(&child.node, bounds.size(), theme, measurer))
                .collect::<Vec<_>>();
            let placements = resolve_taffy_flex(
                bounds,
                &flexes,
                &measured,
                *gap,
                *padding,
                FlexDirection::Column,
                *align,
            );
            output.scene.push_clip(bounds);
            let semantic_children = children
                .iter()
                .zip(placements)
                .filter_map(|(child, placement)| {
                    placement
                        .and_then(|rect| resolve_node(&child.node, rect, theme, measurer, output))
                })
                .collect();
            output.scene.pop_clip();
            Some(group_semantics(bounds, semantic_children))
        }
        Node::Panel {
            radius,
            color,
            padding,
            child,
        } => {
            output
                .scene
                .rounded_rect(bounds, *radius, color.resolve(theme));
            resolve_node(child, bounds.inset(*padding), theme, measurer, output)
        }
        Node::Label {
            text,
            size,
            color,
            align,
            overflow,
            measurement: _,
        } => {
            output.scene.push_clip(bounds);
            output.scene.styled_text(
                bounds,
                text.clone(),
                *size,
                color.resolve(theme),
                *align,
                *overflow,
            );
            output.scene.pop_clip();
            Some(SemanticNode::leaf(None, SemanticRole::Label, text, bounds))
        }
        Node::Icon { icon, color, label } => {
            output.scene.icon(bounds, *icon, color.resolve(theme));
            Some(SemanticNode::leaf(None, SemanticRole::Image, label, bounds))
        }
        Node::Image {
            image,
            opacity,
            fit,
            tint,
            label,
        } => {
            let image_bounds = fit_image(bounds, image, *fit);
            let coloring = match tint {
                ImageTint::None => ImageColoring::Original,
                ImageTint::Multiply(role) => ImageColoring::Multiply(role.resolve(theme)),
                ImageTint::Mask(role) => ImageColoring::Mask(role.resolve(theme)),
            };
            if *fit == ImageFit::Cover {
                output.scene.push_clip(bounds);
            }
            output
                .scene
                .colored_image(image_bounds, image.clone(), *opacity, coloring);
            if *fit == ImageFit::Cover {
                output.scene.pop_clip();
            }
            Some(SemanticNode::leaf(None, SemanticRole::Image, label, bounds))
        }
        Node::Pressable {
            id,
            label,
            pressed,
            hold,
            selected,
            style,
            child,
        } => {
            let background = if *pressed {
                style.pressed_background.or(style.background)
            } else {
                style.background
            };
            if let Some(background) = background {
                output.scene.rounded_rect(
                    bounds,
                    style.corner_radius.unwrap_or(theme.corner_radius),
                    background.resolve(theme),
                );
            }
            match hold {
                Some(threshold) => output.interactions.press_and_hold(*id, bounds, *threshold),
                None => output.interactions.button(*id, bounds),
            }
            let content_theme = style.content_foreground.map_or(theme, |role| Theme {
                foreground: role.resolve(theme),
                ..theme
            });
            let child_semantics = resolve_node(
                child,
                bounds.inset(style.padding.max(0.0)),
                content_theme,
                measurer,
                output,
            )
            .into_iter()
            .collect();
            let role = if selected.is_some() {
                SemanticRole::Toggle
            } else {
                SemanticRole::Button
            };
            let mut semantic = SemanticNode::leaf(Some(*id), role, label, bounds);
            semantic.selected = selected.unwrap_or(false);
            semantic.children = child_semantics;
            Some(semantic)
        }
        Node::Button {
            id,
            label,
            icon,
            pressed,
            hold,
        } => {
            button(
                &mut output.scene,
                &mut output.interactions,
                *id,
                bounds,
                *pressed,
                *hold,
                theme,
            );
            draw_control_content(&mut output.scene, bounds, icon, label, theme.foreground);
            Some(SemanticNode::leaf(
                Some(*id),
                SemanticRole::Button,
                label,
                bounds,
            ))
        }
        Node::Toggle {
            id,
            label,
            icon,
            selected,
            pressed,
        } => {
            let mut local_theme = theme;
            if *selected {
                local_theme.control = theme.accent;
                local_theme.control_pressed = theme.accent.mix(theme.foreground, 0.2);
                local_theme.accent = Color::TRANSPARENT;
                local_theme.foreground = theme.accent.contrasting_foreground();
            }
            button(
                &mut output.scene,
                &mut output.interactions,
                *id,
                bounds,
                *pressed,
                None,
                local_theme,
            );
            draw_control_content(
                &mut output.scene,
                bounds,
                icon,
                label,
                local_theme.foreground,
            );
            let mut semantic = SemanticNode::leaf(Some(*id), SemanticRole::Toggle, label, bounds);
            semantic.selected = *selected;
            Some(semantic)
        }
        Node::Slider { id, label, value } => {
            slider(
                &mut output.scene,
                &mut output.interactions,
                *id,
                bounds.inset(4.0),
                *value,
                theme,
            );
            let mut semantic = SemanticNode::leaf(Some(*id), SemanticRole::Slider, label, bounds);
            semantic.value = Some(format!("{:.0}%", value.clamp(0.0, 1.0) * 100.0));
            semantic.hint = Some("Slide horizontally to adjust".into());
            Some(semantic)
        }
        Node::StyledSlider {
            id,
            label,
            value,
            style,
        } => {
            draw_styled_slider(
                &mut output.scene,
                &mut output.interactions,
                *id,
                bounds,
                *value,
                *style,
                theme,
            );
            let mut semantic = SemanticNode::leaf(Some(*id), SemanticRole::Slider, label, bounds);
            semantic.value = Some(format_continuous_value(*value));
            semantic.hint = Some("Slide horizontally to adjust".into());
            Some(semantic)
        }
        Node::Meter {
            label,
            value,
            peak,
            style,
        } => {
            draw_meter(&mut output.scene, bounds, *value, *peak, *style, theme);
            let mut semantic = SemanticNode::leaf(None, SemanticRole::Progress, label, bounds);
            semantic.value = Some(format_continuous_value(*value));
            Some(semantic)
        }
        Node::TinyGraph {
            label,
            samples,
            minimum,
            maximum,
            style,
        } => {
            draw_tiny_graph(
                &mut output.scene,
                bounds,
                samples,
                *minimum,
                *maximum,
                *style,
                theme,
            );
            Some(SemanticNode::leaf(
                None,
                SemanticRole::Canvas,
                label,
                bounds,
            ))
        }
        Node::Progress {
            label,
            value,
            track,
            fill,
        } => {
            draw_progress(
                &mut output.scene,
                bounds,
                *value,
                track.resolve(theme),
                fill.resolve(theme),
            );
            let mut semantic = SemanticNode::leaf(None, SemanticRole::Progress, label, bounds);
            semantic.value = Some(match value {
                ProgressValue::Determinate(value) => {
                    format!("{:.0}%", value.clamp(0.0, 1.0) * 100.0)
                }
                ProgressValue::Indeterminate { .. } => "In progress".into(),
            });
            Some(semantic)
        }
        Node::Opacity { opacity, child } => {
            output.scene.push_opacity(*opacity);
            let semantic = resolve_node(child, bounds, theme, measurer, output);
            output.scene.pop_opacity();
            semantic
        }
        Node::Motion { motion, child } => {
            output.scene.push_motion(
                Point::new(
                    bounds.x + bounds.width * 0.5,
                    bounds.y + bounds.height * 0.5,
                ),
                *motion,
            );
            let semantic = resolve_node(child, bounds, theme, measurer, output);
            output.scene.pop_motion();
            semantic
        }
        Node::Canvas {
            label,
            viewbox,
            commands,
        } => {
            draw_canvas(&mut output.scene, bounds, *viewbox, commands, theme);
            Some(SemanticNode::leaf(
                None,
                SemanticRole::Canvas,
                label,
                bounds,
            ))
        }
        Node::ShaderEffect {
            label,
            preferred_size: _,
            effect,
        } => {
            output.scene.shader_effect(effect.clone(), bounds, theme);
            Some(SemanticNode::leaf(
                None,
                SemanticRole::Canvas,
                label,
                bounds,
            ))
        }
        Node::Responsive { id, variants } => {
            let chosen = variants
                .iter()
                .filter(|variant| {
                    variant.representation != Representation::Hidden
                        && bounds.width >= variant.minimum_width
                })
                .max_by_key(|variant| variant.representation);
            let representation = chosen
                .map(|variant| variant.representation)
                .unwrap_or(Representation::Hidden);
            output.representations.insert(*id, representation);
            chosen.and_then(|variant| resolve_node(&variant.node, bounds, theme, measurer, output))
        }
    }
}

fn intrinsic_size(
    node: &Node,
    maximum: crate::Size,
    theme: Theme,
    measurer: &impl LayoutMeasurer,
) -> crate::Size {
    let maximum = crate::Size::new(maximum.width.max(0.0), maximum.height.max(0.0));
    let clamp = |size: crate::Size| {
        crate::Size::new(
            size.width.min(maximum.width),
            size.height.min(maximum.height),
        )
    };
    match node {
        Node::Empty => crate::Size::default(),
        Node::Layer(children) => clamp(children.iter().fold(
            crate::Size::default(),
            |measured, child| {
                let child = intrinsic_size(child, maximum, theme, measurer);
                crate::Size::new(
                    measured.width.max(child.width),
                    measured.height.max(child.height),
                )
            },
        )),
        Node::Positioned { frame, .. } => clamp(crate::Size::new(
            frame.x.max(0.0) + frame.width.max(0.0),
            frame.y.max(0.0) + frame.height.max(0.0),
        )),
        Node::Row {
            gap,
            padding,
            children,
            ..
        } => intrinsic_flex_size(children, *gap, *padding, true, maximum, theme, measurer),
        Node::Column {
            gap,
            padding,
            children,
            ..
        } => intrinsic_flex_size(children, *gap, *padding, false, maximum, theme, measurer),
        Node::Panel { padding, child, .. } => {
            let inset = *padding * 2.0;
            let inner_maximum = crate::Size::new(
                (maximum.width - inset).max(0.0),
                (maximum.height - inset).max(0.0),
            );
            let child = intrinsic_size(child, inner_maximum, theme, measurer);
            clamp(crate::Size::new(child.width + inset, child.height + inset))
        }
        Node::Label {
            text,
            size,
            measurement,
            ..
        } => {
            let sample = match measurement {
                TextMeasurement::Content => text,
                TextMeasurement::Reserve(sample) => sample,
            };
            measurer.measure_text(sample, *size, maximum)
        }
        Node::Icon { .. } => clamp(crate::Size::new(28.0, 28.0)),
        Node::Image { image, .. } => {
            clamp(crate::Size::new(image.width as f32, image.height as f32))
        }
        Node::Pressable { style, child, .. } => {
            let inset = style.padding.max(0.0) * 2.0;
            let inner_maximum = crate::Size::new(
                (maximum.width - inset).max(0.0),
                (maximum.height - inset).max(0.0),
            );
            let child = intrinsic_size(child, inner_maximum, theme, measurer);
            clamp(crate::Size::new(
                (child.width + inset).max(style.minimum_size.width),
                (child.height + inset).max(style.minimum_size.height),
            ))
        }
        Node::Button { label, icon, .. } | Node::Toggle { label, icon, .. } => {
            let text = measurer.measure_text(label, 13.0, maximum);
            let icon_width = if icon.is_some() { 27.0 } else { 0.0 };
            clamp(crate::Size::new(text.width + icon_width + 16.0, 44.0))
        }
        Node::Slider { .. } | Node::StyledSlider { .. } => clamp(crate::Size::new(120.0, 44.0)),
        Node::Meter { .. } => clamp(crate::Size::new(120.0, 12.0)),
        Node::TinyGraph { .. } => clamp(crate::Size::new(120.0, 28.0)),
        Node::Progress { .. } => clamp(crate::Size::new(120.0, 8.0)),
        Node::Opacity { child, .. } | Node::Motion { child, .. } => {
            intrinsic_size(child, maximum, theme, measurer)
        }
        Node::Canvas { viewbox, .. } => clamp(*viewbox),
        Node::ShaderEffect { preferred_size, .. } => clamp(*preferred_size),
        Node::Responsive { variants, .. } => variants
            .iter()
            .filter(|variant| {
                variant.representation != Representation::Hidden
                    && maximum.width >= variant.minimum_width
            })
            .max_by_key(|variant| variant.representation)
            .map_or_else(crate::Size::default, |variant| {
                intrinsic_size(&variant.node, maximum, theme, measurer)
            }),
    }
}

fn draw_canvas(
    scene: &mut Scene,
    bounds: Rect,
    viewbox: Size,
    commands: &[CanvasCommand],
    theme: Theme,
) {
    if bounds.width <= 0.0 || bounds.height <= 0.0 {
        return;
    }
    let scale_x = bounds.width / viewbox.width.max(f32::EPSILON);
    let scale_y = bounds.height / viewbox.height.max(f32::EPSILON);
    let stroke_scale = scale_x.abs().min(scale_y.abs());
    let point =
        |point: Point| Point::new(bounds.x + point.x * scale_x, bounds.y + point.y * scale_y);
    let rect = |rect: Rect| {
        Rect::new(
            bounds.x + rect.x * scale_x,
            bounds.y + rect.y * scale_y,
            rect.width * scale_x,
            rect.height * scale_y,
        )
    };

    scene.push_clip(bounds);
    for command in commands {
        match command {
            CanvasCommand::FillRect {
                rect: local,
                radius,
                paint,
            } => scene.rounded_rect(rect(*local), *radius * stroke_scale, paint.resolve(theme)),
            CanvasCommand::Line {
                start,
                end,
                width,
                paint,
            } => scene.line(
                point(*start),
                point(*end),
                *width * stroke_scale,
                paint.resolve(theme),
            ),
            CanvasCommand::Polyline {
                points,
                width,
                paint,
            } => {
                let color = paint.resolve(theme);
                for pair in points.windows(2) {
                    scene.line(point(pair[0]), point(pair[1]), *width * stroke_scale, color);
                }
            }
            CanvasCommand::FillCircle {
                center,
                radius,
                paint,
            } => {
                let center = point(*center);
                let radius = *radius * stroke_scale;
                scene.rounded_rect(
                    Rect::new(
                        center.x - radius,
                        center.y - radius,
                        radius * 2.0,
                        radius * 2.0,
                    ),
                    radius,
                    paint.resolve(theme),
                );
            }
            CanvasCommand::Text {
                rect: local,
                text,
                size,
                paint,
                align,
            } => scene.styled_text(
                rect(*local),
                text,
                *size * stroke_scale,
                paint.resolve(theme),
                *align,
                crate::TextOverflow::Clip,
            ),
            CanvasCommand::FillLinearGradientRect {
                rect: local,
                radius,
                gradient,
            } => scene.linear_gradient_rect(
                rect(*local),
                *radius * stroke_scale,
                point(gradient.start),
                point(gradient.end),
                gradient.start_color.resolve(theme),
                gradient.end_color.resolve(theme),
            ),
            CanvasCommand::StrokePath {
                subpaths,
                width,
                paint,
            } => {
                let color = paint.resolve(theme);
                for subpath in subpaths.iter() {
                    for pair in subpath.windows(2) {
                        scene.line(point(pair[0]), point(pair[1]), *width * stroke_scale, color);
                    }
                }
            }
            CanvasCommand::FillPath { triangles, paint } => scene.triangle_mesh(
                triangles.iter().copied().map(point).collect(),
                paint.resolve(theme),
            ),
        }
    }
    scene.pop_clip();
}

fn intrinsic_flex_size(
    children: &[FlexItem],
    gap: f32,
    padding: f32,
    row: bool,
    maximum: crate::Size,
    theme: Theme,
    measurer: &impl LayoutMeasurer,
) -> crate::Size {
    let padding = padding.max(0.0);
    let inner = crate::Size::new(
        (maximum.width - padding * 2.0).max(0.0),
        (maximum.height - padding * 2.0).max(0.0),
    );
    let mut primary = 0.0_f32;
    let mut cross = 0.0_f32;
    for child in children {
        let measured = intrinsic_size(&child.node, inner, theme, measurer);
        let measured_primary = if row { measured.width } else { measured.height };
        let child_primary = if child.flex.intrinsic {
            measured_primary
        } else {
            child.flex.basis
        }
        .clamp(child.flex.minimum, child.flex.maximum);
        let child_cross = if row { measured.height } else { measured.width };
        primary += child_primary;
        cross = cross.max(child_cross);
    }
    primary += gap.max(0.0) * children.len().saturating_sub(1) as f32 + padding * 2.0;
    cross += padding * 2.0;
    if row {
        crate::Size::new(primary.min(maximum.width), cross.min(maximum.height))
    } else {
        crate::Size::new(cross.min(maximum.width), primary.min(maximum.height))
    }
}

fn group_semantics(bounds: Rect, children: Vec<SemanticNode>) -> SemanticNode {
    SemanticNode {
        id: None,
        role: SemanticRole::Group,
        label: String::new(),
        value: None,
        hint: None,
        bounds,
        enabled: true,
        selected: false,
        children,
    }
}

fn draw_control_content(
    scene: &mut Scene,
    bounds: Rect,
    icon: &Option<Icon>,
    label: &str,
    color: Color,
) {
    let inner = bounds.inset(8.0);
    match icon {
        Some(icon) if label.is_empty() || inner.width < 58.0 => {
            scene.icon(inner, *icon, color);
        }
        Some(icon) => {
            let icon_width = inner.height.min(22.0);
            scene.icon(
                Rect::new(inner.x, inner.y, icon_width, inner.height),
                *icon,
                color,
            );
            scene.text(
                Rect::new(
                    inner.x + icon_width + 5.0,
                    inner.y,
                    (inner.width - icon_width - 5.0).max(0.0),
                    inner.height,
                ),
                label,
                13.0,
                color,
                TextAlign::Leading,
            );
        }
        None => scene.text(inner, label, 13.0, color, TextAlign::Center),
    }
}

fn fit_image(bounds: Rect, image: &Image, fit: ImageFit) -> Rect {
    if fit == ImageFit::Stretch {
        return bounds;
    }
    let scale_x = bounds.width / image.width as f32;
    let scale_y = bounds.height / image.height as f32;
    let scale = match fit {
        ImageFit::Contain => scale_x.min(scale_y),
        ImageFit::Cover => scale_x.max(scale_y),
        ImageFit::Stretch => unreachable!(),
    };
    let width = image.width as f32 * scale;
    let height = image.height as f32 * scale;
    Rect::new(
        bounds.x + (bounds.width - width) * 0.5,
        bounds.y + (bounds.height - height) * 0.5,
        width,
        height,
    )
}

fn draw_styled_slider(
    scene: &mut Scene,
    interactions: &mut InteractionMap,
    id: WidgetId,
    bounds: Rect,
    value: ContinuousValue,
    style: SliderStyle,
    theme: Theme,
) {
    let normalized = value.normalized();
    let thumb_diameter = if style.show_thumb {
        style.thumb_diameter.clamp(2.0, bounds.height)
    } else {
        0.0
    };
    let radius = thumb_diameter * 0.5;
    let track_height = style.track_height.clamp(1.0, bounds.height);
    let track = Rect::new(
        bounds.x + radius,
        bounds.y + (bounds.height - track_height) * 0.5,
        (bounds.width - thumb_diameter).max(1.0),
        track_height,
    );
    scene.rounded_rect(track, track_height * 0.5, style.track.resolve(theme));
    scene.rounded_rect(
        Rect::new(track.x, track.y, track.width * normalized, track.height),
        track_height * 0.5,
        style.fill.resolve(theme),
    );
    if style.tick_count > 1 {
        for index in 0..style.tick_count {
            let amount = f32::from(index) / f32::from(style.tick_count - 1);
            let x = track.x + track.width * amount;
            scene.rounded_rect(
                Rect::new(x - 0.5, track.y - 2.0, 1.0, track.height + 4.0),
                0.5,
                theme.muted,
            );
        }
    }
    if style.show_thumb {
        let center_x = track.x + track.width * normalized;
        scene.rounded_rect(
            Rect::new(
                center_x - radius,
                bounds.y + (bounds.height - thumb_diameter) * 0.5,
                thumb_diameter,
                thumb_diameter,
            ),
            radius,
            style.thumb.resolve(theme),
        );
    }
    interactions.slider(id, bounds);
}

// Float raster coordinates are deliberately quantized; dimensions are bounded.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn draw_meter(
    scene: &mut Scene,
    bounds: Rect,
    value: ContinuousValue,
    peak: Option<f32>,
    style: MeterStyle,
    theme: Theme,
) {
    let amount = value.normalized();
    let track = style.track.resolve(theme);
    let low = style.low.resolve(theme);
    let high = style.high.resolve(theme);
    if style.segments < 2 {
        scene.rounded_rect(bounds, bounds.height * 0.5, track);
        scene.rounded_rect(
            Rect::new(bounds.x, bounds.y, bounds.width * amount, bounds.height),
            bounds.height * 0.5,
            low.mix(high, amount),
        );
    } else {
        let count = usize::from(style.segments);
        let gap = style.gap.max(0.0);
        let segment_width =
            ((bounds.width - gap * (count.saturating_sub(1)) as f32) / count as f32).max(0.0);
        let active = (amount * count as f32).ceil() as usize;
        for index in 0..count {
            let progress = if count > 1 {
                index as f32 / (count - 1) as f32
            } else {
                0.0
            };
            scene.rounded_rect(
                Rect::new(
                    bounds.x + index as f32 * (segment_width + gap),
                    bounds.y,
                    segment_width,
                    bounds.height,
                ),
                segment_width.min(bounds.height) * 0.3,
                if index < active {
                    low.mix(high, progress)
                } else {
                    track
                },
            );
        }
    }
    if let Some(peak) = peak {
        let peak = ContinuousValue::new(value.minimum(), value.maximum(), peak).normalized();
        let x = bounds.x + bounds.width * peak;
        scene.rounded_rect(
            Rect::new(x - 1.0, bounds.y - 1.0, 2.0, bounds.height + 2.0),
            1.0,
            style.peak.resolve(theme),
        );
    }
}

// Float raster coordinates are deliberately quantized; dimensions are bounded.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn draw_tiny_graph(
    scene: &mut Scene,
    bounds: Rect,
    samples: &[f32],
    minimum: f32,
    maximum: f32,
    style: TinyGraphStyle,
    theme: Theme,
) {
    if samples.is_empty() || bounds.width <= 0.0 || bounds.height <= 0.0 {
        return;
    }
    if let Some(role) = style.baseline {
        scene.rounded_rect(
            Rect::new(bounds.x, bounds.y + bounds.height - 1.0, bounds.width, 1.0),
            0.5,
            role.resolve(theme),
        );
    }
    let minimum = if minimum.is_finite() { minimum } else { 0.0 };
    let maximum = if maximum.is_finite() && maximum > minimum {
        maximum
    } else {
        minimum + 1.0
    };
    let visible_count = samples.len().min(bounds.width.ceil().max(1.0) as usize);
    let group_size = samples.len().div_ceil(visible_count);
    let reduced_count = samples.len().div_ceil(group_size);
    let slot = bounds.width / reduced_count as f32;
    let bar_width = (slot - style.gap.max(0.0)).max(0.5);
    let low = style.low.resolve(theme);
    let high = style.high.resolve(theme);
    for (index, chunk) in samples.chunks(group_size).enumerate() {
        let sample = chunk.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let amount = if sample.is_finite() {
            ((sample - minimum) / (maximum - minimum)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let bar_height = (bounds.height * amount)
            .max(style.minimum_bar_height)
            .min(bounds.height);
        scene.rounded_rect(
            Rect::new(
                bounds.x + index as f32 * slot,
                bounds.y + bounds.height - bar_height,
                bar_width,
                bar_height,
            ),
            bar_width.min(2.0) * 0.5,
            low.mix(high, amount),
        );
    }
}

fn format_continuous_value(value: ContinuousValue) -> String {
    if value.minimum() == 0.0 && value.maximum() == 1.0 {
        format!("{:.0}%", value.normalized() * 100.0)
    } else {
        format!("{:.2}", value.value())
    }
}

fn draw_progress(
    scene: &mut Scene,
    bounds: Rect,
    value: ProgressValue,
    track_color: Color,
    fill_color: Color,
) {
    let height = bounds.height.clamp(0.0, 8.0);
    let track = Rect::new(
        bounds.x,
        bounds.y + (bounds.height - height) * 0.5,
        bounds.width,
        height,
    );
    scene.rounded_rect(track, height * 0.5, track_color);
    let fill = match value {
        ProgressValue::Determinate(value) => Rect::new(
            track.x,
            track.y,
            track.width * value.clamp(0.0, 1.0),
            track.height,
        ),
        ProgressValue::Indeterminate { phase } => {
            let segment = track.width * 0.3;
            Rect::new(
                track.x + (track.width - segment) * phase.rem_euclid(1.0),
                track.y,
                segment,
                track.height,
            )
        }
    };
    scene.rounded_rect(fill, height * 0.5, fill_color);
}

impl ColorRole {
    /// Resolve a semantic paint role against the latest daemon theme snapshot.
    pub fn resolve(self, theme: Theme) -> Color {
        match self {
            Self::Background => theme.background,
            Self::Control => theme.control,
            Self::ControlPressed => theme.control_pressed,
            Self::Track => theme.track,
            Self::Foreground => theme.foreground,
            Self::Muted => theme.muted,
            Self::Accent => theme.accent,
            Self::Destructive => theme.destructive,
            Self::OnAccent => theme.accent.contrasting_foreground(),
            Self::OnDestructive => theme.destructive.contrasting_foreground(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameScheduler {
    dirty: bool,
    visible: bool,
    continuous: bool,
    animate_until: Option<Duration>,
}

impl Default for FrameScheduler {
    fn default() -> Self {
        Self {
            dirty: true,
            visible: true,
            continuous: false,
            animate_until: None,
        }
    }
}

impl FrameScheduler {
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    pub fn set_visible(&mut self, visible: bool) {
        if visible && !self.visible {
            self.dirty = true;
        }
        self.visible = visible;
    }

    pub fn set_continuous(&mut self, continuous: bool) {
        self.continuous = continuous;
        if continuous {
            self.dirty = true;
        }
    }

    pub fn animate_for(&mut self, now: Duration, duration: Duration) {
        let deadline = now.saturating_add(duration);
        if self.animate_until.is_none_or(|current| deadline > current) {
            self.animate_until = Some(deadline);
        }
        self.dirty = true;
    }

    pub fn should_render(&self, now: Duration) -> bool {
        self.visible && (self.dirty || self.is_animating(now))
    }

    pub fn frame_rendered(&mut self, now: Duration) {
        self.dirty = false;
        if self.animate_until.is_some_and(|deadline| now >= deadline) {
            self.animate_until = None;
        }
    }

    pub fn wants_next_frame(&self, now: Duration) -> bool {
        self.visible && self.is_animating(now)
    }

    fn is_animating(&self, now: Duration) -> bool {
        self.continuous || self.animate_until.is_some_and(|deadline| now < deadline)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn flex_shrinks_then_hides_low_priority_trailing_items() {
        let items = [
            Flex::flexible(40.0, 80.0, 120.0).required(),
            Flex::fixed(50.0).priority(-1),
            Flex::fixed(50.0).priority(-1),
        ];
        let placements = resolve_flex_row(Rect::new(0.0, 0.0, 100.0, 60.0), &items, 4.0);
        assert!(placements[0].is_some());
        assert!(placements[1].is_some());
        assert!(placements[2].is_none());
        assert_eq!(placements[0].unwrap().width, 46.0);
    }

    #[test]
    fn responsive_nodes_choose_the_richest_fitting_representation() {
        let id = WidgetId(7);
        let node = Node::responsive(
            id,
            vec![
                ResponsiveVariant::new(Representation::Minimal, 24.0, Node::label("M", 10.0)),
                ResponsiveVariant::new(Representation::Full, 120.0, Node::label("FULL", 10.0)),
                ResponsiveVariant::new(Representation::Compact, 60.0, Node::label("COMPACT", 10.0)),
            ],
        );
        let resolved =
            RetainedUi::new(node).resolve(Rect::new(0.0, 0.0, 80.0, 60.0), Theme::default());
        assert_eq!(
            resolved.inspector.representations.get(&id),
            Some(&Representation::Compact)
        );
    }

    #[test]
    fn one_layout_produces_pixels_hit_targets_and_semantics() {
        let id = WidgetId(4);
        let node = Node::button(id, "Play", Some(Icon::Play), false);
        let resolved =
            RetainedUi::new(node).resolve(Rect::new(0.0, 0.0, 96.0, 60.0), Theme::default());
        assert!(!resolved.scene.primitives.is_empty());
        assert_eq!(resolved.interactions.targets.len(), 1);
        assert_eq!(resolved.inspector.semantics.id, Some(id));
        assert_eq!(resolved.inspector.semantics.bounds.width, 96.0);
    }

    #[test]
    fn pressable_accepts_arbitrary_layout_and_keeps_one_control_identity() {
        let id = WidgetId(90);
        let node = Node::Pressable {
            id,
            label: "Now playing".into(),
            pressed: false,
            hold: Some(Duration::from_millis(250)),
            selected: None,
            style: PressableStyle::control(),
            child: Box::new(Node::row(
                4.0,
                0.0,
                vec![
                    FlexItem::new(
                        Flex::fixed(20.0),
                        Node::Icon {
                            icon: Icon::Play,
                            color: ColorRole::Accent,
                            label: "Playing".into(),
                        },
                    ),
                    FlexItem::new(
                        Flex::content(10.0, 120.0).grow(1.0),
                        Node::label("A long track title", 12.0),
                    ),
                ],
            )),
        };
        let resolved =
            RetainedUi::new(node).resolve(Rect::new(0.0, 0.0, 180.0, 60.0), Theme::default());

        assert_eq!(resolved.interactions.targets.len(), 1);
        assert_eq!(resolved.interactions.targets[0].id, id);
        assert_eq!(
            resolved.interactions.targets[0].kind,
            crate::InteractionKind::Button {
                hold: Some(Duration::from_millis(250))
            }
        );
        assert_eq!(resolved.inspector.semantics.id, Some(id));
        assert_eq!(resolved.inspector.semantics.role, SemanticRole::Button);
        assert_eq!(resolved.inspector.semantics.children.len(), 1);
        assert_eq!(resolved.inspector.semantics.children[0].children.len(), 2);
    }

    #[test]
    fn image_fit_and_theme_tint_are_resolved_each_frame() {
        let image = Image::rgba8(91, 1, 100, 50, vec![255; 100 * 50 * 4]).unwrap();
        let build = || Node::Image {
            image: image.clone(),
            opacity: 0.8,
            fit: ImageFit::Contain,
            tint: ImageTint::Mask(ColorRole::Accent),
            label: "Tintable logo".into(),
        };
        let first_theme = Theme {
            accent: Color::rgb(1.0, 0.0, 0.0),
            ..Theme::default()
        };
        let first = RetainedUi::new(build()).resolve(Rect::new(0.0, 0.0, 60.0, 60.0), first_theme);
        let [
            crate::Primitive::Image {
                rect,
                image: rendered,
                opacity,
                coloring,
            },
        ] = first.scene.primitives.as_slice()
        else {
            panic!("a contained image should emit one image primitive");
        };
        assert_rect_near(*rect, Rect::new(0.0, 15.0, 60.0, 30.0));
        assert_eq!(rendered, &image);
        assert_eq!(*opacity, 0.8);
        assert_eq!(*coloring, ImageColoring::Mask(first_theme.accent));

        let mut second_theme = first_theme;
        second_theme.accent = Color::rgb(0.0, 0.5, 1.0);
        let second =
            RetainedUi::new(build()).resolve(Rect::new(0.0, 0.0, 60.0, 60.0), second_theme);
        assert!(matches!(
            second.scene.primitives[0],
            crate::Primitive::Image {
                coloring: ImageColoring::Mask(color),
                ..
            } if color == second_theme.accent
        ));
    }

    #[test]
    fn cover_images_clip_and_opacity_wraps_all_child_paint() {
        let image = Image::rgba8(92, 1, 100, 50, vec![255; 100 * 50 * 4]).unwrap();
        let node = Node::opacity(
            0.4,
            Node::Image {
                image,
                opacity: 1.0,
                fit: ImageFit::Cover,
                tint: ImageTint::None,
                label: "Wide crop".into(),
            },
        );
        let resolved =
            RetainedUi::new(node).resolve(Rect::new(10.0, 5.0, 40.0, 60.0), Theme::default());
        let [
            crate::Primitive::PushOpacity { opacity },
            crate::Primitive::PushClip { rect: clip },
            crate::Primitive::Image { rect, .. },
            crate::Primitive::PopClip,
            crate::Primitive::PopOpacity,
        ] = resolved.scene.primitives.as_slice()
        else {
            panic!("cover image should be clipped inside its opacity group");
        };
        assert_eq!(*opacity, 0.4);
        assert_rect_near(*clip, Rect::new(10.0, 5.0, 40.0, 60.0));
        assert_rect_near(*rect, Rect::new(-30.0, 5.0, 120.0, 60.0));
    }

    #[test]
    fn progress_is_a_theme_aware_semantic_leaf() {
        let theme = Theme {
            accent: Color::rgb(0.2, 0.7, 0.9),
            ..Theme::default()
        };
        let resolved = RetainedUi::new(Node::progress("Track position", 0.25))
            .resolve(Rect::new(0.0, 0.0, 100.0, 30.0), theme);
        assert_eq!(resolved.inspector.semantics.role, SemanticRole::Progress);
        assert_eq!(resolved.inspector.semantics.value.as_deref(), Some("25%"));
        assert!(
            resolved.scene.primitives.iter().any(
                |p| matches!(p,crate::Primitive::RoundedRect {color,..} if *color==theme.accent)
            )
        );
    }

    #[test]
    fn shader_effect_is_a_dynamic_theme_leaf_with_host_timed_motion() {
        let program = Arc::new(
            crate::EffectProgram::compile(
                "let amount = 0.5 + 0.5 * sin(uv.x * 8.0 - time); let color = mix(background, accent, amount);",
            )
            .unwrap(),
        );
        let effect = crate::ShaderEffect {
            program,
            parameters: [0.0; crate::MAX_EFFECT_PARAMETERS],
            opacity: 0.75,
            started: Duration::from_secs(1),
            period: Some(Duration::from_secs(2)),
        };
        let theme = Theme {
            accent: Color::rgb(0.2, 0.8, 0.4),
            ..Theme::default()
        };
        let resolved = RetainedUi::new(Node::shader_effect(
            "Ambient wave",
            Size::new(160.0, 60.0),
            effect.clone(),
        ))
        .resolve(Rect::new(10.0, 5.0, 220.0, 50.0), theme);
        assert_eq!(resolved.inspector.semantics.role, SemanticRole::Canvas);
        assert!(matches!(
            &resolved.scene.primitives[0],
            crate::Primitive::ShaderEffect {
                effect: received,
                rect,
                theme: received_theme,
            } if received == &effect
                && *rect == Rect::new(10.0, 5.0, 220.0, 50.0)
                && *received_theme == theme
        ));
        assert!(
            resolved
                .scene
                .has_active_motion(Duration::from_secs(3), crate::MotionPolicy::Full)
        );
        assert!(
            !resolved
                .scene
                .has_active_motion(Duration::from_secs(3), crate::MotionPolicy::Reduced)
        );
        assert_eq!(
            effect.time(Duration::from_millis(3_500), crate::MotionPolicy::Full),
            0.5
        );
        assert_eq!(
            effect.time(Duration::from_millis(3_500), crate::MotionPolicy::Reduced),
            0.0
        );
    }

    #[test]
    fn styled_slider_maps_ranges_and_keeps_normalized_input_geometry() {
        let id = WidgetId(81);
        let value = ContinuousValue::new(-20.0, 20.0, 0.0).step(5.0);
        let style = SliderStyle {
            tick_count: 5,
            ..SliderStyle::default()
        };
        let bounds = Rect::new(10.0, 4.0, 140.0, 44.0);
        let resolved = RetainedUi::new(Node::styled_slider(id, "Pan", value, style))
            .resolve(bounds, Theme::default());
        assert_eq!(resolved.interactions.target(id).unwrap().bounds, bounds);
        assert_eq!(resolved.inspector.semantics.value.as_deref(), Some("0.00"));
        assert_eq!(
            resolved.inspector.semantics.hint.as_deref(),
            Some("Slide horizontally to adjust")
        );
        assert!(resolved.scene.primitives.iter().any(|primitive| matches!(
            primitive,
            crate::Primitive::RoundedRect { rect, color, .. }
                if *color == Theme::default().accent && (rect.width - 61.0).abs() < 0.01
        )));
    }

    #[test]
    fn meter_graph_and_scrubber_cells_resolve_live_theme_roles() {
        let theme = Theme {
            accent: Color::rgb(0.95, 0.85, 0.15),
            ..Theme::default()
        };
        let meter = Node::meter(
            "Input level",
            ContinuousValue::unit(0.6),
            Some(0.8),
            MeterStyle {
                segments: 5,
                ..MeterStyle::default()
            },
        );
        let meter = RetainedUi::new(meter).resolve(Rect::new(0.0, 0.0, 100.0, 8.0), theme);
        assert_eq!(meter.inspector.semantics.value.as_deref(), Some("60%"));
        assert_eq!(meter.scene.primitives.len(), 6);

        let graph = Node::tiny_graph(
            "CPU history",
            [0.0, 0.25, 0.5, 1.0],
            0.0,
            1.0,
            TinyGraphStyle::default(),
        );
        let graph = RetainedUi::new(graph).resolve(Rect::new(0.0, 0.0, 40.0, 20.0), theme);
        assert_eq!(graph.scene.primitives.len(), 5);
        assert!(graph.scene.primitives.iter().any(|primitive| matches!(
            primitive,
            crate::Primitive::RoundedRect { color, .. } if *color == theme.accent
        )));

        let cell = Node::scrubber_cell(
            "2",
            ScrubberItemState {
                selected: true,
                highlighted: false,
            },
            ScrubberCellStyle::default(),
        );
        let cell = RetainedUi::new(cell).resolve(Rect::new(0.0, 0.0, 40.0, 40.0), theme);
        assert!(cell.scene.primitives.iter().any(|primitive| matches!(
            primitive,
            crate::Primitive::Text { color, .. } if *color == Color::BLACK
        )));
    }

    #[test]
    fn selected_toggle_derives_readable_text_from_a_light_accent() {
        let theme = Theme {
            accent: Color::rgb(0.96, 0.88, 0.20),
            foreground: Color::WHITE,
            ..Theme::default()
        };
        let node = Node::Toggle {
            id: WidgetId(93),
            label: "SELECTED".into(),
            icon: None,
            selected: true,
            pressed: false,
        };
        let resolved = RetainedUi::new(node).resolve(Rect::new(0.0, 0.0, 100.0, 44.0), theme);
        assert!(resolved.scene.primitives.iter().any(|primitive| matches!(
            primitive,
            crate::Primitive::Text { color, .. } if *color == Color::BLACK
        )));
    }

    #[test]
    fn intrinsic_text_uses_the_supplied_measurer() {
        struct FixedMeasurer(Cell<u32>);

        impl LayoutMeasurer for FixedMeasurer {
            fn measure_text(&self, _text: &str, _size: f32, maximum: crate::Size) -> crate::Size {
                self.0.set(self.0.get() + 1);
                crate::Size::new(73.0_f32.min(maximum.width), 11.0_f32.min(maximum.height))
            }
        }

        let node = Node::row_aligned(
            0.0,
            0.0,
            CrossAxisAlignment::Start,
            vec![FlexItem::new(
                Flex::content(0.0, 100.0),
                Node::label("measured", 13.0),
            )],
        );
        let measurer = FixedMeasurer(Cell::new(0));
        let resolved = RetainedUi::new(node).resolve_with_measurer(
            Rect::new(0.0, 0.0, 120.0, 60.0),
            Theme::default(),
            &measurer,
        );
        let label = &resolved.inspector.semantics.children[0];
        assert_eq!(label.bounds, Rect::new(0.0, 0.0, 73.0, 11.0));
        assert!(measurer.0.get() > 0);
    }

    #[test]
    fn stable_text_reserves_sample_width_and_marquee_is_preserved() {
        struct LengthMeasurer;

        impl LayoutMeasurer for LengthMeasurer {
            fn measure_text(&self, text: &str, _size: f32, maximum: crate::Size) -> crate::Size {
                crate::Size::new(
                    ((text.len() * 10) as f32).min(maximum.width),
                    12.0_f32.min(maximum.height),
                )
            }
        }

        let node = Node::row(
            0.0,
            0.0,
            vec![FlexItem::new(
                Flex::content(0.0, 100.0),
                Node::stable_label("1:02", "00:00", 10.0),
            )],
        );
        let resolved = RetainedUi::new(node).resolve_with_measurer(
            Rect::new(0.0, 0.0, 100.0, 20.0),
            Theme::default(),
            &LengthMeasurer,
        );
        assert_eq!(resolved.inspector.semantics.children[0].bounds.width, 50.0);

        let marquee = Node::marquee_label("A very long title", 11.0, 24.0, 30.0);
        let resolved =
            RetainedUi::new(marquee).resolve(Rect::new(0.0, 0.0, 40.0, 20.0), Theme::default());
        assert!(resolved.scene.primitives.iter().any(|primitive| matches!(
            primitive,
            crate::Primitive::Text {
                overflow: crate::TextOverflow::Marquee {
                    speed: 24.0,
                    gap: 30.0
                },
                ..
            }
        )));
    }

    #[test]
    fn motion_wraps_paint_without_moving_semantic_geometry() {
        let motion = crate::Motion {
            id: crate::MotionId(9),
            from: crate::VisualTransform {
                translation: Point::new(8.0, 0.0),
                scale: 0.9,
                opacity: 0.0,
            },
            to: crate::VisualTransform::IDENTITY,
            started: Duration::ZERO,
            duration: Duration::from_millis(200),
            easing: crate::Easing::EaseInOut,
            playback: crate::MotionPlayback::Once,
        };
        let bounds = Rect::new(10.0, 4.0, 80.0, 40.0);
        let resolved = RetainedUi::new(Node::motion(motion, Node::label("Title", 11.0)))
            .resolve(bounds, Theme::default());
        assert_eq!(resolved.inspector.semantics.bounds, bounds);
        assert!(matches!(
            resolved.scene.primitives.first(),
            Some(crate::Primitive::PushMotion { origin, motion: emitted })
                if *origin == Point::new(50.0, 24.0) && *emitted == motion
        ));
        assert!(matches!(
            resolved.scene.primitives.last(),
            Some(crate::Primitive::PopMotion)
        ));
    }

    #[test]
    fn nested_layout_adapts_without_losing_widget_identity() {
        let responsive = WidgetId(40);
        let play = WidgetId(41);
        let artwork = Image::rgba8(9, 1, 32, 20, vec![255; 32 * 20 * 4]).unwrap();
        let build = || {
            Node::row_aligned(
                4.0,
                4.0,
                CrossAxisAlignment::Center,
                vec![
                    FlexItem::new(
                        Flex::fixed(32.0).priority(-1),
                        Node::Image {
                            image: artwork.clone(),
                            opacity: 1.0,
                            fit: ImageFit::Cover,
                            tint: ImageTint::None,
                            label: "Artwork".into(),
                        },
                    ),
                    FlexItem::new(
                        Flex::flexible(30.0, 80.0, 300.0).grow(1.0).required(),
                        Node::column_aligned(
                            2.0,
                            0.0,
                            CrossAxisAlignment::Start,
                            vec![
                                FlexItem::new(
                                    Flex::content(10.0, 20.0),
                                    Node::responsive(
                                        responsive,
                                        vec![
                                            ResponsiveVariant::new(
                                                Representation::Minimal,
                                                0.0,
                                                Node::label("M", 10.0),
                                            ),
                                            ResponsiveVariant::new(
                                                Representation::Full,
                                                80.0,
                                                Node::label("A full track title", 10.0),
                                            ),
                                        ],
                                    ),
                                ),
                                FlexItem::new(
                                    Flex::content(10.0, 20.0),
                                    Node::label("Artist", 9.0),
                                ),
                            ],
                        ),
                    ),
                    FlexItem::new(
                        Flex::fixed(40.0).required(),
                        Node::button(play, "", Some(Icon::Play), false),
                    ),
                ],
            )
        };

        let wide =
            RetainedUi::new(build()).resolve(Rect::new(0.0, 0.0, 220.0, 60.0), Theme::default());
        assert_eq!(wide.inspector.semantics.children.len(), 3);
        assert_eq!(
            wide.inspector.representations.get(&responsive),
            Some(&Representation::Full)
        );
        assert_eq!(wide.interactions.targets[0].id, play);

        let narrow =
            RetainedUi::new(build()).resolve(Rect::new(0.0, 0.0, 100.0, 60.0), Theme::default());
        assert_eq!(narrow.inspector.semantics.children.len(), 2);
        assert_eq!(
            narrow.inspector.representations.get(&responsive),
            Some(&Representation::Minimal)
        );
        assert_eq!(narrow.interactions.targets[0].id, play);
        assert!(narrow.interactions.targets[0].bounds.x < wide.interactions.targets[0].bounds.x);
    }

    #[test]
    fn column_uses_the_same_priority_hiding_contract() {
        let node = Node::column(
            4.0,
            0.0,
            vec![
                FlexItem::new(Flex::fixed(18.0).required(), Node::label("kept", 10.0)),
                FlexItem::new(Flex::fixed(18.0).priority(-1), Node::label("hidden", 10.0)),
            ],
        );
        let resolved =
            RetainedUi::new(node).resolve(Rect::new(0.0, 0.0, 80.0, 30.0), Theme::default());
        assert_eq!(resolved.inspector.semantics.children.len(), 1);
        assert_eq!(resolved.inspector.semantics.children[0].label, "kept");
    }

    #[test]
    fn retained_tree_and_scheduler_suspend_when_hidden() {
        let mut ui = RetainedUi::new(Node::label("READY", 12.0));
        assert!(ui.needs_render());
        ui.resolve(Rect::new(0.0, 0.0, 80.0, 60.0), Theme::default());
        assert!(!ui.needs_render());
        ui.set_visible(false);
        ui.invalidate();
        assert!(!ui.needs_render());
        ui.set_visible(true);
        assert!(ui.needs_render());

        let mut scheduler = FrameScheduler::default();
        scheduler.set_continuous(true);
        scheduler.set_visible(false);
        assert!(!scheduler.should_render(Duration::ZERO));
        assert!(!scheduler.wants_next_frame(Duration::ZERO));
        scheduler.set_visible(true);
        assert!(scheduler.should_render(Duration::ZERO));
    }

    #[test]
    fn canvas_scales_clips_and_reresolves_semantic_theme_paints() {
        let commands: Arc<[CanvasCommand]> = vec![
            CanvasCommand::FillRect {
                rect: Rect::new(5.0, 4.0, 20.0, 10.0),
                radius: 2.0,
                paint: CanvasPaint::role(ColorRole::Accent).with_opacity(0.5),
            },
            CanvasCommand::Line {
                start: Point::new(0.0, 0.0),
                end: Point::new(100.0, 50.0),
                width: 2.0,
                paint: CanvasPaint::role(ColorRole::Foreground),
            },
            CanvasCommand::Polyline {
                points: vec![
                    Point::new(0.0, 40.0),
                    Point::new(50.0, 20.0),
                    Point::new(100.0, 40.0),
                ]
                .into(),
                width: 1.0,
                paint: CanvasPaint::rgba(Color::rgba(0.2, 0.4, 0.8, 0.75)),
            },
            CanvasCommand::FillCircle {
                center: Point::new(75.0, 15.0),
                radius: 5.0,
                paint: CanvasPaint::role(ColorRole::Destructive),
            },
            CanvasCommand::FillLinearGradientRect {
                rect: Rect::new(0.0, 45.0, 100.0, 5.0),
                radius: 2.5,
                gradient: CanvasLinearGradient {
                    start: Point::new(0.0, 47.5),
                    end: Point::new(100.0, 47.5),
                    start_color: CanvasPaint::role(ColorRole::Accent),
                    end_color: CanvasPaint::role(ColorRole::Muted).with_opacity(0.25),
                },
            },
            CanvasCommand::StrokePath {
                subpaths: vec![
                    vec![
                        Point::new(5.0, 35.0),
                        Point::new(25.0, 10.0),
                        Point::new(45.0, 35.0),
                    ]
                    .into(),
                ]
                .into(),
                width: 1.5,
                paint: CanvasPaint::role(ColorRole::Accent),
            },
            CanvasCommand::FillPath {
                triangles: vec![
                    Point::new(60.0, 10.0),
                    Point::new(70.0, 20.0),
                    Point::new(60.0, 30.0),
                ]
                .into(),
                paint: CanvasPaint::role(ColorRole::Accent),
            },
            CanvasCommand::Text {
                rect: Rect::new(10.0, 25.0, 30.0, 15.0),
                text: "GPU".into(),
                size: 8.0,
                paint: CanvasPaint::role(ColorRole::Muted),
                align: TextAlign::Leading,
            },
        ]
        .into();
        let node = Node::canvas("Canvas graph", Size::new(100.0, 50.0), commands);
        let theme = Theme {
            accent: Color::rgba(0.8, 0.3, 0.1, 0.8),
            foreground: Color::rgb(0.9, 0.9, 0.9),
            destructive: Color::rgb(1.0, 0.1, 0.2),
            muted: Color::rgb(0.4, 0.5, 0.6),
            ..Theme::default()
        };
        let mut ui = RetainedUi::new(node);
        let canvas_bounds = Rect::new(10.0, 5.0, 200.0, 100.0);
        let resolved = ui.resolve(canvas_bounds, theme);

        assert_eq!(resolved.inspector.semantics.role, SemanticRole::Canvas);
        assert_eq!(resolved.inspector.semantics.label, "Canvas graph");
        assert!(matches!(
            resolved.scene.primitives.first(),
            Some(crate::Primitive::PushClip { rect }) if *rect == Rect::new(10.0, 5.0, 200.0, 100.0)
        ));
        assert!(matches!(
            &resolved.scene.primitives[1],
            crate::Primitive::RoundedRect { rect, radius, color }
                if *rect == Rect::new(20.0, 13.0, 40.0, 20.0)
                    && *radius == 4.0
                    && (color.alpha - 0.4).abs() < 0.001
        ));
        assert!(matches!(
            &resolved.scene.primitives[2],
            crate::Primitive::Line { start, end, width, color }
                if *start == Point::new(10.0, 5.0)
                    && *end == Point::new(210.0, 105.0)
                    && *width == 4.0
                    && *color == theme.foreground
        ));
        assert_eq!(
            resolved
                .scene
                .primitives
                .iter()
                .filter(|primitive| matches!(primitive, crate::Primitive::Line { .. }))
                .count(),
            5
        );
        assert!(resolved.scene.primitives.iter().any(|primitive| matches!(
            primitive,
            crate::Primitive::LinearGradientRect {
                rect,
                start,
                end,
                start_color,
                end_color,
                ..
            } if *rect == Rect::new(10.0, 95.0, 200.0, 10.0)
                && *start == Point::new(10.0, 100.0)
                && *end == Point::new(210.0, 100.0)
                && *start_color == theme.accent
                && end_color.alpha == theme.muted.alpha * 0.25
        )));
        assert!(resolved.scene.primitives.iter().any(|primitive| matches!(
            primitive,
            crate::Primitive::TriangleMesh { triangles, color }
                if triangles.as_ref() == [
                    Point::new(130.0, 25.0),
                    Point::new(150.0, 45.0),
                    Point::new(130.0, 65.0),
                ] && *color == theme.accent
        )));
        assert!(matches!(
            resolved.scene.primitives.last(),
            Some(crate::Primitive::PopClip)
        ));

        let changed_theme = Theme {
            accent: Color::rgb(0.1, 0.8, 0.4),
            ..theme
        };
        let changed = ui.resolve(canvas_bounds, changed_theme);
        assert!(matches!(
            &changed.scene.primitives[1],
            crate::Primitive::RoundedRect { color, .. }
                if *color == Color::rgba(0.1, 0.8, 0.4, 0.5)
        ));
    }

    fn assert_rect_near(actual: Rect, expected: Rect) {
        for (actual, expected) in [
            (actual.x, expected.x),
            (actual.y, expected.y),
            (actual.width, expected.width),
            (actual.height, expected.height),
        ] {
            assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
        }
    }
}
