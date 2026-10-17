use std::collections::BTreeSet;
use touchbar_layout::*;
use touchbar_model::{Axis, Length, SurfaceCaps, SurfaceId, SurfaceInfo, SurfaceKind, UnitSize};
fn surface(width: u32, height: u32) -> SurfaceInfo {
    SurfaceInfo {
        id: SurfaceId(1),
        name: "headless:layout".parse().unwrap(),
        kind: SurfaceKind::Headless,
        width_px: width,
        height_px: height,
        px_per_mm: 6.0,
        caps: SurfaceCaps::default(),
    }
}
// The test surfaces and Length validation bound this intentional conversion.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn px(length: Length, s: &SurfaceInfo) -> u32 {
    length
        .to_px(s, UnitSize::resolve(s, None, None))
        .round()
        .max(1.0) as u32
}
#[test]
fn converted_lengths_drive_pixel_layout_on_all_three_surfaces() {
    for (w, h, unit, percent_w, percent_h) in [
        (2008, 60, 96, 402, 36),
        (2170, 60, 96, 434, 36),
        (1200, 160, 256, 240, 96),
    ] {
        let s = surface(w, h);
        assert_eq!(px(Length::Unit(160.0), &s), unit);
        assert_eq!(
            px(
                Length::Percent {
                    value: 20.0,
                    axis: Axis::Width
                },
                &s
            ),
            percent_w
        );
        assert_eq!(
            px(
                Length::Percent {
                    value: 60.0,
                    axis: Axis::Height
                },
                &s
            ),
            percent_h
        );
        let bar = BarSpec {
            elements: vec![
                Element::Item(ItemSpec::new("unit", unit, unit, unit)),
                Element::FixedSpace { width: percent_h },
                Element::Item(ItemSpec::new("width", percent_w, percent_w, percent_w)),
                Element::FlexibleSpace {
                    minimum: 0,
                    weight: 1,
                },
            ],
            principal_item: None,
        };
        let resolved = resolve(&bar, w, &BTreeSet::new()).unwrap();
        assert_eq!(resolved.content_width, w);
        assert_eq!(
            resolved.placements[1],
            Placement {
                id: ItemId::from("width"),
                x: unit + percent_h,
                width: percent_w
            }
        );
        assert_eq!(resolved, resolve(&bar, w, &BTreeSet::new()).unwrap());
    }
}
#[test]
fn responsive_uses_rank_and_converted_thresholds_without_declaration_order_bias() {
    for (w, h, expected) in [
        (2008, 60, Some(0)),
        (2170, 60, Some(0)),
        (1200, 160, Some(2)),
    ] {
        let s = surface(w, h);
        let variants = [
            ResponsiveVariant {
                representation: Representation::Full,
                minimum_width: px(Length::Unit(333.33334), &s),
            },
            ResponsiveVariant {
                representation: Representation::Hidden,
                minimum_width: 0,
            },
            ResponsiveVariant {
                representation: Representation::Compact,
                minimum_width: 100,
            },
            ResponsiveVariant {
                representation: Representation::Minimal,
                minimum_width: 0,
            },
        ];
        let width = px(
            Length::Percent {
                value: 20.0,
                axis: Axis::Width,
            },
            &s,
        );
        assert_eq!(resolve_responsive(&variants, width), expected);
        assert_eq!(resolve_responsive(&variants, 0), Some(3));
        assert_eq!(resolve_responsive(&variants[..1], 0), None);
    }
}
