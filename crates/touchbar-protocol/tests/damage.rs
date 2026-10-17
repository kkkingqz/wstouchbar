use touchbar_protocol::{DamageRect, SwapchainHeader, XRGB8888, damage::*};
fn header(rotated: bool) -> SwapchainHeader {
    let (w, h): (u32, u32) = if rotated { (60, 2008) } else { (2008, 60) };
    SwapchainHeader {
        logical_width: 2008,
        logical_height: 60,
        physical_width: w,
        physical_height: h,
        format: XRGB8888,
        pitch: w.div_ceil(64) * 64 * 4,
        buffer_size: u64::from(w.div_ceil(64) * 64 * 4) * u64::from(h),
        buffer_count: 3,
        frame_damage: true,
    }
}
#[test]
fn damage_maps_both_ways_using_v1_axis_exchange() {
    let logical = DamageRect {
        x: 100,
        y: 4,
        width: 20,
        height: 8,
    };
    for rotated in [false, true] {
        let h = header(rotated);
        let physical = logical_to_physical(h, logical).unwrap();
        assert_eq!(physical_to_logical(h, physical).unwrap(), logical);
        assert_eq!(
            physical,
            if rotated {
                DamageRect {
                    x: 4,
                    y: 100,
                    width: 8,
                    height: 20,
                }
            } else {
                logical
            }
        );
    }
    assert!(
        logical_to_physical(
            header(true),
            DamageRect {
                x: 2000,
                y: 0,
                width: 9,
                height: 1
            }
        )
        .is_err()
    );
    assert!(
        physical_to_logical(
            header(true),
            DamageRect {
                x: 60,
                y: 0,
                width: 1,
                height: 1
            }
        )
        .is_err()
    );
}
#[test]
fn overlapping_damage_is_unioned_and_excess_complexity_falls_back_to_full() {
    let a = DamageRect {
        x: 0,
        y: 0,
        width: 10,
        height: 10,
    };
    let b = DamageRect {
        x: 5,
        y: 5,
        width: 10,
        height: 10,
    };
    assert_eq!(
        merge_damage(&[a, b], 100, 100).unwrap(),
        Damage::Rects(vec![DamageRect {
            x: 0,
            y: 0,
            width: 15,
            height: 15
        }])
    );
    let separate: Vec<_> = (0..9)
        .map(|x| DamageRect {
            x: x * 3,
            y: 0,
            width: 1,
            height: 1,
        })
        .collect();
    assert_eq!(merge_damage(&separate, 100, 100).unwrap(), Damage::Full);
    assert_eq!(merge_damage(&[], 100, 100).unwrap(), Damage::Full);
    assert!(
        merge_damage(
            &[DamageRect {
                x: 99,
                y: 0,
                width: 2,
                height: 1
            }],
            100,
            100
        )
        .is_err()
    );
}
