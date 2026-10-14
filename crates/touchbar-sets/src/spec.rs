use crate::{document::error, *};
use std::collections::BTreeSet;
use touchbar_model::{
    DismissalPolicy, PanelSpec, SetSpec, StateBinding, WidgetId, WidgetKind, WidgetSpec,
};
struct Conversion {
    used: BTreeSet<String>,
    next: usize,
}
impl Conversion {
    fn id(&mut self, id: Option<&str>) -> WidgetId {
        if let Some(id) = id {
            return id.parse().expect("validated widget id");
        }
        loop {
            let id = format!("auto-layout-{}", self.next);
            self.next += 1;
            if self.used.insert(id.clone()) {
                return id.parse().expect("generated id");
            }
        }
    }
    fn node(&mut self, node: &LayoutNode) -> WidgetSpec {
        let kind = match node.kind {
            LayoutKind::Button => WidgetKind::Button,
            LayoutKind::Slider => WidgetKind::Slider {
                minimum: 0.0,
                maximum: 1.0,
            },
            LayoutKind::Scrubber => WidgetKind::Scrubber,
            LayoutKind::Canvas => WidgetKind::Canvas,
            LayoutKind::Key => WidgetKind::Key {
                key: node.key.expect("validated key"),
            },
            _ => WidgetKind::Noninteractive,
        };
        let mut spec = WidgetSpec::new(self.id(node.id.as_deref()), kind);
        spec.hold_ms = node.hold_ms;
        spec.handlers = node.actions().map(|(_, a, k)| (k, a.to_string())).collect();
        for (k, b) in [
            (
                StateBinding::Text,
                node.text_from.as_ref().or(node.label_from.as_ref()),
            ),
            (StateBinding::Symbol, node.symbol_from.as_ref()),
            (StateBinding::Value, node.value_from.as_ref()),
            (StateBinding::Enabled, node.enabled_from.as_ref()),
            (StateBinding::Visible, node.visible_from.as_ref()),
            (StateBinding::Selected, node.selected_from.as_ref()),
            (StateBinding::Canvas, node.canvas_from.as_ref()),
            (StateBinding::Records, node.repeat_from.as_ref()),
        ] {
            if let Some(b) = b {
                spec.state_bindings.insert(k, b.clone());
            }
        }
        spec.expanded_panel = node
            .expanded_panel
            .as_ref()
            .map(|p| p.parse().expect("validated panel"));
        spec.press_and_hold_panel = node
            .press_and_hold_panel
            .as_ref()
            .map(|p| p.parse().expect("validated panel"));
        spec.children = node.children.iter().map(|c| self.node(c)).collect();
        spec.children
            .extend(node.variants.iter().map(|v| self.node(&v.layout)));
        if let Some(t) = &node.template {
            spec.children.push(self.node(t));
        }
        for key in &node.keys {
            let id = self.id(key.id.as_deref());
            spec.children
                .push(WidgetSpec::new(id, WidgetKind::Key { key: key.key }));
        }
        spec
    }
}
impl SetFile {
    /// The manifest supplies logic presence; layout/style data remain in this SetFile (D66).
    pub fn to_spec(&self, plugin: &PluginManifest) -> Result<SetSpec, DocumentError> {
        self.validate()?;
        plugin.validate()?;
        if plugin.logic.is_none()
            && (!self.persist.is_empty()
                || self.layout.has_logic()
                || self.panels.iter().any(|p| p.layout.has_logic()))
        {
            return Err(error(
                "layout",
                "state bindings, handlers and persist require a logic provider",
            ));
        }
        let mut convert = Conversion {
            used: BTreeSet::new(),
            next: 0,
        };
        let mut reserve = |node: &LayoutNode, _: &str| {
            if let Some(id) = &node.id {
                convert.used.insert(id.clone());
            }
            for key in &node.keys {
                if let Some(id) = &key.id {
                    convert.used.insert(id.clone());
                }
            }
            Ok(())
        };
        self.layout.walk("layout", &mut reserve)?;
        for p in &self.panels {
            p.layout.walk("panel", &mut reserve)?;
        }
        Ok(SetSpec {
            reference: format!("{}/{}", plugin.id, self.id)
                .parse()
                .expect("validated set reference"),
            logic: plugin.logic.as_ref().map(|_| plugin.id.clone()),
            logic_config: self.logic_config.clone(),
            persist: self.persist.clone(),
            units: self.units.map(|u| u.u),
            widgets: vec![convert.node(&self.layout)],
            panels: self
                .panels
                .iter()
                .map(|p| PanelSpec {
                    id: p.id.parse().expect("validated panel"),
                    widgets: vec![convert.node(&p.layout)],
                    principal_item: p
                        .principal_item
                        .as_ref()
                        .map(|id| id.parse().expect("validated principal widget")),
                    dismiss_on_selection: p.dismiss_on_selection,
                    dismissal: DismissalPolicy::default(),
                })
                .collect(),
        })
    }
}
