use crate::{MAX_DIMENSION, VERSION, invalid};
use std::io;
pub const SWAPCHAIN_MAGIC: &[u8; 8] = b"TBARHWD2";
pub const SWAPCHAIN_HEADER_SIZE: usize = 48;
pub const XRGB8888: u32 = 0x34325258;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputTransform {
    Identity,
    QuarterTurn,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwapchainHeader {
    pub logical_width: u32,
    pub logical_height: u32,
    pub physical_width: u32,
    pub physical_height: u32,
    pub format: u32,
    pub pitch: u32,
    pub buffer_size: u64,
    pub buffer_count: u16,
    pub frame_damage: bool,
}
impl SwapchainHeader {
    pub fn transform(self) -> Option<OutputTransform> {
        if (self.logical_width, self.logical_height) == (self.physical_width, self.physical_height)
        {
            Some(OutputTransform::Identity)
        } else if (self.logical_width, self.logical_height)
            == (self.physical_height, self.physical_width)
        {
            Some(OutputTransform::QuarterTurn)
        } else {
            None
        }
    }
    pub fn validate(self) -> io::Result<Self> {
        if !(2..=3).contains(&self.buffer_count) {
            return Err(invalid("swapchain requires 2..3 buffers"));
        }
        if [
            self.logical_width,
            self.logical_height,
            self.physical_width,
            self.physical_height,
        ]
        .iter()
        .any(|v| *v == 0 || *v > MAX_DIMENSION)
            || self.transform().is_none()
        {
            return Err(invalid("invalid dimensions or orientation"));
        }
        if self.format != XRGB8888
            || self.pitch < self.physical_width * 4
            || !self.pitch.is_multiple_of(4)
            || self.pitch > MAX_DIMENSION * 4
        {
            return Err(invalid("invalid XRGB8888 layout"));
        }
        if self.buffer_size < u64::from(self.pitch) * u64::from(self.physical_height)
            || self.buffer_size > 1024 * 1024 * 1024
        {
            return Err(invalid("invalid buffer size"));
        }
        Ok(self)
    }
    pub fn encode(self) -> io::Result<[u8; 48]> {
        self.validate()?;
        let mut b = [0; 48];
        b[..8].copy_from_slice(SWAPCHAIN_MAGIC);
        b[8..10].copy_from_slice(&VERSION.to_le_bytes());
        b[10..12].copy_from_slice(&self.buffer_count.to_le_bytes());
        for (offset, value) in [
            (12, self.logical_width),
            (16, self.logical_height),
            (20, self.physical_width),
            (24, self.physical_height),
            (28, self.format),
            (32, self.pitch),
            (36, u32::from(self.frame_damage)),
        ] {
            b[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        b[40..48].copy_from_slice(&self.buffer_size.to_le_bytes());
        Ok(b)
    }
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() != 48
            || &bytes[..8] != SWAPCHAIN_MAGIC
            || bytes[8..10] != VERSION.to_le_bytes()
        {
            return Err(invalid("invalid swapchain header"));
        }
        let word =
            |i| u32::from_le_bytes(bytes[i..i + 4].try_into().expect("checked header length"));
        if word(36) > 1 {
            return Err(invalid("unknown swapchain flags"));
        }
        Self {
            buffer_count: u16::from_le_bytes(bytes[10..12].try_into().expect("two bytes")),
            logical_width: word(12),
            logical_height: word(16),
            physical_width: word(20),
            physical_height: word(24),
            format: word(28),
            pitch: word(32),
            frame_damage: word(36) == 1,
            buffer_size: u64::from_le_bytes(bytes[40..48].try_into().expect("eight bytes")),
        }
        .validate()
    }
}
