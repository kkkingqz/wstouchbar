use crate::{ModelError, SurfaceInfo};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    Width,
    Height,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    Unit(f32),
    Percent { value: f32, axis: Axis },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitSize {
    pub value: f32,
    pub axis: Axis,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mm(#[serde(with = "crate::wire_float::f32")] pub f32);
impl Mm {
    pub fn to_px(self, px_per_mm: f32) -> f32 {
        self.0 * px_per_mm
    }
}
impl Axis {
    #[allow(clippy::cast_precision_loss)]
    fn side(self, surface: &SurfaceInfo) -> f32 {
        match self {
            Self::Width => surface.width_px as f32,
            Self::Height => surface.height_px as f32,
        }
    }
    fn suffix(self) -> &'static str {
        match self {
            Self::Width => "%w",
            Self::Height => "%h",
        }
    }
}
impl Length {
    pub fn validate(self) -> Result<(), ModelError> {
        let (value, limit) = match self {
            Self::Unit(v) => (v, 10_000.0),
            Self::Percent { value, .. } => (value, 100.0),
        };
        if !value.is_finite() || !(0.0..=limit).contains(&value) {
            Err(ModelError(format!(
                "length must be finite and in 0..={limit}"
            )))
        } else {
            Ok(())
        }
    }
    pub fn to_px(self, surface: &SurfaceInfo, unit: UnitSize) -> f32 {
        match self {
            Self::Unit(value) => value * unit.value / 100.0 * unit.axis.side(surface),
            Self::Percent { value, axis } => value / 100.0 * axis.side(surface),
        }
    }
}
impl UnitSize {
    pub fn validate(self) -> Result<(), ModelError> {
        if !self.value.is_finite() || self.value <= 0.0 || self.value > 100.0 {
            Err(ModelError(
                "unit size must be finite, positive and at most 100%".into(),
            ))
        } else {
            Ok(())
        }
    }
    pub fn resolve(surface: &SurfaceInfo, set: Option<Self>, profile: Option<Self>) -> Self {
        set.or(profile).unwrap_or(Self {
            value: 1.0,
            axis: if surface.width_px < surface.height_px {
                Axis::Width
            } else {
                Axis::Height
            },
        })
    }
}
impl Serialize for Length {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.validate().map_err(serde::ser::Error::custom)?;
        match *self {
            Self::Unit(v) => crate::wire_float::f32::serialize(&v, s),
            Self::Percent { value, axis } => s.serialize_str(&format!("{value}{}", axis.suffix())),
        }
    }
}
impl<'de> Deserialize<'de> for Length {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = Length;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a nonnegative length N, Nu, N%w or N%h")
            }
            #[allow(clippy::cast_possible_truncation)]
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Length, E> {
                from_number(v, None)
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Length, E> {
                self.visit_f64(v as f64)
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Length, E> {
                self.visit_f64(v as f64)
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Length, E> {
                for (suffix, axis) in [
                    ("%w", Some(Axis::Width)),
                    ("%h", Some(Axis::Height)),
                    ("u", None),
                ] {
                    if let Some(number) = s.strip_suffix(suffix) {
                        let value: f64 = number.parse().map_err(E::custom)?;
                        return from_number(value, axis);
                    }
                }
                Err(E::custom("unknown length unit"))
            }
        }
        #[allow(clippy::cast_possible_truncation)]
        fn from_number<E: de::Error>(v: f64, axis: Option<Axis>) -> Result<Length, E> {
            let limit = if axis.is_some() { 100.0 } else { 10_000.0 };
            if !v.is_finite() || !(0.0..=limit).contains(&v) {
                return Err(E::custom("length is outside permitted bounds"));
            }
            let value = v as f32;
            Ok(match axis {
                Some(axis) => Length::Percent { value, axis },
                None => Length::Unit(value),
            })
        }
        d.deserialize_any(Visitor)
    }
}
impl Serialize for UnitSize {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.validate().map_err(serde::ser::Error::custom)?;
        Length::Percent {
            value: self.value,
            axis: self.axis,
        }
        .serialize(s)
    }
}
impl<'de> Deserialize<'de> for UnitSize {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let Length::Percent { value, axis } = Length::deserialize(d)? else {
            return Err(de::Error::custom("unit size must use %w or %h"));
        };
        let unit = Self { value, axis };
        unit.validate().map_err(de::Error::custom)?;
        Ok(unit)
    }
}
