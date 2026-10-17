//! Translate the CPU-audited fixed effect interface to GLES3 and reflect its UBO.
use anyhow::{Context, Result, ensure};
use naga::{
    back::glsl,
    valid::{Capabilities, ValidationFlags, Validator},
};
pub(crate) fn translate(body: &str) -> Result<(String, String)> {
    let program = touchbar_ui::EffectProgram::compile(body)?;
    let module = naga::front::wgsl::parse_str(program.wgsl()).context("parse audited WGSL")?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .context("validate effect module")?;
    let mut source = String::new();
    let options = glsl::Options {
        version: glsl::Version::new_gles(300),
        writer_flags: glsl::WriterFlags::empty(),
        binding_map: glsl::BindingMap::default(),
        zero_initialize_workgroup_memory: false,
    };
    let pipeline = glsl::PipelineOptions {
        shader_stage: naga::ShaderStage::Fragment,
        entry_point: touchbar_ui::EFFECT_ENTRY_POINT.into(),
        multiview: None,
    };
    let reflection = glsl::Writer::new(
        &mut source,
        &module,
        &info,
        &options,
        &pipeline,
        naga::proc::BoundsCheckPolicies::default(),
    )
    .context("create GLSL writer")?
    .write()
    .context("translate WGSL to GLSL ES300")?;
    ensure!(
        reflection.uniforms.len() == 1,
        "effect must have one uniform block"
    );
    let uniform = module
        .global_variables
        .iter()
        .find_map(|(h, var)| {
            (var.name.as_deref() == Some(touchbar_ui::EFFECT_UNIFORM_NAME)).then_some(h)
        })
        .context("fixed effect uniform missing")?;
    let block = reflection
        .uniforms
        .get(&uniform)
        .context("uniform reflection missing")?
        .clone();
    ensure!(
        source.starts_with("#version 300 es\n"),
        "GLES300 shader required"
    );
    Ok((source, block))
}
