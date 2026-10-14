use crate::{
    Documents, Frame, FrameLayout, NamedSlot, SetDocument,
    builder::{self, Built, Timelines},
    canvas, composition,
};
use anyhow::{Context, Result};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};
use touchbar_core::{
    InstanceStatus, PresentationLayout, Scene, SceneItem, ScrubberLayout, SlotBounds,
};
use touchbar_layout::{self as l, BarSpec, PixelRect};
use touchbar_model as m;
use touchbar_ui as u;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub trees_built: u64,
    pub layouts_resolved: u64,
}
struct Cached {
    doc: Arc<SetDocument>,
    item: SceneItem,
    size: u::Size,
    surface_size: (u32, u32),
    theme: u::Theme,
    built: Built,
    ui: u::RetainedUi,
    bounds: u::Rect,
    resolved: u::ResolvedUi,
}
#[derive(Default)]
pub struct SceneRenderer {
    trees: BTreeMap<(m::SurfaceId, m::InstanceId, Option<m::PanelId>), Cached>,
    layouts: BTreeMap<(m::SurfaceId, m::ScenePlane), (BarSpec, u32, l::ResolvedLayout)>,
    stats: CacheStats,
    symbols: u::SymbolCatalog,
}
impl SceneRenderer {
    pub fn cache_stats(&self) -> CacheStats {
        self.stats
    }
    pub fn remove_surface(&mut self, surface: m::SurfaceId) {
        self.trees.retain(|(s, _, _), _| *s != surface);
        self.layouts.retain(|(s, _), _| *s != surface);
    }
    #[allow(clippy::too_many_arguments)]
    pub fn frame(
        &mut self,
        surface: &m::SurfaceInfo,
        scene: &Scene,
        docs: &Documents,
        theme: u::Theme,
        painter: &mut u::CpuPainter,
        now: m::Timestamp,
        policy: u::MotionPolicy,
    ) -> Result<Frame> {
        surface.validate()?;
        let composition = composition::compose(surface, scene, docs)?;
        let mut bounds = BTreeMap::new();
        let mut layout = FrameLayout::default();
        let mut used_layouts = BTreeSet::new();
        for (plane, bar) in composition.bars {
            let key = (surface.id, plane);
            used_layouts.insert(key);
            if !self
                .layouts
                .get(&key)
                .is_some_and(|(old, width, _)| old == &bar && *width == surface.width_px)
            {
                let resolved = l::resolve(&bar, surface.width_px, &BTreeSet::new())?;
                self.layouts.insert(key, (bar, surface.width_px, resolved));
                self.stats.layouts_resolved += 1;
            }
            let resolved = &self.layouts[&key].2;
            for (id, (slot, instance, p)) in &composition.slots {
                if *p != plane {
                    continue;
                }
                let rect = resolved
                    .placements
                    .iter()
                    .find(|p| p.id == *id)
                    .map(|p| m::Rect {
                        x: p.x as f32,
                        y: 0.0,
                        width: p.width as f32,
                        height: surface.height_px as f32,
                    });
                layout.named_slots.push(NamedSlot {
                    slot: slot.clone(),
                    instance: *instance,
                    bounds: rect,
                });
                if let (Some(instance), Some(rect)) = (instance, rect) {
                    layout.slots.push(SlotBounds {
                        instance: *instance,
                        rect,
                    });
                    bounds.insert(*instance, rect);
                }
            }
        }
        self.layouts
            .retain(|key, _| key.0 != surface.id || used_layouts.contains(key));
        let top = scene
            .items
            .iter()
            .filter(|i| {
                !matches!(i.status, InstanceStatus::Hidden | InstanceStatus::Disposed)
                    && matches!(i.plane, m::ScenePlane::Layer(_))
            })
            .map(|i| i.plane)
            .max()
            .unwrap_or(m::ScenePlane::Profile);
        let mut paint = u::Scene::new(theme.background);
        let mut faults = vec![];
        let mut warnings = vec![];
        let mut used_trees = BTreeSet::new();
        for item in &scene.items {
            if item.plane != top
                || matches!(
                    item.status,
                    InstanceStatus::Hidden | InstanceStatus::Disposed
                )
            {
                continue;
            }
            let Some(bounds) = bounds.get(&item.instance).copied() else {
                continue;
            };
            let key = (surface.id, item.instance, None);
            used_trees.insert(key);
            self.draw_item(
                surface,
                item,
                docs,
                theme,
                painter,
                now,
                policy,
                bounds,
                None,
                item.plane,
                &mut paint,
                &mut layout,
                &mut faults,
                &mut warnings,
            )?;
        }
        if let Some(panel) = &scene.presentation
            && let Some(owner) = scene.items.iter().find(|i| {
                i.instance == panel.owner
                    && !matches!(i.status, InstanceStatus::Hidden | InstanceStatus::Disposed)
            })
            && let Some(owner_bounds) = bounds.get(&owner.instance).copied()
        {
            let doc = docs
                .sets
                .get(&owner.set.reference)
                .context("owner document absent")?;
            let definition = doc
                .file
                .panels
                .iter()
                .find(|p| p.id == panel.panel.id.as_str())
                .context("panel document absent")?;
            let sizing = l::PanelSizing {
                minimum: sizing_px(definition.minimum_width, surface, owner.units)?,
                preferred: sizing_px(definition.preferred_width, surface, owner.units)?,
                maximum: sizing_px(definition.maximum_width, surface, owner.units)?,
            };
            // Region/full do not use width constraints; anchored validates them.
            let rect = l::panel_bounds(
                PixelRect {
                    x: 0,
                    y: 0,
                    width: surface.width_px,
                    height: surface.height_px,
                },
                pixel_rect(owner_bounds)?,
                pixel_rect(panel.anchor)?,
                match panel.placement {
                    m::Placement::Region => l::PanelPlacement::Region,
                    m::Placement::Full => l::PanelPlacement::Full,
                    m::Placement::Anchored => l::PanelPlacement::Anchored,
                },
                sizing,
            )?;
            let rect = m::Rect {
                x: rect.x as f32,
                y: rect.y as f32,
                width: rect.width as f32,
                height: rect.height as f32,
            };
            layout.presentation = Some(PresentationLayout {
                id: panel.id,
                bounds: rect,
            });
            // Drop covered base hits. Core owns panel dismissal/capture cancellation.
            layout.hit_map.retain(|h| !overlaps(h.rect, rect));
            layout.scrubbers.retain(|s| !overlaps(s.viewport, rect));
            paint.rounded_rect(canvas::rect(rect), 0.0, theme.background);
            used_trees.insert((surface.id, owner.instance, Some(panel.panel.id.clone())));
            self.draw_item(
                surface,
                owner,
                docs,
                theme,
                painter,
                now,
                policy,
                rect,
                Some(&panel.panel.id),
                m::ScenePlane::Presentation,
                &mut paint,
                &mut layout,
                &mut faults,
                &mut warnings,
            )?;
        }
        self.trees
            .retain(|key, _| key.0 != surface.id || used_trees.contains(key));
        let time = Duration::from_millis(now.0);
        let animate = paint.has_active_motion(time, policy);
        let display_list = painter.display_list(&paint, time, policy)?;
        Ok(Frame {
            display_list,
            layout,
            animate,
            faults,
            warnings,
        })
    }
    #[allow(clippy::too_many_arguments)]
    fn draw_item(
        &mut self,
        surface: &m::SurfaceInfo,
        item: &SceneItem,
        docs: &Documents,
        theme: u::Theme,
        painter: &mut u::CpuPainter,
        now: m::Timestamp,
        policy: u::MotionPolicy,
        bounds: m::Rect,
        panel: Option<&m::PanelId>,
        plane: m::ScenePlane,
        paint: &mut u::Scene,
        layout: &mut FrameLayout,
        faults: &mut Vec<(m::InstanceId, String)>,
        warnings: &mut Vec<String>,
    ) -> Result<()> {
        if bounds.width == 0.0 || bounds.height == 0.0 {
            return Ok(());
        }
        let bounds = canvas::rect(bounds);
        match item.status {
            InstanceStatus::Loading => {
                paint.rounded_rect(bounds.inset(4.0), theme.corner_radius, theme.control);
                return Ok(());
            }
            InstanceStatus::Faulted => {
                placeholder(paint, bounds, theme);
                return Ok(());
            }
            _ => {}
        }
        let doc = docs
            .sets
            .get(&item.set.reference)
            .context("set document absent")?
            .clone();
        let key = (surface.id, item.instance, panel.cloned());
        let rebuild = !self.trees.get(&key).is_some_and(|old| {
            Arc::ptr_eq(&old.doc, &doc)
                && old.item.state == item.state
                && old.item.interactions == item.interactions
                && old.item.units == item.units
                && old.item.set == item.set
                && old.size == bounds.size()
                && old.surface_size == (surface.width_px, surface.height_px)
                && old.theme == theme
        });
        if rebuild {
            let (root, spec) = if let Some(panel) = panel {
                let definition = doc
                    .file
                    .panels
                    .iter()
                    .find(|p| p.id == panel.as_str())
                    .context("panel definition absent")?;
                let spec = item
                    .set
                    .panels
                    .iter()
                    .find(|p| p.id == *panel)
                    .and_then(|p| p.widgets.first())
                    .context("panel WidgetSpec absent")?;
                (&definition.layout, spec)
            } else {
                (
                    &doc.file.layout,
                    item.set.widgets.first().context("root WidgetSpec absent")?,
                )
            };
            let timelines = self
                .trees
                .get(&key)
                .map_or_else(Timelines::default, |old| old.built.timelines.clone());
            match builder::build(
                item,
                &doc,
                root,
                spec,
                surface,
                bounds.size(),
                Duration::from_millis(now.0),
                timelines,
                &mut self.symbols,
            ) {
                Ok(built) => {
                    let mut ui = u::RetainedUi::new(built.root.clone());
                    let resolved = ui.resolve_with_measurer(bounds, theme, painter);
                    warnings.extend(built.warnings.clone());
                    self.trees.insert(
                        key.clone(),
                        Cached {
                            doc,
                            item: item.clone(),
                            size: bounds.size(),
                            surface_size: (surface.width_px, surface.height_px),
                            theme,
                            built,
                            ui,
                            bounds,
                            resolved,
                        },
                    );
                    self.stats.trees_built += 1;
                }
                Err(error) => {
                    self.trees.remove(&key);
                    placeholder(paint, bounds, theme);
                    faults.push((item.instance, error.to_string()));
                    return Ok(());
                }
            }
        }
        let cached = self.trees.get_mut(&key).unwrap();
        if cached.bounds != bounds {
            cached.resolved = cached.ui.resolve_with_measurer(bounds, theme, painter);
            cached.bounds = bounds;
        }
        let time = Duration::from_millis(now.0);
        // Materialize and validate each instance before publishing its primitives/hits.
        let materialized = match painter
            .materialize(&cached.resolved.scene, time, policy)
            .and_then(|scene| {
                u::DisplayList::from_scene(&scene, time, policy)?;
                Ok(scene)
            }) {
            Ok(scene) => scene,
            Err(error) => {
                self.trees.remove(&key);
                placeholder(paint, bounds, theme);
                faults.push((item.instance, error.to_string()));
                return Ok(());
            }
        };
        paint.push_clip(bounds);
        paint.primitives.extend(materialized.primitives);
        paint.pop_clip();
        let root_id = u::WidgetId(1);
        if panel.is_none() {
            layout.representations.push((
                item.instance,
                cached
                    .resolved
                    .inspector
                    .representations
                    .get(&root_id)
                    .map_or("", |r| match r {
                        u::Representation::Hidden => "hidden",
                        u::Representation::Minimal => "minimal",
                        u::Representation::Compact => "compact",
                        u::Representation::Full => "full",
                    })
                    .into(),
            ));
        }
        for target in cached.resolved.interactions.targets() {
            let Some(identity) = cached.built.identities.get(&target.id) else {
                continue;
            };
            let rect = model_rect(u::physical_rect(target.bounds)?);
            if rect.width <= 0.0 || rect.height <= 0.0 {
                continue;
            }
            layout.hit_map.push(m::HitTarget {
                instance: item.instance,
                widget: identity.widget.clone(),
                plane,
                rect,
                record_key: identity.record.clone(),
            });
            if let Some((content_width, cells)) = &identity.scrubber {
                let mut cells = cells.clone();
                for cell in &mut cells {
                    cell.rect.height = rect.height;
                }
                layout.scrubbers.push(ScrubberLayout {
                    instance: item.instance,
                    widget: identity.widget.clone(),
                    viewport: rect,
                    content_width: *content_width,
                    items: cells,
                });
            }
        }
        Ok(())
    }
}
fn sizing_px(length: m::Length, s: &m::SurfaceInfo, u: m::UnitSize) -> Result<u32> {
    Ok(crate::sizing::pixels(length, s, u)?)
}
fn model_rect(r: u::Rect) -> m::Rect {
    m::Rect {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    }
}
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn pixel_rect(r: m::Rect) -> Result<PixelRect> {
    let r = u::physical_rect(canvas::rect(r))?;
    anyhow::ensure!(r.x >= 0.0 && r.y >= 0.0, "panel rectangle outside surface");
    Ok(PixelRect {
        x: r.x as u32,
        y: r.y as u32,
        width: r.width as u32,
        height: r.height as u32,
    })
}
fn overlaps(a: m::Rect, b: m::Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}
fn placeholder(scene: &mut u::Scene, bounds: u::Rect, theme: u::Theme) {
    scene.rounded_rect(bounds.inset(4.0), theme.corner_radius, theme.control);
    scene.text(
        bounds,
        "!",
        bounds.height.min(24.0),
        theme.destructive,
        u::TextAlign::Center,
    );
}
