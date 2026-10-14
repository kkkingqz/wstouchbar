use std::{sync::Arc, time::Duration};

use anyhow::{Result, anyhow, bail};
use naga::{
    AddressSpace, BinaryOperator, Expression, MathFunction, RelationalFunction, ScalarKind,
    ShaderStage, Statement, TypeInner, UnaryOperator,
    valid::{Capabilities, ValidationFlags, Validator},
};

pub const MAX_EFFECT_SOURCE_BYTES: usize = 4 * 1024;
pub const MAX_EFFECT_TYPES: usize = 64;
pub const MAX_EFFECT_EXPRESSIONS: usize = 256;
pub const MAX_EFFECT_STATEMENTS: usize = 64;
pub const MAX_EFFECT_PROGRAMS: usize = 8;
pub const MAX_EFFECT_NODES: usize = 4;
pub const MAX_EFFECT_PARAMETERS: usize = 8;

pub const ENTRY_POINT: &str = "otb_effect";
pub const UNIFORM_NAME: &str = "otb_uniforms";

const PREFIX: &str = r#"
struct OtbEffectUniforms {
    geometry: vec4<f32>,
    runtime: vec4<f32>,
    background: vec4<f32>,
    control: vec4<f32>,
    control_pressed: vec4<f32>,
    track: vec4<f32>,
    foreground: vec4<f32>,
    muted: vec4<f32>,
    accent: vec4<f32>,
    destructive: vec4<f32>,
    on_accent: vec4<f32>,
    on_destructive: vec4<f32>,
    params0: vec4<f32>,
    params1: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> otb_uniforms: OtbEffectUniforms;

@fragment
fn otb_effect(@builtin(position) otb_position: vec4<f32>) -> @location(0) vec4<f32> {
    let otb_point = vec2<f32>(otb_position.x, otb_uniforms.runtime.y - otb_position.y);
    let uv = (otb_point - otb_uniforms.geometry.xy) / max(otb_uniforms.geometry.zw, vec2<f32>(0.0001));
    let size = otb_uniforms.geometry.zw;
    let time = otb_uniforms.runtime.z;
    let opacity = otb_uniforms.runtime.w;
    let background = otb_uniforms.background;
    let control = otb_uniforms.control;
    let control_pressed = otb_uniforms.control_pressed;
    let track = otb_uniforms.track;
    let foreground = otb_uniforms.foreground;
    let muted = otb_uniforms.muted;
    let accent = otb_uniforms.accent;
    let destructive = otb_uniforms.destructive;
    let on_accent = otb_uniforms.on_accent;
    let on_destructive = otb_uniforms.on_destructive;
    let params0 = otb_uniforms.params0;
    let params1 = otb_uniforms.params1;
"#;

const SUFFIX: &str = r#"
    let otb_invalid = !all(color == color);
    let otb_safe = select(clamp(color, vec4<f32>(0.0), vec4<f32>(1.0)), vec4<f32>(0.0), otb_invalid);
    let otb_alpha = otb_safe.a * opacity;
    return vec4<f32>(otb_safe.rgb * otb_alpha, otb_alpha);
}
"#;

/// A parsed, statically bounded WGSL effect. GL translation belongs to touchbar-gl.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectProgram {
    body: Arc<str>,
    wgsl: Arc<str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShaderEffect {
    pub program: Arc<EffectProgram>,
    pub parameters: [f32; MAX_EFFECT_PARAMETERS],
    pub opacity: f32,
    pub started: Duration,
    pub period: Option<Duration>,
}

impl ShaderEffect {
    pub fn time(&self, now: Duration, motion: crate::MotionPolicy) -> f32 {
        if motion != crate::MotionPolicy::Full {
            return 0.0;
        }
        let Some(period) = self.period.filter(|p| !p.is_zero()) else {
            return 0.0;
        };
        now.saturating_sub(self.started)
            .as_secs_f32()
            .rem_euclid(period.as_secs_f32())
    }

    pub fn is_active(&self, motion: crate::MotionPolicy) -> bool {
        motion == crate::MotionPolicy::Full && self.period.is_some_and(|p| !p.is_zero())
    }
}

impl EffectProgram {
    pub fn compile(body: &str) -> Result<Self> {
        validate_source_envelope(body)?;
        let wrapped = format!("{PREFIX}\n{body}\n{SUFFIX}");
        let module = naga::front::wgsl::parse_str(&wrapped).map_err(|error| {
            anyhow!(
                "effect WGSL parse failed: {}",
                error.emit_to_string(&wrapped)
            )
        })?;
        audit_module(&module)?;
        Validator::new(ValidationFlags::all(), Capabilities::empty())
            .validate(&module)
            .map_err(|error| {
                anyhow!(
                    "effect WGSL validation failed: {}",
                    error.emit_to_string(&wrapped)
                )
            })?;

        Ok(Self {
            body: body.into(),
            wgsl: wrapped.into(),
        })
    }
    pub fn body(&self) -> &Arc<str> {
        &self.body
    }
    pub fn wgsl(&self) -> &str {
        &self.wgsl
    }
}

fn validate_source_envelope(body: &str) -> Result<()> {
    if body.trim().is_empty() {
        bail!("effect body must not be empty");
    }
    if body.len() > MAX_EFFECT_SOURCE_BYTES {
        bail!(
            "effect body is {} bytes; limit is {}",
            body.len(),
            MAX_EFFECT_SOURCE_BYTES
        );
    }
    if !body.is_ascii()
        || body
            .bytes()
            .any(|byte| byte.is_ascii_control() && !matches!(byte, b'\n' | b'\r' | b'\t'))
    {
        bail!("effect body must contain only printable ASCII, tabs, and newlines");
    }
    for forbidden in ["{", "}", "@", "#", "//", "/*", "*/", "otb_"] {
        if body.contains(forbidden) {
            bail!("effect body contains forbidden wrapper syntax `{forbidden}`");
        }
    }
    Ok(())
}

fn audit_module(module: &naga::Module) -> Result<()> {
    if module.types.len() > MAX_EFFECT_TYPES {
        bail!("effect exceeds the type budget");
    }
    for (_, ty) in module.types.iter() {
        match &ty.inner {
            TypeInner::Scalar(scalar) | TypeInner::Vector { scalar, .. } => {
                if scalar.width != 4 || !matches!(scalar.kind, ScalarKind::Float | ScalarKind::Bool)
                {
                    bail!("effect uses a non-f32/non-boolean scalar type");
                }
            }
            TypeInner::Matrix { scalar, .. } => {
                if scalar.width != 4 || scalar.kind != ScalarKind::Float {
                    bail!("effect matrices must contain f32 values");
                }
            }
            TypeInner::Struct { .. } => {}
            _ => bail!("effect uses a forbidden resource, pointer, array, or atomic type"),
        }
    }
    if !module.constants.is_empty()
        || !module.overrides.is_empty()
        || !module.global_expressions.is_empty()
        || !module.functions.is_empty()
        || !module.diagnostic_filters.is_empty()
        || module.diagnostic_filter_leaf.is_some()
    {
        bail!("effect introduced constants, functions, or diagnostic directives");
    }
    if module.global_variables.len() != 1 {
        bail!("effect must use only the fixed uniform block");
    }
    let (_, uniform) = module.global_variables.iter().next().unwrap();
    if uniform.name.as_deref() != Some(UNIFORM_NAME)
        || uniform.space != AddressSpace::Uniform
        || uniform
            .binding
            .as_ref()
            .map(|binding| (binding.group, binding.binding))
            != Some((0, 0))
        || uniform.init.is_some()
    {
        bail!("effect changed the fixed uniform interface");
    }
    let TypeInner::Struct { members, span } = &module.types[uniform.ty].inner else {
        bail!("effect uniform interface is not the fixed struct");
    };
    const MEMBER_NAMES: [&str; 14] = [
        "geometry",
        "runtime",
        "background",
        "control",
        "control_pressed",
        "track",
        "foreground",
        "muted",
        "accent",
        "destructive",
        "on_accent",
        "on_destructive",
        "params0",
        "params1",
    ];
    if *span != 14 * 16 || members.len() != MEMBER_NAMES.len() {
        bail!("effect changed the fixed uniform block size");
    }
    for (index, (member, expected_name)) in members.iter().zip(MEMBER_NAMES).enumerate() {
        if member.name.as_deref() != Some(expected_name)
            || member.offset != u32::try_from(index * 16)?
            || !matches!(
                module.types[member.ty].inner,
                TypeInner::Vector {
                    size: naga::VectorSize::Quad,
                    scalar: naga::Scalar {
                        kind: ScalarKind::Float,
                        width: 4,
                    },
                }
            )
        {
            bail!("effect changed fixed uniform member `{expected_name}`");
        }
    }
    if module.entry_points.len() != 1 {
        bail!("effect must contain exactly the fixed fragment entry point");
    }
    let entry = &module.entry_points[0];
    if entry.name != ENTRY_POINT || entry.stage != ShaderStage::Fragment {
        bail!("effect changed the fixed fragment entry point");
    }
    let function = &entry.function;
    if function.arguments.len() != 1
        || function.arguments[0].name.as_deref() != Some("otb_position")
        || !matches!(
            function.arguments[0].binding,
            Some(naga::Binding::BuiltIn(naga::BuiltIn::Position { .. }))
        )
        || !matches!(
            function
                .result
                .as_ref()
                .and_then(|result| result.binding.as_ref()),
            Some(naga::Binding::Location { location: 0, .. })
        )
    {
        bail!("effect changed the fixed fragment input or output interface");
    }
    if !function.local_variables.is_empty() {
        bail!("effect mutable local variables are forbidden");
    }
    if function.expressions.len() > MAX_EFFECT_EXPRESSIONS {
        bail!("effect exceeds the expression budget");
    }
    if !function
        .named_expressions
        .values()
        .any(|name| name == "color")
    {
        bail!("effect body must define `let color: vec4<f32> = ...;`");
    }
    audit_expressions(function)?;
    audit_statements(&function.body)?;
    Ok(())
}

fn audit_expressions(function: &naga::Function) -> Result<()> {
    for (_, expression) in function.expressions.iter() {
        match expression {
            Expression::Literal(_)
            | Expression::ZeroValue(_)
            | Expression::Compose { .. }
            | Expression::AccessIndex { .. }
            | Expression::Splat { .. }
            | Expression::Swizzle { .. }
            | Expression::FunctionArgument(_)
            | Expression::GlobalVariable(_)
            | Expression::Load { .. }
            | Expression::Select { .. }
            | Expression::Derivative { .. }
            | Expression::Relational {
                fun:
                    RelationalFunction::All
                    | RelationalFunction::Any
                    | RelationalFunction::IsNan
                    | RelationalFunction::IsInf,
                ..
            }
            | Expression::Unary {
                op: UnaryOperator::Negate | UnaryOperator::LogicalNot,
                ..
            }
            | Expression::Binary {
                op:
                    BinaryOperator::Add
                    | BinaryOperator::Subtract
                    | BinaryOperator::Multiply
                    | BinaryOperator::Divide
                    | BinaryOperator::Modulo
                    | BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::LessEqual
                    | BinaryOperator::Greater
                    | BinaryOperator::GreaterEqual
                    | BinaryOperator::LogicalAnd
                    | BinaryOperator::LogicalOr,
                ..
            } => {}
            Expression::Math { fun, .. } if allowed_math(*fun) => {}
            _ => bail!("effect contains a forbidden expression: {expression:?}"),
        }
    }
    Ok(())
}

fn allowed_math(function: MathFunction) -> bool {
    matches!(
        function,
        MathFunction::Abs
            | MathFunction::Min
            | MathFunction::Max
            | MathFunction::Clamp
            | MathFunction::Saturate
            | MathFunction::Cos
            | MathFunction::Sin
            | MathFunction::Tan
            | MathFunction::Acos
            | MathFunction::Asin
            | MathFunction::Atan
            | MathFunction::Atan2
            | MathFunction::Radians
            | MathFunction::Degrees
            | MathFunction::Ceil
            | MathFunction::Floor
            | MathFunction::Round
            | MathFunction::Fract
            | MathFunction::Trunc
            | MathFunction::Exp
            | MathFunction::Exp2
            | MathFunction::Log
            | MathFunction::Log2
            | MathFunction::Pow
            | MathFunction::Dot
            | MathFunction::Cross
            | MathFunction::Distance
            | MathFunction::Length
            | MathFunction::Normalize
            | MathFunction::FaceForward
            | MathFunction::Reflect
            | MathFunction::Refract
            | MathFunction::Sign
            | MathFunction::Mix
            | MathFunction::Step
            | MathFunction::SmoothStep
            | MathFunction::Sqrt
            | MathFunction::InverseSqrt
    )
}

fn audit_statements(block: &naga::Block) -> Result<()> {
    if block.len() > MAX_EFFECT_STATEMENTS {
        bail!(
            "effect has {} straight-line statements; limit is {MAX_EFFECT_STATEMENTS}",
            block.len()
        );
    }
    let mut returns = 0;
    for (index, statement) in block.iter().enumerate() {
        match statement {
            Statement::Emit(_) => {}
            Statement::Return { value: Some(_) } if index + 1 == block.len() => returns += 1,
            _ => bail!("effect contains forbidden control flow or mutation: {statement:?}"),
        }
    }
    if returns != 1 {
        bail!("effect must end in the host-owned return");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
let centered = uv - vec2<f32>(0.5, 0.5);
let wave = 0.5 + 0.5 * sin(centered.x * 18.0 - time * 6.0);
let glow = 1.0 - smoothstep(0.0, 0.45, abs(centered.y - (wave - 0.5) * 0.25));
let color = mix(background, accent, glow * 0.35);
"#;

    fn audit_unchecked_body(body: &str) -> Result<()> {
        let wrapped = format!("{PREFIX}\n{body}\n{SUFFIX}");
        let module = naga::front::wgsl::parse_str(&wrapped).unwrap();
        audit_module(&module)
    }

    #[test]
    fn straight_line_theme_effect_validates_without_gpu() {
        let effect = EffectProgram::compile(VALID).unwrap();
        assert_eq!(effect.body().as_ref(), VALID);
        assert!(effect.wgsl().contains(ENTRY_POINT));
    }

    #[test]
    fn wrapper_escape_and_mutable_state_are_rejected() {
        assert!(EffectProgram::compile("} fn hostile() {} {").is_err());
        assert!(
            EffectProgram::compile(
                "var mutable = 0.5; mutable = mutable + time; let color = vec4<f32>(mutable);"
            )
            .is_err()
        );
    }

    #[test]
    fn control_flow_and_dynamic_indexing_are_rejected() {
        assert!(EffectProgram::compile("loop {} let color = accent;").is_err());
        assert!(
            EffectProgram::compile(
                "let index = i32(time); let value = params0[index]; let color = vec4<f32>(value);"
            )
            .is_err()
        );
        assert!(
            audit_unchecked_body("if time > 0.0 { discard; } let color = accent;")
                .unwrap_err()
                .to_string()
                .contains("forbidden control flow")
        );
        assert!(
            audit_unchecked_body("loop { break; } let color = accent;")
                .unwrap_err()
                .to_string()
                .contains("forbidden control flow")
        );
    }

    #[test]
    fn missing_color_and_expression_bombs_are_rejected() {
        assert!(EffectProgram::compile("let value = accent;").is_err());
        let mut body = String::new();
        for index in 0..90 {
            body.push_str(&format!("let v{index} = sin(time + {index}.0);\n"));
        }
        body.push_str("let color = accent;");
        assert!(body.len() < MAX_EFFECT_SOURCE_BYTES);
        assert!(EffectProgram::compile(&body).is_err());
    }

    #[test]
    fn functions_arrays_resources_and_early_returns_cannot_cross_the_ir_audit() {
        assert!(
            audit_unchecked_body(
                "let samples = array<f32, 2>(0.0, 1.0); let color = vec4<f32>(samples[0]);"
            )
            .is_err()
        );
        assert!(
            audit_unchecked_body("return accent; let color = background;")
                .unwrap_err()
                .to_string()
                .contains("forbidden control flow")
        );

        let source = r#"
fn guest_function(value: f32) -> f32 { return value; }
@fragment
fn otb_effect(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(guest_function(position.x));
}
"#;
        let module = naga::front::wgsl::parse_str(source).unwrap();
        assert!(audit_module(&module).is_err());

        let source = r#"
@group(0) @binding(0) var guest_texture: texture_2d<f32>;
@fragment
fn otb_effect(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(guest_texture, vec2<i32>(position.xy), 0);
}
"#;
        let module = naga::front::wgsl::parse_str(source).unwrap();
        assert!(audit_module(&module).is_err());
    }
}
