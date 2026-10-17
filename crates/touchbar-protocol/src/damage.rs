//! Shared damage geometry for the session copy path and hardware presenter.
use crate::{DamageRect, MAX_FRAME_DAMAGE, OutputTransform, SwapchainHeader, invalid};
use std::io;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Damage {
    Full,
    Rects(Vec<DamageRect>),
}
pub fn logical_to_physical(header: SwapchainHeader, rect: DamageRect) -> io::Result<DamageRect> {
    header.validate()?;
    if !rect.fits(header.logical_width, header.logical_height) {
        return Err(invalid("logical damage outside surface"));
    }
    map(header, rect)
}
pub fn physical_to_logical(header: SwapchainHeader, rect: DamageRect) -> io::Result<DamageRect> {
    header.validate()?;
    if !rect.fits(header.physical_width, header.physical_height) {
        return Err(invalid("physical damage outside surface"));
    }
    map(header, rect)
}
fn map(header: SwapchainHeader, rect: DamageRect) -> io::Result<DamageRect> {
    // v1's QuarterTurn exchanges axes (physical X=logical Y, physical Y=logical X).
    match header.transform() {
        Some(OutputTransform::Identity) => Ok(rect),
        Some(OutputTransform::QuarterTurn) => Ok(DamageRect {
            x: rect.y,
            y: rect.x,
            width: rect.height,
            height: rect.width,
        }),
        None => Err(invalid("unsupported output transform")),
    }
}
fn overlap(a: DamageRect, b: DamageRect) -> bool {
    u32::from(a.x) <= u32::from(b.x) + u32::from(b.width)
        && u32::from(b.x) <= u32::from(a.x) + u32::from(a.width)
        && u32::from(a.y) <= u32::from(b.y) + u32::from(b.height)
        && u32::from(b.y) <= u32::from(a.y) + u32::from(a.height)
}
fn union(a: DamageRect, b: DamageRect) -> DamageRect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    DamageRect {
        x,
        y,
        width: u16::try_from(
            (u32::from(a.x) + u32::from(a.width)).max(u32::from(b.x) + u32::from(b.width))
                - u32::from(x),
        )
        .expect("validated surface dimensions"),
        height: u16::try_from(
            (u32::from(a.y) + u32::from(a.height)).max(u32::from(b.y) + u32::from(b.height))
                - u32::from(y),
        )
        .expect("validated surface dimensions"),
    }
}
pub fn merge_damage(rects: &[DamageRect], width: u32, height: u32) -> io::Result<Damage> {
    if width == 0 || height == 0 || width > crate::MAX_DIMENSION || height > crate::MAX_DIMENSION {
        return Err(invalid("invalid damage extent"));
    }
    for rect in rects {
        if !rect.fits(width, height) {
            return Err(invalid("damage outside surface"));
        }
    }
    let mut result: Vec<DamageRect> = Vec::new();
    for rect in rects {
        let mut merged = *rect;
        // Restart when expanded bounds bridge two previously separate rectangles.
        let mut i = 0;
        while i < result.len() {
            if overlap(merged, result[i]) {
                merged = union(merged, result.remove(i));
                i = 0;
            } else {
                i += 1;
            }
        }
        result.push(merged);
        if result.len() > MAX_FRAME_DAMAGE {
            return Ok(Damage::Full);
        }
    }
    result.sort_by_key(|r| (r.y, r.x, r.height, r.width));
    if result.is_empty() || result.len() > MAX_FRAME_DAMAGE {
        Ok(Damage::Full)
    } else {
        Ok(Damage::Rects(result))
    }
}
