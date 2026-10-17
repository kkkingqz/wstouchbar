use crate::{
    Color, EffectProgram, ImageColoring, MotionPolicy, Point, Primitive, Rect, Scene, Theme,
    VisualTransform,
};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

/// A complete, portable frame. Resource keys are SHA-256 of tagged content;
/// cache-local image IDs never become part of the packet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayList {
    pub clear: Color,
    pub commands: Vec<DrawCommand>,
    pub resources: BTreeMap<String, Resource>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Resource {
    Raster {
        width: u32,
        height: u32,
        pixels: Arc<[u8]>,
    },
    Shader {
        body: Arc<str>,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DrawCommand {
    RoundedRect {
        rect: Rect,
        radius: f32,
        color: Color,
    },
    Image {
        rect: Rect,
        resource: String,
        opacity: f32,
        coloring: ImageColoring,
    },
    PushOpacity {
        opacity: f32,
    },
    PopOpacity,
    PushTransform {
        origin: Point,
        transform: VisualTransform,
    },
    PopTransform,
    ShaderEffect {
        rect: Rect,
        resource: String,
        uniforms: EffectUniforms,
    },
    Line {
        start: Point,
        end: Point,
        width: f32,
        color: Color,
    },
    LinearGradientRect {
        rect: Rect,
        radius: f32,
        start: Point,
        end: Point,
        start_color: Color,
        end_color: Color,
    },
    TriangleMesh {
        triangles: Arc<[Point]>,
        color: Color,
    },
    PushClip {
        rect: Rect,
    },
    PopClip,
}
/// Viewport dimensions and effective parent opacity are supplied by GL when
/// packing its fixed uniform block; all scene-specific values live here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectUniforms {
    pub theme: Theme,
    pub parameters: [f32; 8],
    pub time: f32,
    pub opacity: f32,
}

impl Resource {
    pub fn key(&self) -> String {
        let mut hash = Sha256::new();
        match self {
            Self::Raster {
                width,
                height,
                pixels,
            } => {
                hash.update(b"raster-rgba8\0");
                hash.update(width.to_le_bytes());
                hash.update(height.to_le_bytes());
                hash.update(pixels);
            }
            Self::Shader { body } => {
                hash.update(b"shader-wgsl-body\0");
                hash.update(body.as_bytes());
            }
        }
        format!("{:x}", hash.finalize())
    }
}
impl DisplayList {
    /// Text and icons must first be materialized with CpuPainter. This boundary
    /// ensures GL never shapes text, reads fonts or discovers external assets.
    pub fn from_scene(scene: &Scene, now: Duration, policy: MotionPolicy) -> Result<Self> {
        let mut list = Self {
            clear: scene.clear,
            commands: Vec::with_capacity(scene.primitives.len()),
            resources: BTreeMap::new(),
        };
        for p in &scene.primitives {
            list.commands.push(match p {
                Primitive::RoundedRect {
                    rect,
                    radius,
                    color,
                } => DrawCommand::RoundedRect {
                    rect: physical_rect(*rect)?,
                    radius: *radius,
                    color: *color,
                },
                Primitive::Image {
                    rect,
                    image,
                    opacity,
                    coloring,
                } => {
                    let bytes = raster_bytes(image.width, image.height)?;
                    if image.pixels.len() != bytes {
                        bail!("raster payload does not match dimensions");
                    }
                    let resource = Resource::Raster {
                        width: image.width,
                        height: image.height,
                        pixels: image.pixels.clone(),
                    };
                    let key = resource.key();
                    list.resources.entry(key.clone()).or_insert(resource);
                    DrawCommand::Image {
                        rect: physical_rect(*rect)?,
                        resource: key,
                        opacity: *opacity,
                        coloring: *coloring,
                    }
                }
                Primitive::PushOpacity { opacity } => {
                    DrawCommand::PushOpacity { opacity: *opacity }
                }
                Primitive::PopOpacity => DrawCommand::PopOpacity,
                Primitive::PushMotion { origin, motion } => DrawCommand::PushTransform {
                    origin: *origin,
                    transform: motion.sample(now, policy),
                },
                Primitive::PopMotion => DrawCommand::PopTransform,
                Primitive::ShaderEffect {
                    rect,
                    effect,
                    theme,
                } => {
                    let resource = Resource::Shader {
                        body: effect.program.body().clone(),
                    };
                    let key = resource.key();
                    list.resources.entry(key.clone()).or_insert(resource);
                    DrawCommand::ShaderEffect {
                        rect: physical_rect(*rect)?,
                        resource: key,
                        uniforms: EffectUniforms {
                            theme: *theme,
                            parameters: effect.parameters,
                            time: effect.time(now, policy),
                            opacity: effect.opacity,
                        },
                    }
                }
                Primitive::Line {
                    start,
                    end,
                    width,
                    color,
                } => DrawCommand::Line {
                    start: *start,
                    end: *end,
                    width: *width,
                    color: *color,
                },
                Primitive::LinearGradientRect {
                    rect,
                    radius,
                    start,
                    end,
                    start_color,
                    end_color,
                } => DrawCommand::LinearGradientRect {
                    rect: physical_rect(*rect)?,
                    radius: *radius,
                    start: *start,
                    end: *end,
                    start_color: *start_color,
                    end_color: *end_color,
                },
                Primitive::TriangleMesh { triangles, color } => DrawCommand::TriangleMesh {
                    triangles: triangles.clone(),
                    color: *color,
                },
                Primitive::PushClip { rect } => DrawCommand::PushClip {
                    rect: physical_rect(*rect)?,
                },
                Primitive::PopClip => DrawCommand::PopClip,
                Primitive::Text { .. } | Primitive::Icon { .. } => {
                    bail!("text and icons require CPU materialization")
                }
            });
        }
        list.validate()?;
        Ok(list)
    }
    /// Call after deserializing and before GPU use. Reject invalid resources,
    /// numeric values and unbalanced groups rather than relying on GL behavior.
    /// This also accepts a surface composed of multiple validated instances.
    pub fn validate(&self) -> Result<()> {
        color(self.clear)?;
        for (key, resource) in &self.resources {
            if *key != resource.key() {
                bail!("resource content hash mismatch");
            }
            match resource {
                Resource::Raster {
                    width,
                    height,
                    pixels,
                } => {
                    if pixels.len() != raster_bytes(*width, *height)? {
                        bail!("invalid raster payload");
                    }
                }
                Resource::Shader { body } => {
                    EffectProgram::compile(body)?;
                }
            }
        }
        let mut stack = Vec::new();
        for command in &self.commands {
            match command {
                DrawCommand::RoundedRect {
                    rect,
                    radius,
                    color: c,
                } => {
                    rectangle(*rect)?;
                    nonnegative(*radius)?;
                    color(*c)?;
                }
                DrawCommand::Image {
                    rect,
                    resource,
                    opacity,
                    coloring,
                } => {
                    rectangle(*rect)?;
                    unit(*opacity)?;
                    if !matches!(self.resources.get(resource), Some(Resource::Raster { .. })) {
                        bail!("missing raster resource {resource}");
                    }
                    if let ImageColoring::Mask(c) | ImageColoring::Multiply(c) = coloring {
                        color(*c)?;
                    }
                }
                DrawCommand::PushOpacity { opacity } => {
                    unit(*opacity)?;
                    stack.push(0);
                }
                DrawCommand::PopOpacity => pop(&mut stack, 0)?,
                DrawCommand::PushTransform { origin, transform } => {
                    point(*origin)?;
                    point(transform.translation)?;
                    nonnegative(transform.scale)?;
                    unit(transform.opacity)?;
                    stack.push(1);
                }
                DrawCommand::PopTransform => pop(&mut stack, 1)?,
                DrawCommand::ShaderEffect {
                    rect,
                    resource,
                    uniforms,
                } => {
                    rectangle(*rect)?;
                    if !matches!(self.resources.get(resource), Some(Resource::Shader { .. })) {
                        bail!("missing shader resource {resource}");
                    }
                    for p in uniforms.parameters {
                        finite(p)?;
                    }
                    nonnegative(uniforms.time)?;
                    unit(uniforms.opacity)?;
                    let t = uniforms.theme;
                    for c in [
                        t.background,
                        t.control,
                        t.control_pressed,
                        t.accent,
                        t.track,
                        t.foreground,
                        t.muted,
                        t.destructive,
                    ] {
                        color(c)?;
                    }
                    nonnegative(t.corner_radius)?;
                }
                DrawCommand::Line {
                    start,
                    end,
                    width,
                    color: c,
                } => {
                    point(*start)?;
                    point(*end)?;
                    nonnegative(*width)?;
                    color(*c)?;
                }
                DrawCommand::LinearGradientRect {
                    rect,
                    radius,
                    start,
                    end,
                    start_color,
                    end_color,
                } => {
                    rectangle(*rect)?;
                    nonnegative(*radius)?;
                    point(*start)?;
                    point(*end)?;
                    color(*start_color)?;
                    color(*end_color)?;
                }
                DrawCommand::TriangleMesh {
                    triangles,
                    color: c,
                } => {
                    if !triangles.len().is_multiple_of(3) {
                        bail!("incomplete triangle");
                    }
                    for p in triangles.iter() {
                        point(*p)?;
                    }
                    color(*c)?;
                }
                DrawCommand::PushClip { rect } => {
                    rectangle(*rect)?;
                    stack.push(2);
                }
                DrawCommand::PopClip => pop(&mut stack, 2)?,
            }
        }
        if !stack.is_empty() {
            bail!("unclosed display group");
        }
        Ok(())
    }

    /// Apply plugin quotas before merging this instance into a surface frame.
    pub fn validate_instance(&self) -> Result<()> {
        self.validate()?;
        if self
            .resources
            .values()
            .filter(|r| matches!(r, Resource::Shader { .. }))
            .count()
            > crate::MAX_EFFECT_PROGRAMS
        {
            bail!("too many shader programs in instance");
        }
        if self
            .commands
            .iter()
            .filter(|c| matches!(c, DrawCommand::ShaderEffect { .. }))
            .count()
            > crate::MAX_EFFECT_NODES
        {
            bail!("too many shader nodes in instance");
        }
        Ok(())
    }
}
fn pop(stack: &mut Vec<u8>, kind: u8) -> Result<()> {
    if stack.pop() != Some(kind) {
        bail!("unbalanced display group");
    }
    Ok(())
}
fn finite(v: f32) -> Result<()> {
    if !v.is_finite() {
        bail!("non-finite display value");
    }
    Ok(())
}
fn nonnegative(v: f32) -> Result<()> {
    finite(v)?;
    if v < 0.0 {
        bail!("negative display size");
    }
    Ok(())
}
fn unit(v: f32) -> Result<()> {
    finite(v)?;
    if !(0.0..=1.0).contains(&v) {
        bail!("display color/opacity out of range");
    }
    Ok(())
}
fn color(c: Color) -> Result<()> {
    for v in [c.red, c.green, c.blue, c.alpha] {
        unit(v)?;
    }
    Ok(())
}
fn point(p: Point) -> Result<()> {
    finite(p.x)?;
    finite(p.y)
}
fn rectangle(r: Rect) -> Result<()> {
    point(Point::new(r.x, r.y))?;
    nonnegative(r.width)?;
    nonnegative(r.height)?;
    for v in [r.x, r.y, r.width, r.height] {
        if v != v.round() {
            bail!("rectangle must use physical pixels");
        }
    }
    Ok(())
}
pub fn physical_rect(r: Rect) -> Result<Rect> {
    point(Point::new(r.x, r.y))?;
    nonnegative(r.width)?;
    nonnegative(r.height)?;
    Ok(Rect::new(
        r.x.round(),
        r.y.round(),
        if r.width > 0.0 {
            r.width.round().max(1.0)
        } else {
            0.0
        },
        if r.height > 0.0 {
            r.height.round().max(1.0)
        } else {
            0.0
        },
    ))
}
// Keep dimensions compatible with native GL signed sizes and bound allocation.
// 16 MiB per raster matches the SVG CPU resource budget.
pub(crate) fn raster_bytes(width: u32, height: u32) -> Result<usize> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || width > 8192 || height > 8192 || pixels > 4 * 1024 * 1024 {
        bail!("raster dimensions exceed CPU resource limit");
    }
    Ok(usize::try_from(pixels * 4)?)
}
