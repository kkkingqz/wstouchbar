//! Common pure TOML diagnostics and declarative conditions for document formats.
use crate::DocumentError;
pub use crate::document::{error, parse as parse_document};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use touchbar_model::{Condition, FactName, FactValue};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct When(pub Value);
impl When {
    pub fn to_condition(&self, path: &str) -> Result<Condition, DocumentError> {
        let c = condition(&self.0, path, 1, &mut 0)?;
        c.validate().map_err(|e| error(path, e.0))?;
        Ok(c)
    }
}
fn condition(
    value: &Value,
    path: &str,
    depth: usize,
    nodes: &mut usize,
) -> Result<Condition, DocumentError> {
    *nodes += 1;
    if depth > 8 || *nodes > 128 {
        return Err(error(path, "condition exceeds depth 8 or node budget 128"));
    }
    let map = value
        .as_object()
        .filter(|m| !m.is_empty())
        .ok_or_else(|| error(path, "condition requires a nonempty table"))?;
    let logical = map
        .keys()
        .any(|k| matches!(k.as_str(), "exists" | "not" | "all" | "any"));
    if logical {
        if map.len() != 1 {
            return Err(error(
                path,
                "logical node must contain exactly one operator",
            ));
        }
        let (key, value) = map.iter().next().expect("nonempty table");
        let p = format!("{path}.{key}");
        return match key.as_str() {
            "exists" => {
                let name = value
                    .as_str()
                    .ok_or_else(|| error(&p, "exists requires a fact name"))?;
                fact_name(name, &p)?;
                Ok(Condition::Exists(FactName::from(name)))
            }
            "not" => Ok(Condition::Not(Box::new(condition(
                value,
                &p,
                depth + 1,
                nodes,
            )?))),
            "all" | "any" => {
                let values = value
                    .as_array()
                    .filter(|a| !a.is_empty())
                    .ok_or_else(|| error(&p, "all/any requires a nonempty array of conditions"))?;
                let mut children = Vec::new();
                for (i, v) in values.iter().enumerate() {
                    children.push(condition(v, &format!("{p}[{i}]"), depth + 1, nodes)?);
                }
                Ok(if key == "all" {
                    Condition::All(children)
                } else {
                    Condition::Any(children)
                })
            }
            _ => unreachable!("logical single-key node"),
        };
    }
    let implicit = map.len() > 1;
    let mut children = Vec::new();
    for (name, value) in map {
        let p = format!("{path}.{name}");
        fact_name(name, &p)?;
        if implicit {
            *nodes += 1;
            if depth + 1 > 8 || *nodes > 128 {
                return Err(error(&p, "condition exceeds depth 8 or node budget 128"));
            }
        }
        let name = FactName::from(name.clone());
        children.push(if let Some(op) = value.as_object() {
            if op.len() != 1 || !op.contains_key("in") {
                return Err(error(&p, "fact table requires only in"));
            }
            let p = format!("{p}.in");
            let values = op["in"]
                .as_array()
                .filter(|a| !a.is_empty())
                .ok_or_else(|| error(&p, "in requires a nonempty array"))?;
            let values = values
                .iter()
                .enumerate()
                .map(|(i, v)| fact_value(v, &format!("{p}[{i}]")))
                .collect::<Result<Vec<_>, _>>()?;
            Condition::In { name, values }
        } else {
            Condition::Eq {
                name,
                value: fact_value(value, &p)?,
            }
        });
    }
    if implicit {
        Ok(Condition::All(children))
    } else {
        Ok(children.pop().expect("nonempty fact table"))
    }
}
fn fact_name(name: &str, path: &str) -> Result<(), DocumentError> {
    if name.is_empty()
        || name.len() > 128
        || name.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(error(
            path,
            "fact name must be nonempty, bounded and contain no whitespace/control characters",
        ));
    }
    Ok(())
}
fn fact_value(value: &Value, path: &str) -> Result<FactValue, DocumentError> {
    match value {
        Value::Bool(v) => Ok(FactValue::Boolean(*v)),
        Value::String(v) => Ok(FactValue::Text(v.clone())),
        Value::Number(v) => v
            .as_f64()
            .filter(|n| n.is_finite())
            .map(FactValue::Number)
            .ok_or_else(|| error(path, "fact number must be finite")),
        Value::Array(v) => Ok(FactValue::List(
            v.iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| error(path, "fact list requires strings"))
                })
                .collect::<Result<_, _>>()?,
        )),
        _ => Err(error(
            path,
            "fact value must be boolean, number, text or string list",
        )),
    }
}
/// Attach TOML source locations to validation errors discovered after parsing,
/// for example while resolving includes across several documents.
pub fn locate_error(file: &std::path::Path, source: &str, error: DocumentError) -> DocumentError {
    crate::document::locate_error(file, source, error)
}
