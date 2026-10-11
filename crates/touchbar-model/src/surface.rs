use crate::{FactValue, Facts, ModelError, SurfaceId};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SurfaceKind {
    Touchbar,
    Window,
    Headless,
}
impl SurfaceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Touchbar => "touchbar",
            Self::Window => "window",
            Self::Headless => "headless",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SurfaceName(String);
impl SurfaceName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl FromStr for SurfaceName {
    type Err = ModelError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "touchbar"
            || ["window:", "headless:"].iter().any(|prefix| {
                s.strip_prefix(prefix).is_some_and(|name| {
                    !name.is_empty()
                        && name
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                })
            })
        {
            Ok(Self(s.into()))
        } else {
            Err(ModelError("invalid surface name".into()))
        }
    }
}
impl TryFrom<String> for SurfaceName {
    type Error = ModelError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}
impl From<SurfaceName> for String {
    fn from(s: SurfaceName) -> Self {
        s.0
    }
}
impl fmt::Display for SurfaceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceCaps {
    pub modes: bool,
    pub backlight: bool,
    pub watch_keys: bool,
    pub inject_keys: bool,
    pub damage: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfaceInfo {
    pub id: SurfaceId,
    pub name: SurfaceName,
    pub kind: SurfaceKind,
    pub width_px: u32,
    pub height_px: u32,
    #[serde(with = "crate::wire_float::f32")]
    pub px_per_mm: f32,
    pub caps: SurfaceCaps,
}
impl SurfaceInfo {
    pub fn validate(&self) -> Result<(), ModelError> {
        if self.width_px == 0
            || self.height_px == 0
            || !self.px_per_mm.is_finite()
            || self.px_per_mm <= 0.0
        {
            return Err(ModelError(
                "surface dimensions and density must be positive and finite".into(),
            ));
        }
        Ok(())
    }
    pub fn facts(&self) -> Facts {
        let width = f64::from(self.width_px);
        let height = f64::from(self.height_px);
        let density = f64::from(self.px_per_mm);
        Facts::from([
            (
                "surface.kind".into(),
                FactValue::Text(self.kind.as_str().into()),
            ),
            (
                "surface.width-mm".into(),
                FactValue::Number(width / density),
            ),
            (
                "surface.height-mm".into(),
                FactValue::Number(height / density),
            ),
            ("surface.aspect".into(), FactValue::Number(width / height)),
        ])
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerSurfaces {
    #[default]
    Source,
    Names(Vec<SurfaceName>),
    All,
}
