use std::{collections::BTreeMap, sync::Arc};
use touchbar_core::*;
use touchbar_model::*;
use touchbar_scene::*;
use touchbar_testkit::*;
use touchbar_ui::*;
pub fn fonts() -> CpuPainter {
    let mut db = cosmic_text::fontdb::Database::new();
    db.load_font_data(include_bytes!("../../../../assets/fonts/test/InterVariable.ttf").to_vec());
    db.set_sans_serif_family("Inter Variable");
    CpuPainter::new(cosmic_text::FontSystem::new_with_locale_and_db(
        "en-US".into(),
        db,
    ))
}
pub struct ActualRenderer {
    pub renderer: SceneRenderer,
    pub docs: Documents,
    pub painter: CpuPainter,
}
impl ActualRenderer {
    pub fn controls(plugin: &LoadedPlugin, wide: bool) -> Self {
        let mut docs = Documents::default();
        for file in &plugin.sets {
            let mut file = file.clone();
            if wide && file.id != "microphone" {
                file.layout.width = Some(Length::Unit(600.0));
            }
            docs.sets.insert(
                format!("controls/{}", file.id).parse().unwrap(),
                Arc::new(SetDocument {
                    file,
                    assets: BTreeMap::new(),
                }),
            );
        }
        let system = plugin_dir(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/system"),
        )
        .unwrap();
        for mut file in system.sets {
            file.layout.width = Some(Length::Percent {
                value: 100.0,
                axis: Axis::Width,
            });
            docs.sets.insert(
                format!("system/{}", file.id).parse().unwrap(),
                Arc::new(SetDocument {
                    file,
                    assets: BTreeMap::new(),
                }),
            );
        }
        Self {
            renderer: SceneRenderer::default(),
            docs,
            painter: fonts(),
        }
    }
}
impl FrameRenderer for ActualRenderer {
    fn render_layout(
        &mut self,
        info: &SurfaceInfo,
        generation: Generation,
        scene: &touchbar_core::Scene,
        now: Timestamp,
    ) -> Input {
        let frame = self
            .renderer
            .frame(
                info,
                scene,
                &self.docs,
                Theme::dark(),
                &mut self.painter,
                now,
                MotionPolicy::Full,
            )
            .unwrap();
        assert!(frame.faults.is_empty(), "render faults: {:?}", frame.faults);
        Input::Layout {
            surface: info.id,
            generation,
            hit_map: frame.layout.hit_map,
            representations: frame.layout.representations,
            scrubbers: frame.layout.scrubbers,
            slots: frame.layout.slots,
            presentation: frame.layout.presentation,
        }
    }
}
