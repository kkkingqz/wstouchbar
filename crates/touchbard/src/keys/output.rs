use super::system_code;
use crate::input::EVENT_SIZE;
use std::{
    collections::BTreeSet,
    io::{self, Write},
};
use touchbar_model::{KeyPhase, SystemKey};
pub const UINPUT_NAME: &str = "TouchBar System Keys";
/// Owns the output device. Closing it on any write failure releases kernel-held
/// keys even if the failing fd cannot accept explicit releases anymore.
pub struct SystemKeys<W: Write> {
    writer: Option<W>,
    held: BTreeSet<SystemKey>,
}
pub(crate) fn encode(kind: u16, code: u16, value: i32) -> [u8; EVENT_SIZE] {
    let mut b = [0; EVENT_SIZE];
    let p = EVENT_SIZE - 8;
    b[p..p + 2].copy_from_slice(&kind.to_ne_bytes());
    b[p + 2..p + 4].copy_from_slice(&code.to_ne_bytes());
    b[p + 4..].copy_from_slice(&value.to_ne_bytes());
    b
}
impl<W: Write> SystemKeys<W> {
    pub fn from_writer(writer: W) -> Self {
        Self {
            writer: Some(writer),
            held: BTreeSet::new(),
        }
    }
    pub fn send(&mut self, key: SystemKey, phase: KeyPhase) -> io::Result<()> {
        let Some(writer) = self.writer.as_mut() else {
            return Err(io::ErrorKind::NotConnected.into());
        };
        let down = phase == KeyPhase::Pressed;
        if self.held.contains(&key) == down {
            return Ok(());
        }
        let mut bytes = [0; EVENT_SIZE * 2];
        bytes[..EVENT_SIZE].copy_from_slice(&encode(1, system_code(key), i32::from(down)));
        bytes[EVENT_SIZE..].copy_from_slice(&encode(0, 0, 0));
        if let Err(error) = writer.write_all(&bytes) {
            self.writer.take();
            self.held.clear();
            return Err(error);
        }
        if down {
            self.held.insert(key);
        } else {
            self.held.remove(&key);
        }
        Ok(())
    }
    pub fn release_all(&mut self) -> io::Result<()> {
        while let Some(key) = self.held.first().copied() {
            self.send(key, KeyPhase::Released)?;
        }
        Ok(())
    }
}
impl<W: Write> Drop for SystemKeys<W> {
    fn drop(&mut self) {
        let _ = self.release_all();
    }
}
