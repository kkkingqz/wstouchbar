use std::{sync::Arc, time::Duration};
use touchbar_ui::*;
fn scene(image: Image) -> Scene {
    Scene {
        clear: Color::BLACK,
        primitives: vec![Primitive::Image {
            rect: Rect::new(0.0, 0.0, 2.0, 1.0),
            image,
            opacity: 1.0,
            coloring: ImageColoring::Original,
        }],
    }
}
fn image(id: u64) -> Image {
    Image {
        id,
        revision: id,
        width: 2,
        height: 1,
        pixels: Arc::from([255, 0, 0, 255, 0, 255, 0, 255]),
    }
}
fn build(scene: &Scene) -> DisplayList {
    DisplayList::from_scene(scene, Duration::ZERO, MotionPolicy::Full).unwrap()
}
#[test]
fn physical_rectangles_round_without_erasing_subpixel_nonzero_dimensions() {
    let scene = Scene {
        clear: Color::BLACK,
        primitives: vec![Primitive::RoundedRect {
            rect: Rect::new(0.4, 0.6, 0.2, 10.3),
            radius: 0.0,
            color: Color::WHITE,
        }],
    };
    let list = build(&scene);
    assert!(
        matches!(list.commands.as_slice(), [DrawCommand::RoundedRect {rect, ..}] if *rect == Rect::new(0.0,1.0,1.0,10.0))
    );
}
#[test]
fn resource_keys_depend_on_content_not_source_cache_ids() {
    assert_eq!(build(&scene(image(1))), build(&scene(image(200))));
}
#[test]
fn repeated_raster_uses_one_shared_resource() {
    let source = image(1);
    let mut scene = scene(source.clone());
    scene.primitives.push(scene.primitives[0].clone());
    let list = build(&scene);
    assert_eq!(list.commands.len(), 2);
    assert_eq!(list.resources.len(), 1);
    let Resource::Raster { pixels, .. } = list.resources.values().next().unwrap() else {
        panic!("expected raster")
    };
    assert!(Arc::ptr_eq(pixels, &source.pixels));
}
#[test]
fn json_roundtrip_retains_resources_and_can_be_validated_without_source() {
    let expected = build(&scene(image(1)));
    let json = serde_json::to_vec(&expected).unwrap();
    let restored: DisplayList = serde_json::from_slice(&json).unwrap();
    restored.validate().unwrap();
    assert_eq!(restored, expected);
    let Resource::Raster { pixels, .. } = restored.resources.values().next().unwrap() else {
        panic!("expected raster")
    };
    assert_eq!(pixels.as_ref(), [255, 0, 0, 255, 0, 255, 0, 255]);
}
#[test]
fn broken_raster_and_missing_resource_are_rejected_before_gpu_use() {
    let mut bad = image(1);
    bad.width = u32::MAX;
    assert!(DisplayList::from_scene(&scene(bad), Duration::ZERO, MotionPolicy::Full).is_err());
    let mut list = build(&scene(image(1)));
    list.resources.clear();
    assert!(list.validate().is_err());
}
#[test]
fn malformed_stacks_nonfinite_geometry_and_forged_resources_are_rejected() {
    let mut list = build(&scene(image(1)));
    list.commands.push(DrawCommand::PopClip);
    assert!(list.validate().is_err());
    let mut list = build(&scene(image(1)));
    list.commands
        .insert(0, DrawCommand::PushOpacity { opacity: f32::NAN });
    assert!(list.validate().is_err());
    let mut list = build(&scene(image(1)));
    list.commands.push(DrawCommand::PushClip {
        rect: Rect::new(0.0, 0.0, 5.0, 5.0),
    });
    assert!(list.validate().is_err());
    let mut list = build(&scene(image(1)));
    let resource = list.resources.values_mut().next().unwrap();
    if let Resource::Raster { pixels, .. } = resource {
        *pixels = Arc::from([0_u8; 8]);
    }
    assert!(list.validate().is_err());
}
