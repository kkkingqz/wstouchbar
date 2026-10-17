use std::collections::BTreeSet;
use touchbar_layout::*;
#[test]
fn spare_width_is_shared_by_grow_weights_and_redistributed_at_maximum() {
    let bar = BarSpec {
        elements: vec![
            Element::Item(ItemSpec::new("a", 40, 80, 120).grow(1.0)),
            Element::Item(ItemSpec::new("b", 40, 80, 400).grow(3.0)),
        ],
        principal_item: None,
    };
    let resolved = resolve(&bar, 400, &BTreeSet::new()).unwrap();
    assert_eq!(
        resolved
            .placements
            .iter()
            .map(|p| p.width)
            .collect::<Vec<_>>(),
        [120, 280]
    );
    assert_eq!(resolved.content_width, 400);
}
#[test]
fn grow_and_flexible_space_share_the_same_spare_width_pool() {
    let bar = BarSpec {
        elements: vec![
            Element::Item(ItemSpec::new("a", 40, 80, 400).grow(1.0)),
            Element::FlexibleSpace {
                minimum: 20,
                weight: 1,
            },
            Element::Item(ItemSpec::new("b", 40, 80, 80)),
        ],
        principal_item: None,
    };
    let resolved = resolve(&bar, 400, &BTreeSet::new()).unwrap();
    assert_eq!(resolved.placements[0].width, 190);
    assert_eq!(resolved.placements[1].x, 320);
}
#[test]
fn invalid_grow_is_rejected_without_panics() {
    for weight in [-1.0, f32::NAN, f32::INFINITY] {
        let bar = BarSpec {
            elements: vec![Element::Item(ItemSpec::new("a", 10, 20, 40).grow(weight))],
            principal_item: None,
        };
        assert!(resolve(&bar, 200, &BTreeSet::new()).is_err());
    }
}
fn rect(x: u32, y: u32, width: u32, height: u32) -> PixelRect {
    PixelRect {
        x,
        y,
        width,
        height,
    }
}
#[test]
fn region_and_full_panels_use_their_declared_regions() {
    let surface = rect(0, 0, 2008, 60);
    let owner = rect(800, 0, 200, 60);
    let anchor = rect(850, 10, 40, 40);
    let sizing = PanelSizing {
        minimum: 100,
        preferred: 400,
        maximum: 600,
    };
    assert_eq!(
        panel_bounds(surface, owner, anchor, PanelPlacement::Region, sizing).unwrap(),
        owner
    );
    assert_eq!(
        panel_bounds(surface, owner, anchor, PanelPlacement::Full, sizing).unwrap(),
        surface
    );
}
#[test]
fn anchored_panels_center_on_the_anchor_and_clamp_to_surface_edges() {
    let sizing = PanelSizing {
        minimum: 100,
        preferred: 400,
        maximum: 600,
    };
    let surface = rect(0, 0, 2008, 60);
    assert_eq!(
        panel_bounds(
            surface,
            rect(900, 0, 200, 60),
            rect(980, 12, 40, 30),
            PanelPlacement::Anchored,
            sizing
        )
        .unwrap(),
        rect(800, 0, 400, 60)
    );
    assert_eq!(
        panel_bounds(
            surface,
            rect(0, 0, 100, 60),
            rect(0, 10, 40, 40),
            PanelPlacement::Anchored,
            sizing
        )
        .unwrap(),
        rect(0, 0, 400, 60)
    );
    assert_eq!(
        panel_bounds(
            surface,
            rect(1908, 0, 100, 60),
            rect(1968, 10, 40, 40),
            PanelPlacement::Anchored,
            sizing
        )
        .unwrap(),
        rect(1608, 0, 400, 60)
    );
}
#[test]
fn anchored_panels_shrink_to_surface_and_reject_impossible_minimum() {
    let surface = rect(0, 0, 300, 60);
    let owner = rect(0, 0, 100, 60);
    let anchor = rect(10, 10, 40, 40);
    assert_eq!(
        panel_bounds(
            surface,
            owner,
            anchor,
            PanelPlacement::Anchored,
            PanelSizing {
                minimum: 100,
                preferred: 400,
                maximum: 600
            }
        )
        .unwrap(),
        surface
    );
    assert!(
        panel_bounds(
            surface,
            owner,
            anchor,
            PanelPlacement::Anchored,
            PanelSizing {
                minimum: 301,
                preferred: 400,
                maximum: 600
            }
        )
        .is_err()
    );
}
#[test]
fn hidden_growing_slots_do_not_consume_spare_width() {
    let bar = BarSpec {
        elements: vec![
            Element::Item(
                ItemSpec::new("hidden", 90, 90, 300)
                    .grow(1.0)
                    .visibility_priority(-1),
            ),
            Element::Item(ItemSpec::new("visible", 60, 100, 300).grow(1.0)),
        ],
        principal_item: None,
    };
    let resolved = resolve(&bar, 130, &BTreeSet::new()).unwrap();
    assert_eq!(resolved.hidden_items, [ItemId::from("hidden")]);
    assert_eq!(
        resolved.placements,
        [Placement {
            id: ItemId::from("visible"),
            x: 0,
            width: 130
        }]
    );
}
#[test]
fn saturation_redistributes_equally_without_favoring_the_last_slot() {
    let bar = BarSpec {
        elements: vec![
            Element::Item(ItemSpec::new("a", 40, 80, 90).grow(1.0)),
            Element::Item(ItemSpec::new("b", 40, 80, 300).grow(1.0)),
            Element::Item(ItemSpec::new("c", 40, 80, 300).grow(1.0)),
        ],
        principal_item: None,
    };
    let resolved = resolve(&bar, 480, &BTreeSet::new()).unwrap();
    assert_eq!(
        resolved
            .placements
            .iter()
            .map(|p| p.width)
            .collect::<Vec<_>>(),
        [90, 195, 195]
    );
}
#[test]
fn nested_groups_distribute_fixed_and_flexible_spaces_with_growing_slots() {
    let bar = BarSpec {
        elements: vec![Element::Group(GroupSpec::new(
            "group",
            vec![
                GroupElement::Item(ItemSpec::new("a", 40, 80, 80)),
                GroupElement::FixedSpace { width: 10 },
                GroupElement::FlexibleSpace {
                    minimum: 20,
                    weight: 2,
                },
                GroupElement::Item(ItemSpec::new("b", 40, 80, 300).grow(1.0)),
            ],
        ))],
        principal_item: None,
    };
    let resolved = resolve(&bar, 400, &BTreeSet::new()).unwrap();
    assert_eq!(
        resolved.placements,
        [
            Placement {
                id: ItemId::from("a"),
                x: 0,
                width: 80
            },
            Placement {
                id: ItemId::from("b"),
                x: 250,
                width: 150
            }
        ]
    );
}
#[test]
fn nested_flexible_space_requires_positive_weight() {
    let bar = BarSpec {
        elements: vec![Element::Group(GroupSpec::new(
            "group",
            vec![GroupElement::FlexibleSpace {
                minimum: 20,
                weight: 0,
            }],
        ))],
        principal_item: None,
    };
    assert!(resolve(&bar, 400, &BTreeSet::new()).is_err());
}
#[test]
fn panel_geometry_and_sizing_fail_cleanly_for_invalid_inputs() {
    let surface = rect(0, 0, 300, 60);
    let owner = rect(0, 0, 100, 60);
    let anchor = rect(10, 10, 40, 40);
    for sizing in [
        PanelSizing {
            minimum: 200,
            preferred: 100,
            maximum: 300,
        },
        PanelSizing {
            minimum: 100,
            preferred: 400,
            maximum: 300,
        },
        PanelSizing {
            minimum: 0,
            preferred: 0,
            maximum: 0,
        },
    ] {
        assert!(panel_bounds(surface, owner, anchor, PanelPlacement::Anchored, sizing).is_err());
    }
    let sizing = PanelSizing {
        minimum: 100,
        preferred: 200,
        maximum: 300,
    };
    assert!(
        panel_bounds(
            surface,
            rect(290, 0, 20, 60),
            anchor,
            PanelPlacement::Region,
            sizing
        )
        .is_err()
    );
    assert!(
        panel_bounds(
            surface,
            owner,
            rect(290, 0, 20, 40),
            PanelPlacement::Anchored,
            sizing
        )
        .is_err()
    );
    assert!(
        panel_bounds(
            surface,
            rect(0, 0, 0, 60),
            anchor,
            PanelPlacement::Full,
            sizing
        )
        .is_err()
    );
}
