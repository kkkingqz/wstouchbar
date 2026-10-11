use std::collections::BTreeMap;
use touchbar_core::*;
use touchbar_model::*;
#[derive(Default)]
pub struct FakeRenderer {
    pub delay_ms: u64,
    pub frames: Vec<(SurfaceId, Generation)>,
    widths: BTreeMap<String, f32>,
    pub(crate) hits: BTreeMap<SurfaceId, HitMap>,
}
struct Geometry {
    hits: HitMap,
    scrubbers: Vec<ScrubberLayout>,
}
impl FakeRenderer {
    pub fn width(&mut self, widget: &str, width: f32) {
        assert!(
            width.is_finite() && width > 0.0,
            "positive fake widget width"
        );
        self.widths.insert(widget.into(), width);
    }
    fn widget_width(&self, widget: &WidgetSpec, info: &SurfaceInfo, units: UnitSize) -> f32 {
        self.widths
            .get(widget.id.as_str())
            .copied()
            .or_else(|| widget.width.map(|w| w.to_px(info, units)))
            .unwrap_or(80.0)
    }
    fn place(
        &self,
        widgets: &[WidgetSpec],
        item: &SceneItem,
        plane: ScenePlane,
        bounds: Rect,
        info: &SurfaceInfo,
        geometry: &mut Geometry,
    ) {
        let mut leaves = vec![];
        for widget in widgets {
            widget.visit(&mut leaves);
        }
        let mut x = bounds.x;
        for widget in leaves {
            if !widget.children.is_empty() {
                continue;
            }
            let width = self
                .widget_width(widget, info, item.units)
                .min((bounds.x + bounds.width - x).max(0.0));
            if width <= 0.0 {
                break;
            }
            let rect = Rect {
                x,
                y: bounds.y,
                width,
                height: bounds.height,
            };
            x += width;
            if !matches!(widget.kind, WidgetKind::Noninteractive | WidgetKind::Canvas) {
                geometry.hits.push(HitTarget {
                    instance: item.instance,
                    widget: widget.id.clone(),
                    plane,
                    rect,
                    record_key: None,
                });
            }
            if widget.kind == WidgetKind::Scrubber {
                let records = widget
                    .state_bindings
                    .get(&StateBinding::Records)
                    .and_then(|name| item.state.iter().find(|(n, _)| n == name))
                    .and_then(|(_, v)| {
                        if let StateValue::Records(rows) = v {
                            Some(rows)
                        } else {
                            None
                        }
                    });
                let items: Vec<_> = records
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .filter_map(|(index, row)| {
                        let Scalar::Text(key) = row.get("key")? else {
                            return None;
                        };
                        Some(ScrubberItemGeometry {
                            record_key: key.clone(),
                            rect: Rect {
                                x: index as f32 * 60.0,
                                y: 0.0,
                                width: 60.0,
                                height: rect.height,
                            },
                        })
                    })
                    .collect();
                geometry.scrubbers.push(ScrubberLayout {
                    instance: item.instance,
                    widget: widget.id.clone(),
                    viewport: rect,
                    content_width: items.len() as f32 * 60.0,
                    items,
                });
            }
        }
    }
    pub(crate) fn render(
        &mut self,
        info: &SurfaceInfo,
        generation: Generation,
        scene: &Scene,
    ) -> Input {
        self.frames.push((info.id, generation));
        let mut geometry = Geometry {
            hits: vec![],
            scrubbers: vec![],
        };
        let mut slots = vec![];
        let mut x = 0.0;
        for item in &scene.items {
            let mut widgets = vec![];
            for widget in &item.set.widgets {
                widget.visit(&mut widgets);
            }
            let desired = widgets
                .into_iter()
                .filter(|w| w.children.is_empty())
                .map(|w| self.widget_width(w, info, item.units))
                .sum::<f32>()
                .max(1.0);
            let layer = matches!(item.plane, ScenePlane::Layer(_));
            let left = if layer { 0.0 } else { x };
            let available = (info.width_px as f32 - left).max(0.0);
            if available <= 0.0 {
                continue;
            }
            let width = if layer {
                available
            } else {
                desired.min(available)
            };
            let rect = Rect {
                x: left,
                y: 0.0,
                width,
                height: info.height_px as f32,
            };
            self.place(
                &item.set.widgets,
                item,
                item.plane,
                rect,
                info,
                &mut geometry,
            );
            slots.push(SlotBounds {
                instance: item.instance,
                rect,
            });
            if !layer {
                x += width;
            }
        }
        let presentation = scene.presentation.as_ref().and_then(|panel| {
            let item = scene.items.iter().find(|i| i.instance == panel.owner)?;
            let bounds = match panel.placement {
                Placement::Full => Rect {
                    x: 0.0,
                    y: 0.0,
                    width: info.width_px as f32,
                    height: info.height_px as f32,
                },
                Placement::Region => panel.region,
                Placement::Anchored => {
                    let width = (panel.panel.widgets.len() as f32 * 80.0)
                        .max(1.0)
                        .min(info.width_px as f32);
                    Rect {
                        x: panel.anchor.x.min(info.width_px as f32 - width).max(0.0),
                        y: 0.0,
                        width,
                        height: info.height_px as f32,
                    }
                }
            };
            self.place(
                &panel.panel.widgets,
                item,
                ScenePlane::Presentation,
                bounds,
                info,
                &mut geometry,
            );
            Some(PresentationLayout {
                id: panel.id,
                bounds,
            })
        });
        Input::Layout {
            surface: info.id,
            generation,
            hit_map: geometry.hits,
            representations: vec![],
            scrubbers: geometry.scrubbers,
            slots,
            presentation,
        }
    }
}
pub fn headless(id: u64) -> SurfaceInfo {
    SurfaceInfo {
        id: SurfaceId(id),
        name: format!("headless:{id}").parse().expect("surface name"),
        kind: SurfaceKind::Headless,
        width_px: 500,
        height_px: 50,
        px_per_mm: 5.0,
        caps: SurfaceCaps {
            watch_keys: true,
            inject_keys: true,
            ..SurfaceCaps::default()
        },
    }
}
