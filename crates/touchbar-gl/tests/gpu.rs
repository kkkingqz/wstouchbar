use std::time::Duration;
use touchbar_gl::*;
use touchbar_model::SurfaceId;
use touchbar_ui::{Color, DisplayList, MotionPolicy, Rect, Scene};
fn packet(x: f32, color: Color) -> DisplayList {
    let mut scene = Scene::new(Color::BLACK);
    scene.push_clip(Rect::new(x, 0.0, 10.0, 16.0));
    scene.rounded_rect(Rect::new(x, 0.0, 10.0, 16.0), 0.0, color);
    scene.pop_clip();
    DisplayList::from_scene(&scene, Duration::ZERO, MotionPolicy::Disabled).unwrap()
}
#[test]
#[ignore = "requires EGL (Mesa llvmpipe)"]
fn env_gpu_damage_tracks_only_changed_columns_and_reset_is_full() {
    let mut renderer = Renderer::new(Backend::Surfaceless).unwrap();
    let first = renderer
        .render(
            SurfaceId(1),
            (100, 16),
            Orientation::Landscape,
            &packet(20.0, Color::WHITE),
            false,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        first.damage,
        vec![DamageRect {
            x: 0,
            y: 0,
            width: 100,
            height: 16
        }]
    );
    assert!(
        renderer
            .render(
                SurfaceId(1),
                (100, 16),
                Orientation::Landscape,
                &packet(20.0, Color::WHITE),
                false
            )
            .unwrap()
            .is_none()
    );
    let changed = renderer
        .render(
            SurfaceId(1),
            (100, 16),
            Orientation::Landscape,
            &packet(20.0, Color::rgb(1.0, 0.0, 0.0)),
            false,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        changed.damage,
        vec![DamageRect {
            x: 20,
            y: 0,
            width: 10,
            height: 16
        }]
    );
    let reset = renderer
        .render(
            SurfaceId(1),
            (100, 16),
            Orientation::Landscape,
            &packet(20.0, Color::rgb(1.0, 0.0, 0.0)),
            true,
        )
        .unwrap()
        .unwrap();
    assert_eq!(reset.damage, first.damage);
    renderer.remove_surface(SurfaceId(1));
    assert!(
        renderer
            .render(
                SurfaceId(1),
                (100, 16),
                Orientation::Landscape,
                &packet(20.0, Color::WHITE),
                false
            )
            .unwrap()
            .is_some()
    );
}
#[test]
#[ignore = "requires EGL (Mesa llvmpipe)"]
fn env_gpu_portrait_rotation_matches_cpu_pixel_reference() {
    let mut renderer = Renderer::new(Backend::Surfaceless).unwrap();
    let mut scene = Scene::new(Color::BLACK);
    scene.rounded_rect(
        Rect::new(2.0, 1.0, 5.0, 3.0),
        0.0,
        Color::rgb(1.0, 0.0, 0.0),
    );
    scene.rounded_rect(
        Rect::new(12.0, 4.0, 3.0, 2.0),
        0.0,
        Color::rgb(0.0, 1.0, 0.0),
    );
    let list = DisplayList::from_scene(&scene, Duration::ZERO, MotionPolicy::Disabled).unwrap();
    let original = renderer
        .render(SurfaceId(1), (20, 8), Orientation::Landscape, &list, false)
        .unwrap()
        .unwrap();
    for (id, orientation) in [
        (2, Orientation::Clockwise),
        (3, Orientation::CounterClockwise),
    ] {
        let rotated = renderer
            .render(SurfaceId(id), (20, 8), orientation, &list, false)
            .unwrap()
            .unwrap();
        assert_eq!((rotated.width, rotated.height), (8, 20));
        for y in 0..8usize {
            for x in 0..20usize {
                let (rx, ry) = if orientation == Orientation::Clockwise {
                    (7 - y, x)
                } else {
                    (y, 19 - x)
                };
                assert_eq!(
                    &original.pixels[(y * 20 + x) * 4..(y * 20 + x + 1) * 4],
                    &rotated.pixels[(ry * 8 + rx) * 4..(ry * 8 + rx + 1) * 4]
                );
            }
        }
    }
}
#[test]
#[ignore = "requires EGL (Mesa llvmpipe)"]
fn env_gpu_portable_resources_masks_and_audited_shader_draw() {
    let mut scene = Scene::new(Color::BLACK);
    scene.colored_image(
        Rect::new(0.0, 0.0, 4.0, 4.0),
        touchbar_ui::Image::rgba8(1, 1, 1, 1, vec![255; 4]).unwrap(),
        1.0,
        touchbar_ui::ImageColoring::Mask(Color::rgb(1.0, 0.0, 0.0)),
    );
    scene.shader_effect(
        touchbar_ui::ShaderEffect {
            program: std::sync::Arc::new(
                touchbar_ui::EffectProgram::compile("let color = vec4<f32>(0.0, 1.0, 0.0, 1.0);")
                    .unwrap(),
            ),
            parameters: [0.0; 8],
            opacity: 1.0,
            started: Duration::ZERO,
            period: None,
        },
        Rect::new(4.0, 0.0, 4.0, 4.0),
        touchbar_ui::Theme::dark(),
    );
    let packet = DisplayList::from_scene(&scene, Duration::ZERO, MotionPolicy::Disabled).unwrap();
    let packet: DisplayList =
        serde_json::from_slice(&serde_json::to_vec(&packet).unwrap()).unwrap();
    let image = Renderer::new(Backend::Surfaceless)
        .unwrap()
        .render(SurfaceId(1), (8, 4), Orientation::Landscape, &packet, false)
        .unwrap()
        .unwrap();
    assert_eq!(&image.pixels[9 * 4..9 * 4 + 4], &[255, 0, 0, 255]);
    assert_eq!(&image.pixels[14 * 4..14 * 4 + 4], &[0, 255, 0, 255]);
}
#[test]
#[ignore = "requires EGL (Mesa llvmpipe)"]
fn env_gpu_context_ownership_survives_other_renderer_drop() {
    let mut first = Renderer::new(Backend::Surfaceless).unwrap();
    let mut second = Renderer::new(Backend::Surfaceless).unwrap();
    assert!(
        first
            .render(
                SurfaceId(1),
                (100, 16),
                Orientation::Landscape,
                &packet(20.0, Color::WHITE),
                false
            )
            .unwrap()
            .is_some()
    );
    assert!(
        second
            .render(
                SurfaceId(1),
                (100, 16),
                Orientation::Landscape,
                &packet(20.0, Color::WHITE),
                false
            )
            .unwrap()
            .is_some()
    );
    drop(first);
    assert!(
        second
            .render(
                SurfaceId(1),
                (100, 16),
                Orientation::Landscape,
                &packet(20.0, Color::WHITE),
                true
            )
            .unwrap()
            .is_some()
    );
}
#[test]
#[ignore = "requires EGL (Mesa llvmpipe)"]
fn env_gpu_clips_opacity_transforms_mesh_gradients_and_lines() {
    use touchbar_ui::{Point, Primitive, VisualTransform};
    let mut list = packet(20.0, Color::WHITE);
    list.clear = Color::BLACK;
    list.commands = vec![
        touchbar_ui::DrawCommand::PushClip {
            rect: Rect::new(0.0, 0.0, 10.0, 16.0),
        },
        touchbar_ui::DrawCommand::RoundedRect {
            rect: Rect::new(0.0, 0.0, 40.0, 16.0),
            radius: 0.0,
            color: Color::rgb(1.0, 0.0, 0.0),
        },
        touchbar_ui::DrawCommand::PopClip,
        touchbar_ui::DrawCommand::PushOpacity { opacity: 0.5 },
        touchbar_ui::DrawCommand::PushTransform {
            origin: Point::new(0.0, 0.0),
            transform: VisualTransform {
                translation: Point::new(20.0, 0.0),
                scale: 1.0,
                opacity: 1.0,
            },
        },
        touchbar_ui::DrawCommand::RoundedRect {
            rect: Rect::new(0.0, 0.0, 10.0, 16.0),
            radius: 0.0,
            color: Color::WHITE,
        },
        touchbar_ui::DrawCommand::PopTransform,
        touchbar_ui::DrawCommand::PopOpacity,
    ];
    let mut extra = Scene::new(Color::BLACK);
    extra.linear_gradient_rect(
        Rect::new(40.0, 0.0, 10.0, 16.0),
        0.0,
        Point::new(40.0, 0.0),
        Point::new(50.0, 0.0),
        Color::rgb(1.0, 0.0, 0.0),
        Color::rgb(0.0, 1.0, 0.0),
    );
    extra.line(
        Point::new(60.0, 8.0),
        Point::new(70.0, 8.0),
        4.0,
        Color::rgb(0.0, 0.0, 1.0),
    );
    extra.primitives.push(Primitive::TriangleMesh {
        triangles: vec![
            Point::new(80.0, 0.0),
            Point::new(90.0, 0.0),
            Point::new(80.0, 16.0),
        ]
        .into(),
        color: Color::rgb(0.0, 1.0, 0.0),
    });
    list.commands.extend(
        DisplayList::from_scene(&extra, Duration::ZERO, MotionPolicy::Disabled)
            .unwrap()
            .commands,
    );
    let frame = Renderer::new(Backend::Surfaceless)
        .unwrap()
        .render(
            SurfaceId(1),
            (100, 16),
            Orientation::Landscape,
            &list,
            false,
        )
        .unwrap()
        .unwrap();
    let pixel = |x: usize, y: usize| &frame.pixels[(y * 100 + x) * 4..(y * 100 + x + 1) * 4];
    assert_eq!(pixel(5, 8), [255, 0, 0, 255]);
    assert_eq!(pixel(15, 8), [0, 0, 0, 255]);
    assert_eq!(pixel(25, 8), [128, 128, 128, 255]);
    assert_eq!(pixel(65, 8), [0, 0, 255, 255]);
    assert_eq!(pixel(82, 4), [0, 255, 0, 255]);
    assert!(pixel(42, 8)[0] > pixel(42, 8)[1]);
    assert!(pixel(47, 8)[0] < pixel(47, 8)[1]);
}
#[test]
#[ignore = "requires EGL (Mesa llvmpipe)"]
fn env_gpu_damage_merges_smallest_gaps_to_at_most_eight_regions() {
    let mut renderer = Renderer::new(Backend::Surfaceless).unwrap();
    let empty = DisplayList::from_scene(
        &Scene::new(Color::BLACK),
        Duration::ZERO,
        MotionPolicy::Disabled,
    )
    .unwrap();
    renderer
        .render(
            SurfaceId(1),
            (200, 16),
            Orientation::Landscape,
            &empty,
            false,
        )
        .unwrap();
    let mut scene = Scene::new(Color::BLACK);
    for n in 0..9 {
        let rect = Rect::new((10 + n * 20) as f32, 0.0, 2.0, 16.0);
        scene.push_clip(rect);
        scene.rounded_rect(rect, 0.0, Color::WHITE);
        scene.pop_clip();
    }
    let packet = DisplayList::from_scene(&scene, Duration::ZERO, MotionPolicy::Disabled).unwrap();
    let frame = renderer
        .render(
            SurfaceId(1),
            (200, 16),
            Orientation::Landscape,
            &packet,
            false,
        )
        .unwrap()
        .unwrap();
    assert_eq!(frame.damage.len(), 8);
    assert_eq!(frame.damage.iter().map(|r| r.width).sum::<u32>(), 36);
    for n in 0..9 {
        assert!(
            frame
                .damage
                .iter()
                .any(|r| r.x <= 10 + n * 20 && r.x + r.width >= 12 + n * 20)
        );
    }
}
#[test]
#[ignore = "requires EGL (Mesa llvmpipe)"]
fn env_gpu_multiple_surfaces_keep_independent_previous_frames() {
    let mut renderer = Renderer::new(Backend::Surfaceless).unwrap();
    let first = packet(20.0, Color::WHITE);
    let second = packet(60.0, Color::rgb(1.0, 0.0, 0.0));
    renderer
        .render(
            SurfaceId(1),
            (100, 16),
            Orientation::Landscape,
            &first,
            false,
        )
        .unwrap();
    renderer
        .render(
            SurfaceId(2),
            (100, 16),
            Orientation::Landscape,
            &second,
            false,
        )
        .unwrap();
    assert!(
        renderer
            .render(
                SurfaceId(1),
                (100, 16),
                Orientation::Landscape,
                &first,
                false
            )
            .unwrap()
            .is_none()
    );
    assert!(
        renderer
            .render(
                SurfaceId(2),
                (100, 16),
                Orientation::Landscape,
                &second,
                false
            )
            .unwrap()
            .is_none()
    );
}
