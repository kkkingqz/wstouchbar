use serde::Serialize;
use std::io;
use touchbar_protocol::OutputTransform;
#[derive(Clone, Copy, Debug)]
pub struct Measurement<'a> {
    pub model: &'a str,
    pub width_mm100: u32,
    pub height_mm100: u32,
}
/// Populate only from the stand's ruler/connector comparison; guesses are never measurements.
pub const MEASURED_MODELS: &[Measurement<'static>] = &[];
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SizeSource {
    Connector,
    Dmi,
    Fallback,
}
#[derive(Clone, Debug, Serialize)]
pub struct PhysicalSize {
    pub width_mm100: u32,
    pub height_mm100: u32,
    pub source: SizeSource,
    pub warning: Option<String>,
}
pub fn resolve(
    logical: (u32, u32),
    transform: OutputTransform,
    connector_mm: Option<(u32, u32)>,
    model: Option<&str>,
    table: &[Measurement<'_>],
) -> io::Result<PhysicalSize> {
    if logical.0 == 0 || logical.1 == 0 || logical.0 > 16384 || logical.1 > 16384 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid logical dimensions",
        ));
    }
    if let Some((w, h)) = connector_mm.filter(|(w, h)| *w > 0 && *h > 0)
        && let Some((w, h)) = w.checked_mul(100).zip(h.checked_mul(100))
    {
        let (width_mm100, height_mm100) = if transform == OutputTransform::Identity {
            (w, h)
        } else {
            (h, w)
        };
        return Ok(PhysicalSize {
            width_mm100,
            height_mm100,
            source: SizeSource::Connector,
            warning: None,
        });
    }
    if let Some(measured) = table
        .iter()
        .find(|m| Some(m.model) == model && m.width_mm100 > 0 && m.height_mm100 > 0)
    {
        return Ok(PhysicalSize {
            width_mm100: measured.width_mm100,
            height_mm100: measured.height_mm100,
            source: SizeSource::Dmi,
            warning: None,
        });
    }
    Ok(PhysicalSize {
        width_mm100: (logical.0 * 100 + 3) / 6,
        height_mm100: (logical.1 * 100 + 3) / 6,
        source: SizeSource::Fallback,
        warning: Some(format!(
            "no connector size or measured DMI entry for {}; using estimated 6 px/mm",
            model.unwrap_or("unknown model")
        )),
    })
}
