use touchbar_core::*;
use touchbar_model::*;
use touchbar_testkit::*;
struct RealGeometry;
impl FrameRenderer for RealGeometry {
    fn render_layout(
        &mut self,
        info: &SurfaceInfo,
        generation: Generation,
        scene: &Scene,
        _now: Timestamp,
    ) -> Input {
        let hit_map = scene
            .items
            .iter()
            .map(|i| HitTarget {
                instance: i.instance,
                widget: i.set.widgets[0].id.clone(),
                plane: i.plane,
                rect: Rect {
                    x: 100.0,
                    y: 10.0,
                    width: 200.0,
                    height: 40.0,
                },
                record_key: None,
            })
            .collect();
        Input::Layout {
            surface: info.id,
            generation,
            hit_map,
            representations: vec![],
            scrubbers: vec![],
            slots: scene
                .items
                .iter()
                .map(|i| SlotBounds {
                    instance: i.instance,
                    rect: Rect {
                        x: 100.0,
                        y: 0.0,
                        width: 200.0,
                        height: 50.0,
                    },
                })
                .collect(),
            presentation: None,
        }
    }
}
#[test]
fn harness_uses_injected_geometry_for_touch_helpers() {
    let mut h = Harness::with_renderer(CoreConfig::default(), FakeLogic::default(), RealGeometry);
    h.send(Input::Catalog(
        catalog()
            .plugin("p")
            .set("button", |s| {
                s.button("go");
            })
            .profile("default", |p| {
                p.default().slot("slot", &["p/button"]);
            })
            .build(),
    ));
    h.send(Input::SurfaceAdded(headless(1)));
    h.send(Input::Mode {
        surface: SurfaceId(1),
        mode: Mode::Display,
    });
    assert_eq!(h.hit(SurfaceId(1), "go"), Point { x: 200.0, y: 30.0 });
    h.tap(SurfaceId(1), "go");
    assert!(h.snapshot().captures.is_empty());
}
