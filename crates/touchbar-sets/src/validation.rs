use crate::{document::error, *};
use std::{
    collections::BTreeSet,
    path::{Component, Path},
};
use touchbar_model::{ActionKind, Length, PanelId, SetRef, WidgetId};
const MAX_NODES: usize = 256;
const MAX_DEPTH: usize = 32;
const MAX_TEXT: usize = 16 * 1024;
type Result<T> = std::result::Result<T, DocumentError>;
fn check(ok: bool, path: &str, msg: &str) -> Result<()> {
    if ok { Ok(()) } else { Err(error(path, msg)) }
}
fn field(path: &str, name: &str) -> String {
    format!("{path}.{name}")
}
fn name(value: &str, path: &str) -> Result<()> {
    check(
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            && !value.contains(".."),
        path,
        "invalid identifier",
    )
}
fn reference(value: &str, path: &str) -> Result<()> {
    check(
        value.len() <= 128 && format!("p/{value}").parse::<SetRef>().is_ok(),
        path,
        "invalid identifier",
    )
}
fn widget_id(value: &str, path: &str) -> Result<()> {
    check(
        value.len() <= 128 && value.parse::<WidgetId>().is_ok(),
        path,
        "invalid widget identifier",
    )
}
fn relative(value: &str, path: &str) -> Result<()> {
    check(
        !value.is_empty()
            && Path::new(value)
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
        path,
        "expected relative file path without . or ..",
    )
}
fn length(value: Length, path: &str) -> Result<()> {
    value.validate().map_err(|e| error(path, e.0))
}
fn ordered(a: Length, b: Length, path: &str) -> Result<()> {
    let order = match (a, b) {
        (Length::Unit(a), Length::Unit(b)) => Some(a <= b),
        (Length::Percent { value: a, axis: x }, Length::Percent { value: b, axis: y })
            if x == y =>
        {
            Some(a <= b)
        }
        _ => None,
    };
    check(
        order != Some(false),
        path,
        "minimum exceeds maximum for matching units",
    )
}
#[derive(Default)]
struct Budget {
    nodes: usize,
    text: usize,
    ids: BTreeSet<String>,
}
impl Budget {
    fn id(&mut self, id: &str, path: &str) -> Result<()> {
        widget_id(id, path)?;
        check(self.ids.insert(id.into()), path, "duplicate widget id")
    }
    fn text(&mut self, text: &str, path: &str) -> Result<()> {
        self.text += text.len();
        check(self.text <= MAX_TEXT, path, "layout exceeds text budget")
    }
}
impl PluginManifest {
    pub fn validate(&self) -> Result<()> {
        check(self.api == "1", "api", "unsupported api; expected 1")?;
        check(self.id.as_str().len() <= 128, "id", "identifier too long")?;
        check(
            !self.version.trim().is_empty(),
            "version",
            "version cannot be empty",
        )?;
        check(!self.name.trim().is_empty(), "name", "name cannot be empty")?;
        if let Some(logic) = &self.logic {
            relative(logic, "logic")?;
        }
        let mut ids = BTreeSet::new();
        let mut files = BTreeSet::new();
        for (i, set) in self.sets.iter().enumerate() {
            let p = format!("sets[{i}]");
            reference(&set.id, &field(&p, "id"))?;
            relative(&set.file, &field(&p, "file"))?;
            check(ids.insert(&set.id), &field(&p, "id"), "duplicate set id")?;
            check(
                files.insert(&set.file),
                &field(&p, "file"),
                "duplicate set file",
            )?;
        }
        let mut ids = BTreeSet::new();
        let mut files = BTreeSet::new();
        for (i, profile) in self.profiles.iter().enumerate() {
            let p = format!("profiles[{i}]");
            reference(&profile.id, &field(&p, "id"))?;
            relative(&profile.file, &field(&p, "file"))?;
            check(
                ids.insert(&profile.id),
                &field(&p, "id"),
                "duplicate profile id",
            )?;
            check(
                files.insert(&profile.file),
                &field(&p, "file"),
                "duplicate profile file",
            )?;
        }
        check(self.assets.len() <= 256, "assets", "too many assets")?;
        let mut ids = BTreeSet::new();
        let mut pixels = 0u64;
        for (i, asset) in self.assets.iter().enumerate() {
            let p = format!("assets[{i}]");
            reference(&asset.id, &field(&p, "id"))?;
            relative(&asset.file, &field(&p, "file"))?;
            check(
                ids.insert(&asset.id),
                &field(&p, "id"),
                "duplicate asset id",
            )?;
            pixels += u64::from(asset.width) * u64::from(asset.height);
            check(
                asset.width > 0 && asset.height > 0 && asset.width <= 2048 && asset.height <= 2048,
                &p,
                "raster dimensions must be 1..=2048 pixels",
            )?;
            check(
                pixels <= 16 * 1024 * 1024,
                "assets",
                "asset rasters exceed 16M pixels per plugin",
            )?;
        }
        let mut requires = BTreeSet::new();
        for (i, id) in self.requires.iter().enumerate() {
            check(
                id != &self.id && requires.insert(id),
                &format!("requires[{i}]"),
                "duplicate or self dependency",
            )?;
        }
        Ok(())
    }
}
impl SetFile {
    pub fn validate_assets(&self, ids: &BTreeSet<&str>) -> Result<()> {
        let mut check = |node: &LayoutNode, path: &str| {
            if let Some(asset) = &node.asset {
                check(
                    ids.contains(asset.as_str()),
                    &field(path, "asset"),
                    "undeclared asset",
                )?;
            }
            Ok(())
        };
        self.layout.walk("layout", &mut check)?;
        for panel in &self.panels {
            panel.layout.walk("panel", &mut check)?;
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        check(self.version == 1, "version", "unsupported set version")?;
        reference(&self.id, "id")?;
        check(
            !self.label.trim().is_empty(),
            "label",
            "label cannot be empty",
        )?;
        check(
            self.label.len() <= MAX_TEXT,
            "label",
            "label exceeds text budget",
        )?;
        check(
            touchbar_logic_types::wire_size_init(&self.logic_config, &vec![])
                <= touchbar_logic_types::LogicLimits::default().max_update_bytes,
            "logic_config",
            "configuration too large",
        )?;
        check(
            self.persist.len() <= 256,
            "persist",
            "too many persisted fields",
        )?;
        let mut fields = BTreeSet::new();
        for (i, n) in self.persist.iter().enumerate() {
            let p = format!("persist[{i}]");
            name(n, &p)?;
            check(fields.insert(n), &p, "duplicate persisted field")?;
        }
        if let Some(units) = self.units {
            units.u.validate().map_err(|e| error("units.u", e.0))?;
        }
        check(self.panels.len() <= 64, "panels", "too many panels")?;
        let mut budget = Budget::default();
        let mut panels = BTreeSet::new();
        // Validate the owner before panel references, so missing hold_ms is diagnosed first.
        validate_node(&self.layout, "layout", 0, false, &mut budget)?;
        for (i, panel) in self.panels.iter().enumerate() {
            let p = format!("panels[{i}]");
            check(
                panel.id.len() <= 128 && panel.id.parse::<PanelId>().is_ok(),
                &field(&p, "id"),
                "invalid panel identifier",
            )?;
            check(
                panels.insert(panel.id.as_str()),
                &field(&p, "id"),
                "duplicate panel id",
            )?;
            for (n, v) in [
                ("minimum_width", panel.minimum_width),
                ("preferred_width", panel.preferred_width),
                ("maximum_width", panel.maximum_width),
            ] {
                length(v, &field(&p, n))?;
            }
            ordered(
                panel.minimum_width,
                panel.preferred_width,
                &field(&p, "minimum_width"),
            )?;
            ordered(
                panel.preferred_width,
                panel.maximum_width,
                &field(&p, "preferred_width"),
            )?;
            let previous = budget.ids.clone();
            validate_node(&panel.layout, &field(&p, "layout"), 0, false, &mut budget)?;
            if let Some(id) = &panel.principal_item {
                check(
                    budget.ids.contains(id) && !previous.contains(id),
                    &field(&p, "principal_item"),
                    "principal widget must belong to this panel",
                )?;
            }
        }
        self.layout
            .walk("layout", &mut |node, path| panel_refs(node, path, &panels))?;
        for (i, panel) in self.panels.iter().enumerate() {
            panel
                .layout
                .walk(&format!("panels[{i}].layout"), &mut |node, path| {
                    panel_refs(node, path, &panels)
                })?;
        }
        Ok(())
    }
}
fn panel_refs(node: &LayoutNode, path: &str, panels: &BTreeSet<&str>) -> Result<()> {
    for (n, id) in [
        ("expanded_panel", &node.expanded_panel),
        ("press_and_hold_panel", &node.press_and_hold_panel),
    ] {
        if let Some(id) = id {
            check(
                panels.contains(id.as_str()),
                &field(path, n),
                "unknown panel reference",
            )?;
        }
    }
    Ok(())
}
impl LayoutNode {
    pub fn actions(&self) -> impl Iterator<Item = (&'static str, &str, ActionKind)> {
        [
            ("on_pressed", &self.on_pressed, ActionKind::Pressed),
            ("on_activate", &self.on_activate, ActionKind::Activated),
            (
                "on_long_pressed",
                &self.on_long_pressed,
                ActionKind::LongPressed,
            ),
            (
                "on_value_changed",
                &self.on_value_changed,
                ActionKind::ValueChanged,
            ),
            ("on_released", &self.on_released, ActionKind::Released),
            ("on_selected", &self.on_selected, ActionKind::Selected),
            ("on_cancelled", &self.on_cancelled, ActionKind::Cancelled),
        ]
        .into_iter()
        .filter_map(|(n, a, k)| a.as_deref().map(|a| (n, a, k)))
    }
    pub(crate) fn bindings(&self) -> impl Iterator<Item = (&'static str, &str)> {
        [
            ("label_from", &self.label_from),
            ("text_from", &self.text_from),
            ("symbol_from", &self.symbol_from),
            ("asset_from", &self.asset_from),
            ("color_from", &self.color_from),
            ("background_from", &self.background_from),
            ("pressed_background_from", &self.pressed_background_from),
            ("value_from", &self.value_from),
            ("enabled_from", &self.enabled_from),
            ("visible_from", &self.visible_from),
            ("selected_from", &self.selected_from),
            ("pressed_from", &self.pressed_from),
            ("repeat_from", &self.repeat_from),
            ("canvas_from", &self.canvas_from),
        ]
        .into_iter()
        .filter_map(|(n, b)| b.as_deref().map(|b| (n, b)))
    }
    pub(crate) fn walk(
        &self,
        path: &str,
        f: &mut impl FnMut(&Self, &str) -> Result<()>,
    ) -> Result<()> {
        f(self, path)?;
        for (i, c) in self.children.iter().enumerate() {
            c.walk(&format!("{path}.children[{i}]"), f)?;
        }
        for (i, v) in self.variants.iter().enumerate() {
            v.layout.walk(&format!("{path}.variants[{i}].layout"), f)?;
        }
        if let Some(t) = &self.template {
            t.walk(&field(path, "template"), f)?;
        }
        Ok(())
    }
    pub(crate) fn has_logic(&self) -> bool {
        self.bindings().next().is_some()
            || self.actions().next().is_some()
            || self.scalars().any(|s| matches!(s, ScalarBinding::State(_)))
            || self.children.iter().any(Self::has_logic)
            || self.variants.iter().any(|v| v.layout.has_logic())
            || self.template.as_ref().is_some_and(|t| t.has_logic())
    }
    pub(crate) fn scalars(&self) -> impl Iterator<Item = &ScalarBinding> {
        self.effect.iter().flat_map(|e| e.parameters.iter()).chain(
            self.motion
                .iter()
                .flat_map(|m| [&m.from, &m.to])
                .flat_map(|t| [&t.x, &t.y, &t.scale, &t.opacity].into_iter().flatten()),
        )
    }
}
fn validate_node(
    node: &LayoutNode,
    path: &str,
    depth: usize,
    record: bool,
    budget: &mut Budget,
) -> Result<()> {
    budget.nodes += 1;
    check(
        budget.nodes <= MAX_NODES,
        path,
        "layout exceeds node budget",
    )?;
    check(depth <= MAX_DEPTH, path, "layout exceeds depth budget")?;
    if let Some(id) = &node.id {
        budget.id(id, &field(path, "id"))?;
    }
    for (n, l) in [
        ("spacing", Some(node.spacing)),
        ("padding", Some(node.padding)),
        ("width", node.width),
        ("height", node.height),
        ("minimum", node.minimum),
        ("maximum", node.maximum),
        ("corner_radius", node.corner_radius),
        ("font_size", node.font_size),
    ] {
        if let Some(l) = l {
            length(l, &field(path, n))?;
        }
    }
    if let (Some(a), Some(b)) = (node.minimum, node.maximum) {
        ordered(a, b, &field(path, "minimum"))?;
    }
    if let Some(g) = node.grow {
        check(
            g.is_finite() && (0.0..=8192.0).contains(&g),
            &field(path, "grow"),
            "grow must be finite and nonnegative",
        )?;
    }
    for (n, v) in [
        ("opacity", node.opacity),
        ("disabled_opacity", node.disabled_opacity),
    ] {
        if let Some(v) = v {
            check(
                v.is_finite() && (0.0..=1.0).contains(&v),
                &field(path, n),
                "invalid opacity",
            )?;
        }
    }
    if let Some(v) = node.value {
        check(v.is_finite(), &field(path, "value"), "value must be finite")?;
    }
    if let Some(symbol) = &node.symbol {
        check(
            matches!(
                symbol.as_str(),
                "volume"
                    | "muted"
                    | "play"
                    | "pause"
                    | "previous"
                    | "next"
                    | "microphone"
                    | "brightness"
                    | "battery"
                    | "wifi"
                    | "bluetooth"
                    | "workspace"
                    | "capture"
                    | "timer"
                    | "terminal"
                    | "graph"
                    | "check"
                    | "chevron.left"
                    | "chevron.right"
            ),
            &field(path, "symbol"),
            "unknown built-in symbol",
        )?;
    }
    if let Some(asset) = &node.asset {
        name(asset, &field(path, "asset"))?;
    }
    for (n, text) in [("label", &node.label), ("text", &node.text)] {
        if let Some(t) = text {
            budget.text(t, &field(path, n))?;
        }
    }
    for (literal, binding, n) in [
        (
            node.label.is_some(),
            node.label_from.is_some(),
            "label_from",
        ),
        (node.text.is_some(), node.text_from.is_some(), "text_from"),
        (
            node.symbol.is_some(),
            node.symbol_from.is_some(),
            "symbol_from",
        ),
        (
            node.asset.is_some(),
            node.asset_from.is_some(),
            "asset_from",
        ),
        (
            node.value.is_some(),
            node.value_from.is_some(),
            "value_from",
        ),
        (
            node.color.is_some(),
            node.color_from.is_some(),
            "color_from",
        ),
        (
            node.background.is_some(),
            node.background_from.is_some(),
            "background_from",
        ),
        (
            node.pressed_background.is_some(),
            node.pressed_background_from.is_some(),
            "pressed_background_from",
        ),
    ] {
        check(
            !(literal && binding),
            &field(path, n),
            "property has both literal and state binding",
        )?;
    }
    let interactive = matches!(
        node.kind,
        LayoutKind::Button | LayoutKind::Slider | LayoutKind::Scrubber
    );
    for (n, a, k) in node.actions() {
        check(
            interactive,
            &field(path, n),
            "actions require an interactive node",
        )?;
        check(
            node.id.is_some(),
            &field(path, "id"),
            "actions require an explicit id",
        )?;
        if k == ActionKind::ValueChanged {
            check(
                matches!(node.kind, LayoutKind::Slider | LayoutKind::Scrubber),
                &field(path, n),
                "ValueChanged requires slider or scrubber",
            )?;
        }
        if k == ActionKind::Selected {
            check(
                node.kind == LayoutKind::Scrubber,
                &field(path, n),
                "Selected requires scrubber",
            )?;
        }
        name(a, &field(path, n))?;
    }
    if node.expanded_panel.is_some() || node.press_and_hold_panel.is_some() {
        check(
            node.kind == LayoutKind::Button,
            &field(path, "expanded_panel"),
            "panel trigger requires a button",
        )?;
        check(
            node.id.is_some(),
            &field(path, "id"),
            "panel trigger requires an explicit id",
        )?;
    }
    if node.on_long_pressed.is_some() || node.press_and_hold_panel.is_some() {
        check(
            node.hold_ms.is_some(),
            &field(path, "hold_ms"),
            "hold handler or panel requires hold_ms",
        )?;
    }
    if let Some(ms) = node.hold_ms {
        check(
            (1..=60_000).contains(&ms),
            &field(path, "hold_ms"),
            "hold duration outside bounds",
        )?;
    }
    let container = matches!(
        node.kind,
        LayoutKind::Row | LayoutKind::Column | LayoutKind::Stack | LayoutKind::Button
    );
    check(
        (container || node.children.is_empty())
            && (node.kind != LayoutKind::Button || node.children.len() <= 1),
        &field(path, "children"),
        "node does not support these children",
    )?;
    if matches!(node.kind, LayoutKind::Repeat | LayoutKind::Scrubber) {
        for (n, b) in [
            ("repeat_from", node.repeat_from.is_some()),
            ("repeat_key", node.repeat_key.is_some()),
            ("template", node.template.is_some()),
        ] {
            check(
                b,
                &field(path, n),
                "repeat or scrubber requires source, key and template",
            )?;
        }
    } else {
        for (n, b) in [
            ("repeat_from", node.repeat_from.is_some()),
            ("repeat_key", node.repeat_key.is_some()),
            ("template", node.template.is_some()),
        ] {
            check(
                !b,
                &field(path, n),
                "repeat properties require repeat or scrubber node",
            )?;
        }
    }
    if node.kind == LayoutKind::Scrubber {
        check(
            node.value.is_none(),
            &field(path, "value"),
            "scrubber uses records, not scalar value",
        )?;
        check(
            node.value_from.is_none(),
            &field(path, "value_from"),
            "scrubber uses records, not scalar value",
        )?;
        if let Some(template) = &node.template {
            template.walk(&field(path, "template"), &mut |node, path| {
                check(
                    !matches!(
                        node.kind,
                        LayoutKind::Button
                            | LayoutKind::Slider
                            | LayoutKind::Scrubber
                            | LayoutKind::Key
                            | LayoutKind::KeyRow
                    ),
                    path,
                    "scrubber template must be visual; interactions belong to scrubber",
                )
            })?;
        }
    }
    if let Some(k) = &node.repeat_key {
        name(k, &field(path, "repeat_key"))?;
    }
    check(
        node.kind != LayoutKind::Canvas || node.canvas_from.is_some(),
        &field(path, "canvas_from"),
        "canvas requires canvas_from",
    )?;
    check(
        node.canvas_from.is_none() || node.kind == LayoutKind::Canvas,
        &field(path, "canvas_from"),
        "canvas binding requires canvas node",
    )?;
    for (n, b) in node.bindings() {
        if let Some(s) = b.strip_prefix("record.") {
            check(
                record && !matches!(n, "repeat_from" | "canvas_from"),
                &field(path, n),
                "record binding requires repeat template scope",
            )?;
            name(s, &field(path, n))?;
        } else {
            name(b, &field(path, n))?;
        }
    }
    if node.kind == LayoutKind::Responsive {
        check(
            !node.variants.is_empty() && node.variants.len() <= 4,
            &field(path, "variants"),
            "responsive requires 1..4 variants",
        )?;
        let mut seen = BTreeSet::new();
        for (i, v) in node.variants.iter().enumerate() {
            let p = format!("{path}.variants[{i}]");
            check(
                matches!(
                    v.representation.as_str(),
                    "hidden" | "minimal" | "compact" | "full"
                ),
                &field(&p, "representation"),
                "unknown representation",
            )?;
            check(
                seen.insert(&v.representation),
                &field(&p, "representation"),
                "duplicate representation",
            )?;
            length(v.minimum_width, &field(&p, "minimum_width"))?;
            validate_node(&v.layout, &field(&p, "layout"), depth + 1, record, budget)?;
        }
    } else {
        check(
            node.variants.is_empty(),
            &field(path, "variants"),
            "variants require responsive node",
        )?;
    }
    if node.kind == LayoutKind::ShaderEffect {
        check(
            node.effect.is_some(),
            &field(path, "effect"),
            "shader-effect requires effect",
        )?;
    } else {
        check(
            node.effect.is_none(),
            &field(path, "effect"),
            "effect requires shader-effect node",
        )?;
    }
    if let Some(e) = &node.effect {
        let p = field(path, "effect");
        check(e.id > 0, &field(&p, "id"), "effect id must be positive")?;
        check(
            e.source.len() <= 4096,
            &field(&p, "source"),
            "effect source too long",
        )?;
        budget.text(&e.source, &field(&p, "source"))?;
        check(
            e.parameters.len() <= 8,
            &field(&p, "parameters"),
            "too many effect parameters",
        )?;
        if let Some(ms) = e.period_ms {
            check(
                (100..=60_000).contains(&ms),
                &field(&p, "period_ms"),
                "effect period outside bounds",
            )?;
        }
        if let Some(v) = e.opacity {
            check(
                v.is_finite() && (0.0..=1.0).contains(&v),
                &field(&p, "opacity"),
                "invalid effect opacity",
            )?;
        }
    }
    if let Some(m) = &node.motion {
        let p = field(path, "motion");
        check(m.id > 0, &field(&p, "id"), "motion id must be positive")?;
        check(
            (1..=60_000).contains(&m.duration_ms),
            &field(&p, "duration_ms"),
            "motion duration outside bounds",
        )?;
        check(
            matches!(m.easing.as_str(), "linear" | "ease-in-out"),
            &field(&p, "easing"),
            "unknown motion easing",
        )?;
        check(
            matches!(m.playback.as_str(), "once" | "loop" | "alternate"),
            &field(&p, "playback"),
            "unknown motion playback",
        )?;
    }
    for scalar in node.scalars() {
        match scalar {
            ScalarBinding::Number(n) => check(n.is_finite(), path, "scalar must be finite")?,
            ScalarBinding::State(s) => {
                let n = s.strip_prefix("record.").unwrap_or(s);
                check(
                    !s.starts_with("record.") || record,
                    path,
                    "record scalar outside template",
                )?;
                name(n, path)?;
            }
        }
    }
    check(
        node.key.is_none() || node.kind == LayoutKind::Key,
        &field(path, "key"),
        "key property requires key node",
    )?;
    check(
        node.kind != LayoutKind::Key || node.key.is_some(),
        &field(path, "key"),
        "key requires key",
    )?;
    check(
        node.keys.is_empty() || node.kind == LayoutKind::KeyRow,
        &field(path, "keys"),
        "keys require key-row",
    )?;
    check(
        node.kind != LayoutKind::KeyRow || !node.keys.is_empty(),
        &field(path, "keys"),
        "key-row requires keys",
    )?;
    for (i, k) in node.keys.iter().enumerate() {
        budget.nodes += 1;
        check(
            budget.nodes <= MAX_NODES,
            &field(path, "keys"),
            "layout exceeds node budget",
        )?;
        if let Some(id) = &k.id {
            budget.id(id, &format!("{path}.keys[{i}].id"))?;
        }
        if let Some(w) = k.width {
            length(w, &format!("{path}.keys[{i}].width"))?;
        }
    }
    for (i, c) in node.children.iter().enumerate() {
        validate_node(
            c,
            &format!("{path}.children[{i}]"),
            depth + 1,
            record,
            budget,
        )?;
    }
    if let Some(t) = &node.template {
        validate_node(t, &field(path, "template"), depth + 1, true, budget)?;
    }
    Ok(())
}
