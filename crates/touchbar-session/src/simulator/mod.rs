//! Development surfaces: deterministic input routing and Wayland presentation.
mod input;
use crate::surface::SurfaceEvent;
pub use input::*;
#[derive(Clone, Debug)]
pub enum SimulatorEvent {
    Surface(SurfaceEvent),
    ToggleTheme,
    Fault(String),
}
pub fn physical_size(logical: (u32, u32), scale: u32) -> Result<(u32, u32), String> {
    if logical.0 == 0 || logical.1 == 0 || !(1..=8).contains(&scale) {
        return Err("invalid simulator dimensions or scale".into());
    }
    let width = logical
        .0
        .checked_mul(scale)
        .ok_or("simulator width overflow")?;
    let height = logical
        .1
        .checked_mul(scale)
        .ok_or("simulator height overflow")?;
    if width > 8199
        || height > 8199
        || u64::from(width) * u64::from(height) > 4 * 1024 * 1024 + 16 * 8192 + 64
    {
        return Err("simulator framebuffer exceeds size budget".into());
    }
    Ok((width, height))
}
pub fn argb8888(rgba: &[u8]) -> Result<Vec<u8>, String> {
    if !rgba.len().is_multiple_of(4) {
        return Err("incomplete RGBA pixel".into());
    }
    let mut output = Vec::with_capacity(rgba.len());
    for pixel in rgba.as_chunks::<4>().0 {
        output.extend_from_slice(
            &u32::from_be_bytes([pixel[3], pixel[0], pixel[1], pixel[2]]).to_ne_bytes(),
        );
    }
    Ok(output)
}
mod native;
pub use native::Simulator;

mod desktop;
pub use desktop::SimulatorDesktop;
