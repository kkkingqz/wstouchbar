//! Bounds-checked Linux drm_event_vblank decoding, without raw pointer casts.
use std::io;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageFlip {
    pub crtc: u32,
    pub sequence: u32,
    pub timestamp_us: u64,
}
pub fn decode_events(bytes: &[u8]) -> io::Result<Vec<PageFlip>> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "malformed DRM event");
    let mut result = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let header = bytes.get(offset..offset + 8).ok_or_else(invalid)?;
        let word = |data: &[u8]| u32::from_ne_bytes(data.try_into().expect("four checked bytes"));
        let kind = word(&header[..4]);
        let len = usize::try_from(word(&header[4..])).map_err(|_| invalid())?;
        if len < 8 {
            return Err(invalid());
        }
        let end = offset.checked_add(len).ok_or_else(invalid)?;
        let event = bytes.get(offset..end).ok_or_else(invalid)?;
        if kind == 2 {
            if len < 32 {
                return Err(invalid());
            }
            let micros = word(&event[20..24]);
            if micros >= 1_000_000 {
                return Err(invalid());
            }
            result.push(PageFlip {
                crtc: word(&event[28..32]),
                sequence: word(&event[24..28]),
                timestamp_us: u64::from(word(&event[16..20])) * 1_000_000 + u64::from(micros),
            });
        }
        offset = end;
    }
    Ok(result)
}
