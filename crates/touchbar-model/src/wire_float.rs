//! JSON-compatible float encoding for diagnostic inputs, including rejected values.
//! Finite values are numbers; nonfinite values carry their exact IEEE bits.
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

macro_rules! codec {
    ($module:ident, $float:ty, $bits:ty, $digits:expr, $tag:literal) => {
        pub mod $module {
            use super::*;
            pub fn serialize<S: Serializer>(
                value: &$float,
                serializer: S,
            ) -> Result<S::Ok, S::Error> {
                if value.is_finite() {
                    value.serialize(serializer)
                } else {
                    serializer.serialize_str(&format!(
                        concat!($tag, ":{:0width$x}"),
                        value.to_bits(),
                        width = $digits
                    ))
                }
            }
            pub fn deserialize<'de, D: Deserializer<'de>>(
                deserializer: D,
            ) -> Result<$float, D::Error> {
                struct Visitor;
                impl<'de> de::Visitor<'de> for Visitor {
                    type Value = $float;
                    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        f.write_str(concat!(
                            "a finite number or ",
                            $tag,
                            ": followed by nonfinite IEEE bits"
                        ))
                    }
                    #[allow(clippy::cast_possible_truncation)]
                    fn visit_f64<E: de::Error>(self, value: f64) -> Result<$float, E> {
                        let value = value as $float;
                        if value.is_finite() {
                            Ok(value)
                        } else {
                            Err(E::custom("nonfinite numbers require tagged IEEE bits"))
                        }
                    }
                    fn visit_i64<E: de::Error>(self, value: i64) -> Result<$float, E> {
                        self.visit_f64(value as f64)
                    }
                    fn visit_u64<E: de::Error>(self, value: u64) -> Result<$float, E> {
                        self.visit_f64(value as f64)
                    }
                    fn visit_str<E: de::Error>(self, value: &str) -> Result<$float, E> {
                        let bits = value
                            .strip_prefix(concat!($tag, ":"))
                            .filter(|v| {
                                v.len() == $digits && v.bytes().all(|c| c.is_ascii_hexdigit())
                            })
                            .ok_or_else(|| E::custom("invalid tagged IEEE bits"))?;
                        let bits = <$bits>::from_str_radix(bits, 16).map_err(E::custom)?;
                        let number = <$float>::from_bits(bits);
                        if number.is_finite() {
                            Err(E::custom("finite values must be encoded as numbers"))
                        } else {
                            Ok(number)
                        }
                    }
                }
                deserializer.deserialize_any(Visitor)
            }
        }
    };
}
codec!(f32, f32, u32, 8, "f32");
codec!(f64, f64, u64, 16, "f64");

pub mod option_f64 {
    use super::*;
    #[derive(Serialize, Deserialize)]
    struct Number(#[serde(with = "super::f64")] f64);
    pub fn serialize<S: Serializer>(value: &Option<f64>, serializer: S) -> Result<S::Ok, S::Error> {
        value.map(Number).serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<f64>, D::Error> {
        Option::<Number>::deserialize(deserializer).map(|value| value.map(|number| number.0))
    }
}
