//! Bounded signature-directed JSON codec. Message JSON is always an array of arguments.
use serde::{Serialize, Serializer};
use serde_json::{Value as Json, json};
use touchbar_model::{DbusValue, EffectError};
use zbus::zvariant::{self as zv, DynamicType, Signature, Value};
const MAX_JSON: usize = 32768;
const MAX_NODES: usize = 4096;
fn invalid(s: impl Into<String>) -> EffectError {
    EffectError::Failed(s.into())
}
fn cv(e: impl std::fmt::Display) -> EffectError {
    invalid(e.to_string())
}
pub enum Body {
    Empty,
    Fields(zv::Structure<'static>),
}
impl DynamicType for Body {
    fn signature(&self) -> Signature {
        match self {
            Self::Empty => Signature::Unit,
            Self::Fields(s) => s.signature().clone(),
        }
    }
}
impl Serialize for Body {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Empty => ().serialize(s),
            Self::Fields(v) => v.serialize(s),
        }
    }
}
fn signature(s: &str) -> Result<Signature, EffectError> {
    if s.len() > 255 {
        return Err(invalid("signature exceeds 255 bytes"));
    }
    Signature::from_bytes(s.as_bytes()).map_err(cv)
}
fn body_fields(s: &str) -> Result<Vec<Signature>, EffectError> {
    if s.is_empty() {
        return Ok(vec![]);
    }
    let parsed = signature(s)?;
    if parsed.to_string() != s
        && let Signature::Structure(fields) = &parsed
    {
        return Ok(fields.iter().cloned().collect());
    }
    Ok(vec![parsed])
}
fn single(s: &str) -> Result<Signature, EffectError> {
    let fields = body_fields(s)?;
    if fields.len() != 1 {
        return Err(invalid("variant requires exactly one complete type"));
    }
    Ok(fields[0].clone())
}
pub fn encode(input: &DbusValue) -> Result<Body, EffectError> {
    if input.json.len() > MAX_JSON {
        return Err(invalid("D-Bus JSON exceeds 32 KiB"));
    }
    let fields = body_fields(&input.signature)?;
    let json: Json = serde_json::from_str(&input.json).map_err(cv)?;
    let args = json
        .as_array()
        .ok_or_else(|| invalid("message JSON must be an argument array"))?;
    if fields.len() != args.len() {
        return Err(invalid("argument count differs from signature"));
    }
    if fields.is_empty() {
        return Ok(Body::Empty);
    }
    let mut builder = zv::StructureBuilder::new();
    let mut budget = 0;
    for (sig, json) in fields.iter().zip(args) {
        builder = builder.append_field(value(sig, json, 0, &mut budget)?);
    }
    Ok(Body::Fields(builder.build().map_err(cv)?))
}
fn value(
    sig: &Signature,
    j: &Json,
    depth: usize,
    budget: &mut usize,
) -> Result<Value<'static>, EffectError> {
    *budget += 1;
    if depth > 16 || *budget > MAX_NODES {
        return Err(invalid("D-Bus nesting/node budget exceeded"));
    }
    macro_rules! unsigned {
        ($ty:ty) => {
            <$ty>::try_from(
                j.as_u64()
                    .ok_or_else(|| invalid("expected unsigned integer"))?,
            )
            .map(Value::from)
            .map_err(cv)
        };
    }
    macro_rules! signed {
        ($ty:ty) => {
            <$ty>::try_from(
                j.as_i64()
                    .ok_or_else(|| invalid("expected signed integer"))?,
            )
            .map(Value::from)
            .map_err(cv)
        };
    }
    let string = || {
        j.as_str()
            .filter(|s| !s.contains('\0'))
            .ok_or_else(|| invalid("expected D-Bus string without NUL"))
    };
    Ok(match sig {
        Signature::U8 => unsigned!(u8)?,
        Signature::U16 => unsigned!(u16)?,
        Signature::U32 => unsigned!(u32)?,
        Signature::U64 => j.as_u64().ok_or_else(|| invalid("expected u64"))?.into(),
        Signature::I16 => signed!(i16)?,
        Signature::I32 => signed!(i32)?,
        Signature::I64 => j.as_i64().ok_or_else(|| invalid("expected i64"))?.into(),
        Signature::Bool => j.as_bool().ok_or_else(|| invalid("expected bool"))?.into(),
        Signature::F64 => j
            .as_f64()
            .filter(|n| n.is_finite())
            .ok_or_else(|| invalid("expected finite double"))?
            .into(),
        Signature::Str => zv::Str::from(string()?.to_owned()).into(),
        Signature::ObjectPath => zv::ObjectPath::try_from(string()?.to_owned())
            .map_err(cv)?
            .into(),
        Signature::Signature => signature(string()?)?.into(),
        Signature::Variant => {
            let obj = j
                .as_object()
                .filter(|o| o.len() == 2)
                .ok_or_else(|| invalid("variant requires sig and value"))?;
            let sig = single(
                obj.get("sig")
                    .and_then(Json::as_str)
                    .ok_or_else(|| invalid("missing variant sig"))?,
            )?;
            Value::Value(Box::new(value(
                &sig,
                obj.get("value")
                    .ok_or_else(|| invalid("missing variant value"))?,
                depth + 1,
                budget,
            )?))
        }
        Signature::Array(child) => {
            let mut array = zv::Array::new(child.signature());
            for j in j.as_array().ok_or_else(|| invalid("expected array"))? {
                array
                    .append(value(child.signature(), j, depth + 1, budget)?)
                    .map_err(cv)?;
            }
            array.into()
        }
        Signature::Dict { key, value: vsig } => {
            let mut dict = zv::Dict::new(key.signature(), vsig.signature());
            for (k, v) in j
                .as_object()
                .ok_or_else(|| invalid("expected dictionary object"))?
            {
                let kj = match key.signature() {
                    Signature::Str | Signature::ObjectPath | Signature::Signature => {
                        Json::String(k.clone())
                    }
                    _ => serde_json::from_str(k).map_err(cv)?,
                };
                let kval = value(key.signature(), &kj, depth + 1, budget)?;
                if key_string(&kval)? != *k {
                    return Err(invalid("dictionary key is not canonical"));
                }
                dict.append(kval, value(vsig.signature(), v, depth + 1, budget)?)
                    .map_err(cv)?;
            }
            dict.into()
        }
        Signature::Structure(fields) => {
            let args = j
                .as_array()
                .filter(|a| a.len() == fields.len())
                .ok_or_else(|| invalid("structure fields differ from signature"))?;
            let mut s = zv::StructureBuilder::new();
            for (sig, j) in fields.iter().zip(args) {
                s = s.append_field(value(sig, j, depth + 1, budget)?);
            }
            s.build().map_err(cv)?.into()
        }
        Signature::Fd => return Err(EffectError::Unsupported),
        _ => return Err(invalid("unsupported D-Bus type")),
    })
}
fn key_string(value: &Value<'_>) -> Result<String, EffectError> {
    match value {
        Value::Str(s) => Ok(s.to_string()),
        Value::ObjectPath(s) => Ok(s.to_string()),
        Value::Signature(s) => Ok(s.to_string()),
        _ => Ok(to_json(value, 0, &mut 0)?.to_string()),
    }
}
fn to_json(value: &Value<'_>, depth: usize, budget: &mut usize) -> Result<Json, EffectError> {
    *budget += 1;
    if depth > 16 || *budget > MAX_NODES {
        return Err(invalid("D-Bus nesting/node budget exceeded"));
    }
    Ok(match value {
        Value::U8(v) => json!(v),
        Value::U16(v) => json!(v),
        Value::U32(v) => json!(v),
        Value::U64(v) => json!(v),
        Value::I16(v) => json!(v),
        Value::I32(v) => json!(v),
        Value::I64(v) => json!(v),
        Value::Bool(v) => json!(v),
        Value::F64(v) if v.is_finite() => json!(v),
        Value::Str(v) => json!(v.as_str()),
        Value::ObjectPath(v) => json!(v.as_str()),
        Value::Signature(v) => json!(v.to_string()),
        Value::Value(v) => {
            json!({"sig":v.value_signature().to_string(),"value":to_json(v,depth+1,budget)?})
        }
        Value::Array(v) => Json::Array(
            v.inner()
                .iter()
                .map(|v| to_json(v, depth + 1, budget))
                .collect::<Result<_, _>>()?,
        ),
        Value::Structure(v) => Json::Array(
            v.fields()
                .iter()
                .map(|v| to_json(v, depth + 1, budget))
                .collect::<Result<_, _>>()?,
        ),
        Value::Dict(v) => {
            let mut map = serde_json::Map::new();
            for (k, v) in v.iter() {
                let key = key_string(k)?;
                if map.insert(key, to_json(v, depth + 1, budget)?).is_some() {
                    return Err(invalid("duplicate D-Bus dictionary key"));
                }
            }
            Json::Object(map)
        }
        Value::Fd(_) => return Err(EffectError::Unsupported),
        _ => return Err(invalid("non-finite or unsupported D-Bus value")),
    })
}
pub fn decode_message(message: &zbus::Message) -> Result<DbusValue, EffectError> {
    let body = message.body();
    if body.len() > 256 * 1024 {
        return Err(invalid("D-Bus reply exceeds 256 KiB"));
    }
    if body.is_empty() {
        return Ok(DbusValue {
            signature: String::new(),
            json: "[]".into(),
        });
    }
    let fields = body.deserialize::<zv::Structure<'_>>().map_err(cv)?;
    let mut budget = 0;
    let values = fields
        .fields()
        .iter()
        .map(|v| to_json(v, 0, &mut budget))
        .collect::<Result<Vec<_>, _>>()?;
    let json = serde_json::to_string(&values).map_err(cv)?;
    if json.len() > MAX_JSON {
        return Err(invalid("D-Bus JSON exceeds 32 KiB"));
    }
    Ok(DbusValue {
        signature: fields.signature().to_string_no_parens(),
        json,
    })
}
/// Encode a single property value using its own type and a one-element message argument array.
pub fn property(value: &zv::Value<'_>) -> Result<DbusValue, EffectError> {
    let json = serde_json::to_string(&[to_json(value, 0, &mut 0)?]).map_err(cv)?;
    if json.len() > MAX_JSON {
        return Err(invalid("D-Bus JSON exceeds 32 KiB"));
    }
    Ok(DbusValue {
        signature: value.value_signature().to_string(),
        json,
    })
}
