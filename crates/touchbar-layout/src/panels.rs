use super::LayoutError;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelPlacement {
    Region,
    Full,
    Anchored,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelSizing {
    pub minimum: u32,
    pub preferred: u32,
    pub maximum: u32,
}
/// Regions are explicit core geometry (D50). Anchored panels use the original
/// widget's horizontal center and the owner's vertical extent, clamped to the
/// surface, as in the original project's resolve_popover.
pub fn panel_bounds(
    surface: PixelRect,
    owner: PixelRect,
    anchor: PixelRect,
    placement: PanelPlacement,
    sizing: PanelSizing,
) -> Result<PixelRect, LayoutError> {
    if surface.width == 0
        || surface.height == 0
        || !contains(surface, owner)
        || (placement == PanelPlacement::Anchored && !contains(surface, anchor))
    {
        return Err(LayoutError::InvalidPanelGeometry);
    }
    if sizing.minimum > sizing.preferred || sizing.preferred > sizing.maximum || sizing.maximum == 0
    {
        return Err(LayoutError::InvalidPanelSizing);
    }
    match placement {
        PanelPlacement::Region => Ok(owner),
        PanelPlacement::Full => Ok(surface),
        PanelPlacement::Anchored => {
            if sizing.minimum > surface.width {
                return Err(LayoutError::InsufficientWidth {
                    available: surface.width,
                    required: sizing.minimum,
                });
            }
            let width = sizing.preferred.min(surface.width);
            let center = u64::from(anchor.x) + u64::from(anchor.width) / 2;
            let left = center
                .saturating_sub(u64::from(width) / 2)
                .max(u64::from(surface.x));
            let right = u64::from(surface.x) + u64::from(surface.width) - u64::from(width);
            let x =
                u32::try_from(left.min(right)).map_err(|_| LayoutError::InvalidPanelGeometry)?;
            Ok(PixelRect {
                x,
                y: owner.y,
                width,
                height: owner.height,
            })
        }
    }
}
fn contains(outer: PixelRect, inner: PixelRect) -> bool {
    inner.width > 0
        && inner.height > 0
        && inner.x >= outer.x
        && inner.y >= outer.y
        && u64::from(inner.x) + u64::from(inner.width)
            <= u64::from(outer.x) + u64::from(outer.width)
        && u64::from(inner.y) + u64::from(inner.height)
            <= u64::from(outer.y) + u64::from(outer.height)
}
