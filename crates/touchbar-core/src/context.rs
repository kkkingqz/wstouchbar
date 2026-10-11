use crate::*;
impl Core {
    pub(crate) fn synchronize_contexts(&mut self, output: &mut Vec<Output>) {
        let text = |name: &str| {
            self.facts.get(&FactName::from(name)).and_then(|v| {
                if let FactValue::Text(s) = v {
                    Some(s.clone())
                } else {
                    None
                }
            })
        };
        let color_scheme = text("appearance.scheme")
            .filter(|s| s == "light" || s == "dark")
            .unwrap_or_else(|| "dark".into());
        let accent = text("appearance.accent");
        for surface in self.surfaces.values() {
            if surface.mode != Mode::Display
                || surface.layout_generation != Some(surface.generation)
            {
                continue;
            }
            for item in &surface.scene.items {
                let Some(slot) = surface.slots.iter().find(|s| s.instance == item.instance) else {
                    continue;
                };
                let slot_width =
                    slot.rect.width / Length::Unit(1.0).to_px(&surface.info, item.units);
                if !slot_width.is_finite() {
                    warn(output, "invalid context width in base units");
                    continue;
                }
                let Some(instance) = self
                    .instances
                    .get_mut(&item.instance.number)
                    .filter(|i| i.id == item.instance)
                else {
                    continue;
                };
                instance.context_changed(LogicContext {
                    surface_kind: match surface.info.kind {
                        SurfaceKind::Touchbar => "touchbar",
                        SurfaceKind::Window => "window",
                        SurfaceKind::Headless => "headless",
                    }
                    .into(),
                    slot_width,
                    representation: surface
                        .representations
                        .iter()
                        .find(|(id, _)| *id == item.instance)
                        .map(|(_, name)| name.clone())
                        .unwrap_or_default(),
                    color_scheme: color_scheme.clone(),
                    accent: accent.clone(),
                    locale: self.locale.clone(),
                });
            }
        }
    }
}
