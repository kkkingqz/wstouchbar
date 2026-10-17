use touchbar_layout::{ItemId, ItemSpec};
use touchbar_model::{Axis, Length, SurfaceInfo, UnitSize};
use touchbar_sets::LayoutNode;

#[derive(Debug, thiserror::Error)]
#[error("invalid root dimensions")]
pub(crate) struct SizingError;

pub(crate) fn root_item(
    id: ItemId,
    root: &LayoutNode,
    surface: &SurfaceInfo,
    units: UnitSize,
) -> Result<ItemSpec, SizingError> {
    surface.validate().map_err(|_| SizingError)?;
    units.validate().map_err(|_| SizingError)?;
    let basis = pixels(
        root.width.unwrap_or(Length::Unit(800.0 / 3.0)),
        surface,
        units,
    )?;
    if root.minimum.is_none() && root.maximum.is_none() && root.grow.is_none() {
        return Ok(ItemSpec::new(id, basis, basis, basis));
    }
    let minimum = pixels(root.minimum.unwrap_or(Length::Unit(100.0)), surface, units)?;
    let maximum = pixels(
        root.maximum.unwrap_or(Length::Percent {
            value: 100.0,
            axis: Axis::Width,
        }),
        surface,
        units,
    )?;
    let grow = root.grow.unwrap_or(1.0);
    if minimum > maximum || !grow.is_finite() || grow < 0.0 {
        return Err(SizingError);
    }
    Ok(ItemSpec::new(id, minimum, basis.clamp(minimum, maximum), maximum).grow(grow))
}

/// Round each physical dimension once; zero stays zero, positive subpixels stay visible.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(crate) fn pixels(
    length: Length,
    surface: &SurfaceInfo,
    units: UnitSize,
) -> Result<u32, SizingError> {
    length.validate().map_err(|_| SizingError)?;
    let value = length.to_px(surface, units);
    let rounded = if value > 0.0 {
        value.round().max(1.0)
    } else {
        0.0
    };
    if !rounded.is_finite() || f64::from(rounded) > f64::from(u32::MAX) {
        return Err(SizingError);
    }
    Ok(rounded as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use touchbar_layout::{BarSpec, Element, resolve};
    use touchbar_model::{Axis, SurfaceCaps, SurfaceId, SurfaceKind};
    use touchbar_sets::SetFile;

    fn surface(width: u32, height: u32) -> SurfaceInfo {
        SurfaceInfo {
            id: SurfaceId(1),
            name: "headless:sizing".parse().unwrap(),
            kind: SurfaceKind::Headless,
            width_px: width,
            height_px: height,
            px_per_mm: 6.0,
            caps: SurfaceCaps::default(),
        }
    }
    fn item(properties: &str, surface: &SurfaceInfo, units: UnitSize) -> ItemSpec {
        let root = SetFile::from_toml(
            "sizing.toml",
            &format!("version=1\nid='test'\nlabel='Test'\n[layout]\nkind='row'\n{properties}"),
        )
        .unwrap()
        .layout;
        root_item(ItemId::from("slot"), &root, surface, units).unwrap()
    }
    #[test]
    fn source_default_stays_fixed_and_scales_with_surface_units() {
        for (width, height, expected) in [(2008, 60, 160), (2170, 60, 160), (1200, 160, 427)] {
            let surface = surface(width, height);
            let item = item("", &surface, UnitSize::resolve(&surface, None, None));
            assert_eq!(
                (
                    item.min_width,
                    item.preferred_width,
                    item.max_width,
                    item.grow
                ),
                (expected, expected, expected, 0.0)
            );
        }
    }
    #[test]
    fn explicit_width_alone_is_fixed_and_percent_is_surface_relative() {
        let surface = surface(2008, 60);
        let item = item(
            "width='20%w'",
            &surface,
            UnitSize::resolve(&surface, None, None),
        );
        assert_eq!(
            (
                item.min_width,
                item.preferred_width,
                item.max_width,
                item.grow
            ),
            (402, 402, 402, 0.0)
        );
    }
    #[test]
    fn each_constraint_activates_flex_defaults() {
        let surface = surface(2008, 60);
        for properties in ["minimum=100", "maximum='100%w'", "grow=1"] {
            let item = item(
                properties,
                &surface,
                UnitSize::resolve(&surface, None, None),
            );
            assert_eq!(
                (
                    item.min_width,
                    item.preferred_width,
                    item.max_width,
                    item.grow
                ),
                (60, 160, 2008, 1.0)
            );
        }
    }
    #[test]
    fn explicit_constraints_and_custom_units_override_defaults() {
        let surface = surface(2008, 60);
        let item = item(
            "width=200\nminimum=100\nmaximum=300\ngrow=2.5",
            &surface,
            UnitSize {
                value: 2.0,
                axis: Axis::Height,
            },
        );
        assert_eq!(
            (
                item.min_width,
                item.preferred_width,
                item.max_width,
                item.grow
            ),
            (120, 240, 360, 2.5)
        );
    }
    #[test]
    fn basis_is_clamped_to_explicit_limits() {
        let surface = surface(2008, 60);
        let units = UnitSize::resolve(&surface, None, None);
        assert_eq!(item("minimum=300", &surface, units).preferred_width, 180);
        assert_eq!(item("maximum=200", &surface, units).preferred_width, 120);
    }
    #[test]
    fn zero_minimum_can_compress_and_tiny_positive_dimensions_remain_visible() {
        let surface = surface(2008, 60);
        let units = UnitSize::resolve(&surface, None, None);
        let bar = BarSpec {
            elements: vec![Element::Item(item("minimum=0\ngrow=0", &surface, units))],
            principal_item: None,
        };
        assert_eq!(
            resolve(&bar, 0, &BTreeSet::new()).unwrap().placements[0].width,
            0
        );
        assert_eq!(item("width=0.1", &surface, units).preferred_width, 1);
        let zero = item("width=0", &surface, units);
        assert_eq!(zero.preferred_width, 0);
        let bar = BarSpec {
            elements: vec![Element::Item(zero)],
            principal_item: None,
        };
        assert_eq!(
            resolve(&bar, surface.width_px, &BTreeSet::new())
                .unwrap()
                .placements[0]
                .width,
            0
        );
    }
    #[test]
    fn constraints_are_checked_after_unit_conversion() {
        let surface = surface(2008, 60);
        let root = SetFile::from_toml(
            "sizing.toml",
            "version=1\nid='test'\nlabel='Test'\n[layout]\nkind='row'\nminimum='20%w'\nmaximum=200",
        )
        .unwrap()
        .layout;
        assert!(
            root_item(
                ItemId::from("slot"),
                &root,
                &surface,
                UnitSize::resolve(&surface, None, None)
            )
            .is_err()
        );
    }
}
