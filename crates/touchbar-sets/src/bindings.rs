use crate::{document::error, *};
use touchbar_model::{ActionKind, FieldKind, LogicDescription, ScalarKind};
type Result<T> = std::result::Result<T, DocumentError>;
impl SetFile {
    /// Checks render-only bindings as well as the interaction subset carried by SetSpec.
    /// Use from_toml_with_description to attach exact source locations to these errors.
    pub fn validate_bindings(&self, description: &LogicDescription) -> Result<()> {
        self.validate()?;
        node(&self.layout, "layout", description, None)?;
        for (i, p) in self.panels.iter().enumerate() {
            node(&p.layout, &format!("panels[{i}].layout"), description, None)?;
        }
        for (i, n) in self.persist.iter().enumerate() {
            if !description.state.iter().any(|f| &f.name == n && f.persist) {
                return Err(error(
                    &format!("persist[{i}]"),
                    "persist field is missing or not declared persist=true",
                ));
            }
        }
        Ok(())
    }
}
fn scalar(
    path: &str,
    name: &str,
    kind: ScalarKind,
    d: &LogicDescription,
    record: Option<&[(String, ScalarKind)]>,
) -> Result<()> {
    let matches = if let Some(name) = name.strip_prefix("record.") {
        record.is_some_and(|r| r.iter().any(|(n, k)| n == name && *k == kind))
    } else {
        d.state
            .iter()
            .any(|f| f.name == name && f.kind == FieldKind::Scalar(kind))
    };
    if matches {
        Ok(())
    } else {
        Err(error(
            path,
            format!("unknown or incompatible state binding {name}"),
        ))
    }
}
fn field<'a>(name: &str, d: &'a LogicDescription) -> Option<&'a FieldKind> {
    d.state.iter().find(|f| f.name == name).map(|f| &f.kind)
}
fn node(
    n: &LayoutNode,
    p: &str,
    d: &LogicDescription,
    record: Option<&[(String, ScalarKind)]>,
) -> Result<()> {
    for (name, b) in n.bindings() {
        let path = format!("{p}.{name}");
        match name {
            "canvas_from" => {
                if field(b, d) != Some(&FieldKind::Canvas) {
                    return Err(error(
                        &path,
                        "canvas binding requires declared Canvas field",
                    ));
                }
            }
            "repeat_from" => {
                if !matches!(field(b, d), Some(FieldKind::Records(_))) {
                    return Err(error(&path, "repeat requires declared Records field"));
                }
            }
            _ => {
                let kind = match name {
                    "value_from" => ScalarKind::Number,
                    "enabled_from" | "visible_from" | "selected_from" | "pressed_from" => {
                        ScalarKind::Boolean
                    }
                    _ => ScalarKind::Text,
                };
                scalar(&path, b, kind, d, record)?;
            }
        }
    }
    for (name, a, kind) in n.actions() {
        let path = format!("{p}.{name}");
        let Some((_, action)) = d.actions.iter().find(|(name, _)| name == a) else {
            return Err(error(&path, format!("unknown action {a}")));
        };
        if !action.kinds.contains(&kind)
            || kind == ActionKind::ValueChanged && action.value != Some(ScalarKind::Number)
            || matches!(action.value, Some(ScalarKind::Boolean | ScalarKind::Text))
        {
            return Err(error(&path, "action does not accept this event/value"));
        }
    }
    let scalar_binding = |s: &ScalarBinding, path: &str| {
        if let ScalarBinding::State(name) = s {
            scalar(path, name, ScalarKind::Number, d, record)?;
        }
        Ok(())
    };
    if let Some(e) = &n.effect {
        for (i, s) in e.parameters.iter().enumerate() {
            scalar_binding(s, &format!("{p}.effect.parameters[{i}]"))?;
        }
    }
    if let Some(m) = &n.motion {
        for (side, t) in [("from", &m.from), ("to", &m.to)] {
            for (name, s) in [
                ("x", &t.x),
                ("y", &t.y),
                ("scale", &t.scale),
                ("opacity", &t.opacity),
            ] {
                if let Some(s) = s {
                    scalar_binding(s, &format!("{p}.motion.{side}.{name}"))?;
                }
            }
        }
    }
    for (i, c) in n.children.iter().enumerate() {
        node(c, &format!("{p}.children[{i}]"), d, record)?;
    }
    for (i, v) in n.variants.iter().enumerate() {
        node(&v.layout, &format!("{p}.variants[{i}].layout"), d, record)?;
    }
    if let Some(t) = &n.template {
        let Some(FieldKind::Records(fields)) =
            n.repeat_from.as_deref().and_then(|name| field(name, d))
        else {
            return Err(error(
                &format!("{p}.repeat_from"),
                "repeat requires declared Records field",
            ));
        };
        if !fields
            .iter()
            .any(|(name, _)| Some(name) == n.repeat_key.as_ref())
        {
            return Err(error(
                &format!("{p}.repeat_key"),
                "repeat key is not a declared record field",
            ));
        }
        node(t, &format!("{p}.template"), d, Some(fields))?;
    }
    Ok(())
}
