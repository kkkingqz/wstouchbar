use crate::SetDocument;
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};
use touchbar_core::{SceneItem, ScrubberItemGeometry};
use touchbar_model::{self as m, Scalar, StateValue, WidgetSpec};
use touchbar_sets::{LayoutKind as Kind, LayoutNode, ScalarBinding, TransformDefinition};
use touchbar_ui::{self as ui, ColorRole, Flex, FlexItem, Node, PressableStyle};

#[derive(Clone)]
pub(crate) struct Identity {
    pub widget: m::WidgetId,
    pub record: Option<String>,
    pub scrubber: Option<(f32, Vec<ScrubberItemGeometry>)>,
}
#[derive(Clone, Default)]
pub(crate) struct Timelines {
    motions: BTreeMap<(m::WidgetId, Option<String>, u64), ui::Motion>,
    effects: BTreeMap<(m::WidgetId, Option<String>, u64), ui::ShaderEffect>,
}
pub(crate) struct Built {
    pub root: Node,
    pub identities: BTreeMap<ui::WidgetId, Identity>,
    pub timelines: Timelines,
    pub warnings: Vec<String>,
}
#[derive(Clone, Copy)]
struct Record<'a> {
    fields: &'a m::RecordValue,
    key: &'a str,
}
struct Builder<'a> {
    item: &'a SceneItem,
    doc: &'a SetDocument,
    surface: &'a m::SurfaceInfo,
    viewport: ui::Size,
    now: Duration,
    next: u64,
    nodes: usize,
    text: usize,
    identities: BTreeMap<ui::WidgetId, Identity>,
    symbols: &'a mut ui::SymbolCatalog,
    timelines: Timelines,
    seen_motion: BTreeSet<(m::WidgetId, Option<String>, u64)>,
    seen_effect: BTreeSet<(m::WidgetId, Option<String>, u64)>,
    programs: BTreeSet<String>,
    canvas_budget: crate::canvas::Budget,
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn build(
    item: &SceneItem,
    doc: &SetDocument,
    root: &LayoutNode,
    spec: &WidgetSpec,
    surface: &m::SurfaceInfo,
    viewport: ui::Size,
    now: Duration,
    timelines: Timelines,
    symbols: &mut ui::SymbolCatalog,
) -> Result<Built> {
    doc.file.validate()?;
    let mut builder = Builder {
        item,
        doc,
        surface,
        viewport,
        now,
        next: 0,
        nodes: 0,
        text: 0,
        identities: BTreeMap::new(),
        symbols,
        timelines,
        seen_motion: BTreeSet::new(),
        seen_effect: BTreeSet::new(),
        programs: BTreeSet::new(),
        canvas_budget: crate::canvas::Budget::default(),
    };
    let root = builder.node(root, spec, None, 1)?;
    ensure!(node_count(&root) <= 512, "expanded UI node budget exceeded");
    builder
        .timelines
        .motions
        .retain(|k, _| builder.seen_motion.contains(k));
    builder
        .timelines
        .effects
        .retain(|k, _| builder.seen_effect.contains(k));
    Ok(Built {
        root,
        identities: builder.identities,
        timelines: builder.timelines,
        warnings: builder.symbols.take_warnings(),
    })
}
impl Builder<'_> {
    fn spend(&mut self, count: usize) -> Result<()> {
        self.nodes += count;
        ensure!(self.nodes <= 512, "expanded UI node budget exceeded");
        Ok(())
    }
    fn text(&mut self, text: String) -> Result<String> {
        self.text += text.len();
        ensure!(self.text <= 16 * 1024, "rendered text budget exceeded");
        Ok(text)
    }
    fn px(&self, length: m::Length) -> f32 {
        length.to_px(self.surface, self.item.units)
    }
    fn unit(&self, value: f32) -> f32 {
        self.px(m::Length::Unit(value))
    }
    fn state(&self, name: &str) -> Result<&StateValue> {
        self.item
            .state
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
            .with_context(|| format!("missing state {name}"))
    }
    fn scalar<'a>(&'a self, name: &str, record: Option<Record<'a>>) -> Result<&'a Scalar> {
        if let Some(name) = name.strip_prefix("record.") {
            return record
                .and_then(|r| r.fields.get(name))
                .context("missing record scalar");
        }
        match self.state(name)? {
            StateValue::Scalar(v) => Ok(v),
            _ => bail!("binding {name} is not scalar"),
        }
    }
    fn boolean(
        &self,
        path: &Option<String>,
        default: bool,
        record: Option<Record<'_>>,
    ) -> Result<bool> {
        match path {
            None => Ok(default),
            Some(path) => match self.scalar(path, record)? {
                Scalar::Boolean(v) => Ok(*v),
                _ => bail!("binding {path} is not boolean"),
            },
        }
    }
    fn string(
        &self,
        literal: &Option<String>,
        path: &Option<String>,
        record: Option<Record<'_>>,
    ) -> Result<Option<String>> {
        match path {
            None => Ok(literal.clone()),
            Some(path) => match self.scalar(path, record)? {
                Scalar::Text(v) => Ok(Some(v.clone())),
                _ => bail!("binding {path} is not text"),
            },
        }
    }
    #[allow(clippy::cast_possible_truncation)]
    fn number(
        &self,
        literal: Option<f64>,
        path: &Option<String>,
        record: Option<Record<'_>>,
    ) -> Result<f32> {
        let value = match path {
            None => literal.unwrap_or(0.0),
            Some(path) => match self.scalar(path, record)? {
                Scalar::Number(v) => *v,
                _ => bail!("binding {path} is not number"),
            },
        };
        let value = value as f32;
        ensure!(value.is_finite(), "non-finite rendered numeric value");
        Ok(value)
    }
    fn role(
        &self,
        literal: Option<touchbar_sets::ColorRole>,
        path: &Option<String>,
        default: ColorRole,
        record: Option<Record<'_>>,
    ) -> Result<ColorRole> {
        let configured = if path.is_some() {
            let name = self.string(&None, path, record)?.context("missing role")?;
            Some(serde_json::from_value::<touchbar_sets::ColorRole>(
                serde_json::Value::String(name),
            )?)
        } else {
            literal
        };
        Ok(configured.map_or(default, config_role))
    }
    fn flex(&self, node: &LayoutNode, column: bool) -> Result<Flex> {
        let size = if column { node.height } else { node.width }.map(|v| self.px(v));
        let min = node.minimum.map(|v| self.px(v));
        let max = node.maximum.map(|v| self.px(v));
        if let (Some(a), Some(b)) = (min, max) {
            ensure!(a <= b, "minimum exceeds maximum after conversion");
        }
        Ok(match node.kind {
            Kind::FixedSpace => Flex::fixed(size.or(min).unwrap_or(0.0)),
            Kind::FlexibleSpace => Flex::flexible(
                min.unwrap_or(0.0),
                size.unwrap_or(0.0),
                max.unwrap_or(8192.0),
            )
            .grow(node.grow.unwrap_or(1.0)),
            _ if size.is_some() && min.is_none() && max.is_none() && node.grow.is_none() => {
                Flex::fixed(size.unwrap())
            }
            _ if size.is_some() => {
                Flex::flexible(min.unwrap_or(0.0), size.unwrap(), max.unwrap_or(8192.0))
                    .grow(node.grow.unwrap_or(1.0))
            }
            _ => Flex::content(min.unwrap_or(self.unit(100.0)), max.unwrap_or(8192.0))
                .grow(node.grow.unwrap_or(0.0)),
        })
    }
    fn interaction(
        &self,
        spec: &WidgetSpec,
        record: Option<Record<'_>>,
    ) -> Option<&touchbar_core::WidgetInteraction> {
        self.item
            .interactions
            .iter()
            .find(|i| i.widget == spec.id && i.record_key.as_deref() == record.map(|r| r.key))
    }
    fn node(
        &mut self,
        node: &LayoutNode,
        spec: &WidgetSpec,
        record: Option<Record<'_>>,
        depth: usize,
    ) -> Result<Node> {
        self.spend(1)?;
        ensure!(depth <= 32, "rendered tree depth exceeded");
        if !self.boolean(&node.visible_from, true, record)? {
            return Ok(Node::Empty);
        }
        self.next += 1;
        let id = ui::WidgetId(self.next);
        self.identities.insert(
            id,
            Identity {
                widget: spec.id.clone(),
                record: record.map(|r| r.key.into()),
                scrubber: None,
            },
        );
        let label = self
            .string(&node.label, &node.label_from, record)?
            .unwrap_or_default();
        let label = self.text(label)?;
        let color = self.role(node.color, &node.color_from, ColorRole::Foreground, record)?;
        let enabled = self.boolean(&node.enabled_from, true, record)?;
        let mut rendered = match node.kind {
            Kind::Row | Kind::Column | Kind::Stack => {
                let mut children = Vec::new();
                for (n, s) in node.children.iter().zip(&spec.children) {
                    if !self.boolean(&n.visible_from, true, record)? {
                        continue;
                    }
                    let flex = self.flex(n, node.kind == Kind::Column)?;
                    children.push(FlexItem::new(flex, self.node(n, s, record, depth + 1)?));
                }
                match node.kind {
                    Kind::Row => Node::row(self.px(node.spacing), self.px(node.padding), children),
                    Kind::Column => {
                        Node::column(self.px(node.spacing), self.px(node.padding), children)
                    }
                    _ => Node::Layer(children.into_iter().map(|c| c.node).collect()),
                }
            }
            Kind::Repeat | Kind::Scrubber => {
                let source = node
                    .repeat_from
                    .as_deref()
                    .context("record source absent")?;
                let StateValue::Records(records) = self.state(source)? else {
                    bail!("binding {source} is not records")
                };
                // Clone only the bounded record list, allowing tree construction to spend budgets.
                let records = records.clone();
                let template = node.template.as_deref().context("template absent")?;
                let template_spec = spec
                    .children
                    .get(node.children.len() + node.variants.len())
                    .context("template WidgetSpec absent")?;
                let field = node.repeat_key.as_deref().context("record key absent")?;
                ensure!(records.len() <= 512, "record expansion exceeds node budget");
                let mut keys = BTreeSet::new();
                let mut children = Vec::new();
                let mut cells = Vec::new();
                let padding =
                    crate::sizing::pixels(node.padding, self.surface, self.item.units)? as f32;
                let spacing =
                    crate::sizing::pixels(node.spacing, self.surface, self.item.units)? as f32;
                let mut x = padding;
                for fields in &records {
                    let Some(Scalar::Text(key)) = fields.get(field) else {
                        bail!("record key must be text")
                    };
                    ensure!(
                        !key.is_empty() && key.len() <= 128 && keys.insert(key.clone()),
                        "invalid or duplicate record key"
                    );
                    let record = Some(Record { fields, key });
                    if !self.boolean(&template.visible_from, true, record)? {
                        continue;
                    }
                    let child = self.node(template, template_spec, record, depth + 1)?;
                    if node.kind == Kind::Repeat {
                        children.push(FlexItem::new(self.flex(template, false)?, child));
                    } else {
                        let width = crate::sizing::pixels(
                            template.width.unwrap_or(m::Length::Unit(100.0)),
                            self.surface,
                            self.item.units,
                        )? as f32;
                        if width == 0.0 {
                            continue;
                        }
                        let interaction = self.interaction(spec, None);
                        let offset = interaction.map_or(0.0, |i| i.scrubber.offset_px);
                        let selected = interaction
                            .is_some_and(|i| i.scrubber.selected.as_deref() == Some(key));
                        let highlighted = interaction
                            .is_some_and(|i| i.scrubber.highlighted.as_deref() == Some(key));
                        let child = if selected || highlighted {
                            Node::Panel {
                                radius: self.unit(10.0),
                                color: if selected {
                                    ColorRole::Accent
                                } else {
                                    ColorRole::ControlPressed
                                },
                                padding: 0.0,
                                child: Box::new(child),
                            }
                        } else {
                            child
                        };
                        cells.push(ScrubberItemGeometry {
                            record_key: key.clone(),
                            rect: m::Rect {
                                x,
                                y: 0.0,
                                width,
                                height: self.viewport.height,
                            },
                        });
                        children.push(FlexItem::new(
                            Flex::fixed(width),
                            Node::positioned(
                                ui::Rect::new(x - offset, 0.0, width, self.viewport.height),
                                child,
                            ),
                        ));
                        x += width + spacing;
                    }
                }
                if node.kind == Kind::Repeat {
                    Node::row(self.px(node.spacing), self.px(node.padding), children)
                } else {
                    let content = if cells.is_empty() {
                        0.0
                    } else {
                        x - spacing + padding
                    };
                    self.identities.get_mut(&id).unwrap().scrubber = Some((content, cells));
                    let child = Node::Layer(children.into_iter().map(|c| c.node).collect());
                    if enabled {
                        Node::Pressable {
                            id,
                            label: label.clone(),
                            pressed: false,
                            hold: None,
                            selected: None,
                            style: PressableStyle::plain(),
                            child: Box::new(child),
                        }
                    } else {
                        child
                    }
                }
            }
            Kind::Button | Kind::Key => {
                let child = if let Some(child) = node.children.first() {
                    self.node(child, &spec.children[0], record, depth + 1)?
                } else if node.kind == Kind::Key {
                    Node::label(key_label(node.key.context("key absent")?), self.unit(25.0))
                } else {
                    self.content(node, &label, color, record)?
                };
                if enabled {
                    Node::Pressable {
                        id,
                        label: label.clone(),
                        pressed: self.interaction(spec, record).is_some_and(|i| i.pressed)
                            || self.boolean(&node.pressed_from, false, record)?,
                        hold: node.hold_ms.map(Duration::from_millis),
                        selected: Some(self.boolean(&node.selected_from, false, record)?),
                        style: PressableStyle {
                            background: (!node.plain).then_some(self.role(
                                node.background,
                                &node.background_from,
                                ColorRole::Control,
                                record,
                            )?),
                            pressed_background: (!node.plain).then_some(self.role(
                                node.pressed_background,
                                &node.pressed_background_from,
                                ColorRole::ControlPressed,
                                record,
                            )?),
                            corner_radius: node.corner_radius.map(|v| self.px(v)),
                            padding: self.px(node.padding),
                            minimum_size: ui::Size::new(0.0, 0.0),
                            content_foreground: Some(color),
                        },
                        child: Box::new(child),
                    }
                } else {
                    child
                }
            }
            Kind::Text | Kind::Icon | Kind::Image => self.content(node, &label, color, record)?,
            Kind::Slider | Kind::Meter | Kind::Progress => {
                let value = if let Some(v) = self.interaction(spec, record).and_then(|i| i.value) {
                    self.number(Some(v), &None, record)?
                } else {
                    self.number(node.value, &node.value_from, record)?
                };
                match node.kind {
                    Kind::Slider if enabled => Node::slider(id, label.clone(), value),
                    Kind::Progress => Node::progress(label.clone(), value),
                    _ => Node::meter(
                        label.clone(),
                        ui::ContinuousValue::unit(value),
                        None,
                        ui::MeterStyle::default(),
                    ),
                }
            }
            Kind::FixedSpace | Kind::FlexibleSpace => Node::Empty,
            Kind::Responsive => {
                let mut variants = vec![];
                for (i, v) in node.variants.iter().enumerate() {
                    variants.push(ui::ResponsiveVariant::new(
                        match v.representation.as_str() {
                            "hidden" => ui::Representation::Hidden,
                            "minimal" => ui::Representation::Minimal,
                            "compact" => ui::Representation::Compact,
                            "full" => ui::Representation::Full,
                            _ => bail!("invalid representation"),
                        },
                        self.px(v.minimum_width),
                        self.node(
                            &v.layout,
                            &spec.children[node.children.len() + i],
                            record,
                            depth + 1,
                        )?,
                    ));
                }
                Node::responsive(id, variants)
            }
            Kind::KeyRow => {
                let mut keys = vec![];
                for (key, spec) in node.keys.iter().zip(&spec.children) {
                    self.spend(1)?;
                    self.next += 1;
                    let id = ui::WidgetId(self.next);
                    self.identities.insert(
                        id,
                        Identity {
                            widget: spec.id.clone(),
                            record: None,
                            scrubber: None,
                        },
                    );
                    let child = Node::label(key_label(key.key), self.unit(25.0));
                    let child = if enabled {
                        Node::Pressable {
                            id,
                            label: key_label(key.key),
                            pressed: self.interaction(spec, None).is_some_and(|i| i.pressed),
                            hold: None,
                            selected: None,
                            style: PressableStyle {
                                padding: self.unit(5.0),
                                minimum_size: ui::Size::new(0.0, 0.0),
                                ..PressableStyle::control()
                            },
                            child: Box::new(child),
                        }
                    } else {
                        child
                    };
                    keys.push(FlexItem::new(
                        key.width.map_or_else(
                            || Flex::flexible(0.0, self.unit(100.0), 8192.0).grow(1.0),
                            |w| Flex::fixed(self.px(w)),
                        ),
                        child,
                    ));
                }
                Node::row(self.px(node.spacing), self.px(node.padding), keys)
            }
            Kind::Canvas => {
                let StateValue::Canvas(commands) = self.state(
                    node.canvas_from
                        .as_deref()
                        .context("canvas source absent")?,
                )?
                else {
                    bail!("canvas binding is not Canvas")
                };
                let commands = commands.clone();
                let scale = self.unit(1.0);
                let commands = crate::canvas::convert(&commands, &mut self.canvas_budget, scale)?;
                for command in &commands {
                    if let ui::CanvasCommand::Text { text, .. } = command {
                        self.text(text.clone())?;
                    }
                }
                Node::canvas(
                    label.clone(),
                    ui::Size::new(
                        self.viewport.width / self.unit(1.0),
                        self.viewport.height / self.unit(1.0),
                    ),
                    commands,
                )
            }
            Kind::ShaderEffect => {
                let effect = node.effect.as_ref().context("effect absent")?;
                let key = (spec.id.clone(), record.map(|r| r.key.into()), effect.id);
                ensure!(
                    self.seen_effect.insert(key.clone()) && self.seen_effect.len() <= 4,
                    "effect ID or node budget exceeded"
                );
                self.programs.insert(effect.source.clone());
                ensure!(self.programs.len() <= 8, "effect program budget exceeded");
                let period = effect.period_ms.map(|p| Duration::from_millis(p.into()));
                let old = self.timelines.effects.get(&key);
                let (program, started) = if let Some(old) = old.filter(|old| {
                    old.program.body().as_ref() == effect.source.as_str() && old.period == period
                }) {
                    (old.program.clone(), old.started)
                } else {
                    (
                        Arc::new(ui::EffectProgram::compile(&effect.source)?),
                        self.now,
                    )
                };
                let mut parameters = [0.0; 8];
                for (p, v) in parameters.iter_mut().zip(&effect.parameters) {
                    *p = self.bound_scalar(Some(v), 0.0, -1_000_000.0, 1_000_000.0, record)?;
                }
                let effect = ui::ShaderEffect {
                    program,
                    parameters,
                    started,
                    period,
                    opacity: effect.opacity.unwrap_or(1.0),
                };
                self.timelines.effects.insert(key, effect.clone());
                Node::shader_effect(label, self.viewport, effect)
            }
        };
        if !enabled {
            rendered = Node::opacity(node.disabled_opacity.unwrap_or(0.35), rendered);
        }
        if let Some(def) = &node.motion {
            self.spend(1)?;
            let key = (spec.id.clone(), record.map(|r| r.key.into()), def.id);
            ensure!(
                self.seen_motion.insert(key.clone()) && self.seen_motion.len() <= 64,
                "motion ID or animation budget exceeded"
            );
            let mut motion = ui::Motion {
                id: ui::MotionId(def.id),
                from: self.transform(&def.from, record)?,
                to: self.transform(&def.to, record)?,
                started: self.now,
                duration: Duration::from_millis(def.duration_ms.into()),
                easing: if def.easing == "linear" {
                    ui::Easing::Linear
                } else {
                    ui::Easing::EaseInOut
                },
                playback: match def.playback.as_str() {
                    "loop" => ui::MotionPlayback::Loop,
                    "alternate" => ui::MotionPlayback::Alternate,
                    _ => ui::MotionPlayback::Once,
                },
            };
            if let Some(previous) = self.timelines.motions.get(&key) {
                let mut previous = *previous;
                previous.started = self.now;
                if previous == motion {
                    motion.started = self.timelines.motions[&key].started;
                }
            }
            self.timelines.motions.insert(key, motion);
            rendered = Node::motion(motion, rendered);
        }
        if let Some(opacity) = node.opacity {
            self.spend(1)?;
            rendered = Node::opacity(opacity, rendered);
        }
        Ok(rendered)
    }
    fn content(
        &mut self,
        node: &LayoutNode,
        label: &str,
        color: ColorRole,
        record: Option<Record<'_>>,
    ) -> Result<Node> {
        let size = node.font_size.map_or(self.unit(26.666667), |l| self.px(l));
        ensure!(
            size.is_finite() && size > 0.0 && size <= 512.0,
            "rendered font size exceeds budget"
        );
        let text = self
            .string(&node.text, &node.text_from, record)?
            .unwrap_or_else(|| label.into());
        let text = self.text(text)?;
        let symbol = self.string(&node.symbol, &node.symbol_from, record)?;
        let asset = self.string(&node.asset, &node.asset_from, record)?;
        let visual = if let Some(symbol) = symbol {
            let symbol = match symbol.as_str() {
                "chevron.left" => "chevron-left",
                "chevron.right" => "chevron-right",
                s => s,
            };
            let edge =
                crate::sizing::pixels(m::Length::Unit(53.333333), self.surface, self.item.units)?
                    .clamp(1, 8192);
            let image = self.symbols.named_image(symbol, edge)?;
            Some(Node::Image {
                image,
                opacity: 1.0,
                fit: ui::ImageFit::Contain,
                tint: ui::ImageTint::Mask(color),
                label: label.into(),
            })
        } else if let Some(asset) = asset {
            Some(Node::Image {
                image: self
                    .doc
                    .assets
                    .get(&asset)
                    .with_context(|| format!("missing ready image {asset}"))?
                    .clone(),
                opacity: 1.0,
                fit: ui::ImageFit::Contain,
                tint: if self.doc.asset_kinds.get(&asset) == Some(&touchbar_sets::AssetKind::Png) {
                    ui::ImageTint::None
                } else {
                    ui::ImageTint::Mask(color)
                },
                label: label.into(),
            })
        } else {
            None
        };
        let text = Node::Label {
            text,
            size,
            color,
            align: ui::TextAlign::Center,
            overflow: ui::TextOverflow::Ellipsis,
            measurement: ui::TextMeasurement::Content,
        };
        Ok(match (node.kind, visual) {
            (Kind::Text, _) => text,
            (Kind::Icon | Kind::Image, Some(visual)) => visual,
            (_, Some(visual)) => Node::row(
                self.unit(6.666667),
                0.0,
                vec![
                    FlexItem::new(Flex::fixed(self.unit(40.0)), visual),
                    FlexItem::new(
                        Flex::flexible(0.0, self.unit(100.0), 8192.0).grow(1.0),
                        text,
                    ),
                ],
            ),
            _ => text,
        })
    }
    fn bound_scalar(
        &self,
        value: Option<&ScalarBinding>,
        default: f32,
        min: f32,
        max: f32,
        record: Option<Record<'_>>,
    ) -> Result<f32> {
        let value = match value {
            None => default,
            Some(ScalarBinding::Number(v)) => self.number(Some(*v), &None, record)?,
            Some(ScalarBinding::State(p)) => self.number(None, &Some(p.clone()), record)?,
        };
        ensure!(
            (min..=max).contains(&value),
            "motion/effect scalar outside bounds"
        );
        Ok(value)
    }
    fn transform(
        &self,
        t: &TransformDefinition,
        record: Option<Record<'_>>,
    ) -> Result<ui::VisualTransform> {
        Ok(ui::VisualTransform {
            translation: ui::Point::new(
                self.unit(self.bound_scalar(t.x.as_ref(), 0.0, -10_000.0, 10_000.0, record)?),
                self.unit(self.bound_scalar(t.y.as_ref(), 0.0, -10_000.0, 10_000.0, record)?),
            ),
            scale: self.bound_scalar(t.scale.as_ref(), 1.0, 0.0, 100.0, record)?,
            opacity: self.bound_scalar(t.opacity.as_ref(), 1.0, 0.0, 1.0, record)?,
        })
    }
}
fn config_role(role: touchbar_sets::ColorRole) -> ColorRole {
    match role {
        touchbar_sets::ColorRole::Background => ColorRole::Background,
        touchbar_sets::ColorRole::Control => ColorRole::Control,
        touchbar_sets::ColorRole::ControlPressed => ColorRole::ControlPressed,
        touchbar_sets::ColorRole::Track => ColorRole::Track,
        touchbar_sets::ColorRole::Foreground => ColorRole::Foreground,
        touchbar_sets::ColorRole::Muted => ColorRole::Muted,
        touchbar_sets::ColorRole::Accent => ColorRole::Accent,
        touchbar_sets::ColorRole::Destructive => ColorRole::Destructive,
        touchbar_sets::ColorRole::OnAccent => ColorRole::OnAccent,
        touchbar_sets::ColorRole::OnDestructive => ColorRole::OnDestructive,
    }
}

fn key_label(key: m::SystemKey) -> String {
    match key {
        m::SystemKey::Esc => "Esc",
        m::SystemKey::F1 => "F1",
        m::SystemKey::F2 => "F2",
        m::SystemKey::F3 => "F3",
        m::SystemKey::F4 => "F4",
        m::SystemKey::F5 => "F5",
        m::SystemKey::F6 => "F6",
        m::SystemKey::F7 => "F7",
        m::SystemKey::F8 => "F8",
        m::SystemKey::F9 => "F9",
        m::SystemKey::F10 => "F10",
        m::SystemKey::F11 => "F11",
        m::SystemKey::F12 => "F12",
        m::SystemKey::BrightnessDown => "Brightness −",
        m::SystemKey::BrightnessUp => "Brightness +",
        m::SystemKey::KeyboardIlluminationDown => "Keyboard −",
        m::SystemKey::KeyboardIlluminationUp => "Keyboard +",
        m::SystemKey::PreviousSong => "Previous",
        m::SystemKey::PlayPause => "Play/Pause",
        m::SystemKey::NextSong => "Next",
        m::SystemKey::Mute => "Mute",
        m::SystemKey::VolumeDown => "Volume −",
        m::SystemKey::VolumeUp => "Volume +",
        m::SystemKey::MicrophoneMute => "Microphone",
        m::SystemKey::Search => "Search",
    }
    .into()
}

fn node_count(node: &Node) -> usize {
    1 + match node {
        Node::Layer(children) => children.iter().map(node_count).sum::<usize>(),
        Node::Row { children, .. } | Node::Column { children, .. } => {
            children.iter().map(|c| node_count(&c.node)).sum()
        }
        Node::Positioned { child, .. }
        | Node::Panel { child, .. }
        | Node::Pressable { child, .. }
        | Node::Opacity { child, .. }
        | Node::Motion { child, .. } => node_count(child),
        Node::Responsive { variants, .. } => variants.iter().map(|v| node_count(&v.node)).sum(),
        _ => 0,
    }
}
