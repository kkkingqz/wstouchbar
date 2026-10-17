//! All GL handles stay behind the renderer's current, thread-owned EGL context.
#![allow(unsafe_code)]
use crate::{Backend, DamageRect, Orientation, RenderedFrame, egl::DeviceContext, shaders};
use anyhow::{Context, Result, anyhow, bail, ensure};
use glow::HasContext;
use std::collections::BTreeMap;
use touchbar_model::SurfaceId;
use touchbar_ui::{self as u, Color, DisplayList, DrawCommand, Point, Rect, Resource};

struct Program {
    handle: glow::Program,
    uniforms: BTreeMap<&'static str, Option<glow::UniformLocation>>,
}
impl Program {
    fn uniform(&self, name: &str) -> Option<&glow::UniformLocation> {
        self.uniforms.get(name).and_then(Option::as_ref)
    }
}
struct Image {
    texture: glow::Texture,
    bytes: usize,
    used: u64,
}
struct Effect {
    program: glow::Program,
    used: u64,
}
#[derive(Clone, Copy)]
struct Target {
    texture: glow::Texture,
    fbo: glow::Framebuffer,
}
struct Surface {
    size: (u32, u32),
    orientation: Orientation,
    logical: [Target; 2],
    physical: Option<Target>,
    compare: Target,
    previous: Option<usize>,
}
#[derive(Clone, Copy)]
struct Affine {
    scale: f32,
    x: f32,
    y: f32,
}
impl Affine {
    const IDENTITY: Self = Self {
        scale: 1.0,
        x: 0.0,
        y: 0.0,
    };
    fn rect(self, r: Rect) -> Rect {
        Rect::new(
            r.x * self.scale + self.x,
            r.y * self.scale + self.y,
            r.width * self.scale,
            r.height * self.scale,
        )
    }
    fn point(self, p: Point) -> Point {
        Point::new(p.x * self.scale + self.x, p.y * self.scale + self.y)
    }
    fn then(self, origin: Point, t: u::VisualTransform) -> Self {
        let scale = t.scale.max(0.001);
        Self {
            scale: self.scale * scale,
            x: self.x + self.scale * (origin.x * (1.0 - scale) + t.translation.x),
            y: self.y + self.scale * (origin.y * (1.0 - scale) + t.translation.y),
        }
    }
}
pub(crate) struct Native {
    device: DeviceContext,
    programs: BTreeMap<&'static str, Program>,
    mesh_buffer: glow::Buffer,
    mesh_vao: glow::VertexArray,
    uniforms: glow::Buffer,
    images: BTreeMap<String, Image>,
    effects: BTreeMap<String, Effect>,
    surfaces: BTreeMap<SurfaceId, Surface>,
    clock: u64,
}
impl Native {
    pub fn new(backend: &Backend) -> Result<Self> {
        let device = DeviceContext::new(backend)?;
        let gl = &device.gl;
        let mut programs = BTreeMap::new();
        // SAFETY: the device constructor made its context current; handles never escape.
        unsafe {
            for (name, vertex, fragment) in [
                ("shape", shaders::VERTEX, shaders::SHAPE_FRAGMENT),
                ("gradient", shaders::VERTEX, shaders::GRADIENT_FRAGMENT),
                ("line", shaders::VERTEX, shaders::LINE_FRAGMENT),
                ("mesh", shaders::MESH_VERTEX, shaders::MESH_FRAGMENT),
                ("image", shaders::VERTEX, shaders::IMAGE_FRAGMENT),
                ("copy", shaders::VERTEX, shaders::COPY),
                ("compare", shaders::VERTEX, shaders::COMPARE),
            ] {
                let handle = compile_program(gl, vertex, fragment)?;
                let uniforms = [
                    "u_resolution",
                    "u_rect",
                    "u_radius",
                    "u_color",
                    "u_start",
                    "u_end",
                    "u_start_color",
                    "u_end_color",
                    "u_width",
                    "u_opacity",
                    "u_tint",
                    "u_tint_mode",
                    "u_image",
                    "u_old",
                    "u_new",
                    "u_height",
                    "u_size",
                    "u_orientation",
                ]
                .into_iter()
                .map(|name| (name, gl.get_uniform_location(handle, name)))
                .collect();
                programs.insert(name, Program { handle, uniforms });
            }
            let mesh_buffer = gl.create_buffer().map_err(|e| anyhow!(e))?;
            let mesh_vao = gl.create_vertex_array().map_err(|e| anyhow!(e))?;
            let uniforms = gl.create_buffer().map_err(|e| anyhow!(e))?;
            Ok(Self {
                device,
                programs,
                mesh_buffer,
                mesh_vao,
                uniforms,
                images: BTreeMap::new(),
                effects: BTreeMap::new(),
                surfaces: BTreeMap::new(),
                clock: 0,
            })
        }
    }
    pub fn description(&self) -> &str {
        &self.device.description
    }
    pub fn render(
        &mut self,
        id: SurfaceId,
        size: (u32, u32),
        orientation: Orientation,
        list: &DisplayList,
        full: bool,
    ) -> Result<Option<RenderedFrame>> {
        list.validate()?;
        validate_size(size)?;
        self.device.owner.current()?;
        // SAFETY: this device's context is current before capability queries.
        let limit = unsafe { self.device.gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) };
        ensure!(
            i64::from(size.0) <= i64::from(limit) && i64::from(size.1) <= i64::from(limit),
            "target exceeds GPU texture size"
        );
        let mut surface = if let Some(old) = self.surfaces.remove(&id) {
            if old.size == size && old.orientation == orientation {
                old
            } else {
                self.destroy_surface(old);
                self.surface(size, orientation)?
            }
        } else {
            self.surface(size, orientation)?
        };
        let result = self.render_surface(&mut surface, list, full);
        self.surfaces.insert(id, surface);
        result
    }
    fn surface(&self, size: (u32, u32), orientation: Orientation) -> Result<Surface> {
        let mut allocated = Vec::new();
        // SAFETY: validated dimensions, current context, and all partial allocations cleaned on error.
        unsafe {
            let mut make = |size| -> Result<Target> {
                let target = target(&self.device.gl, size)?;
                allocated.push(target);
                Ok(target)
            };
            let result = (|| {
                let logical = [make(size)?, make(size)?];
                let physical = if orientation == Orientation::Landscape {
                    None
                } else {
                    Some(make((size.1, size.0))?)
                };
                let compare = make((size.0, 1))?;
                Ok(Surface {
                    size,
                    orientation,
                    logical,
                    physical,
                    compare,
                    previous: None,
                })
            })();
            if result.is_err() {
                for target in allocated {
                    self.device.gl.delete_framebuffer(target.fbo);
                    self.device.gl.delete_texture(target.texture);
                }
            }
            result
        }
    }
    fn render_surface(
        &mut self,
        surface: &mut Surface,
        list: &DisplayList,
        full: bool,
    ) -> Result<Option<RenderedFrame>> {
        let next = surface.previous.map_or(0, |old| 1 - old);
        let size = surface.size;
        self.draw(&surface.logical[next], size, list)?;
        let spans = match surface.previous {
            Some(previous) if !full => self.compare(surface, previous, next)?,
            _ => vec![(0, size.0)],
        };
        if spans.is_empty() {
            return Ok(None);
        }
        let gl = &self.device.gl;
        // SAFETY: each framebuffer/texture belongs to this current context.
        let (fbo, output_size) = unsafe {
            if let Some(physical) = &surface.physical {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(physical.fbo));
                gl.viewport(0, 0, size.1 as i32, size.0 as i32);
                gl.disable(glow::BLEND);
                gl.disable(glow::SCISSOR_TEST);
                let program = &self.programs["copy"];
                gl.use_program(Some(program.handle));
                gl.bind_vertex_array(None);
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(surface.logical[next].texture));
                gl.uniform_1_i32(program.uniform("u_image"), 0);
                gl.uniform_2_i32(program.uniform("u_size"), size.0 as i32, size.1 as i32);
                gl.uniform_1_i32(
                    program.uniform("u_orientation"),
                    if surface.orientation == Orientation::Clockwise {
                        1
                    } else {
                        2
                    },
                );
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
                (physical.fbo, (size.1, size.0))
            } else {
                (surface.logical[next].fbo, size)
            }
        };
        let mut pixels = vec![0; output_size.0 as usize * output_size.1 as usize * 4];
        // SAFETY: the output slice has exactly width*height*4 bytes; readback waits for GPU completion.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
            gl.read_pixels(
                0,
                0,
                output_size.0 as i32,
                output_size.1 as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
            ensure!(
                gl.get_error() == glow::NO_ERROR,
                "GLES render/readback error"
            );
        }
        flip_rows(&mut pixels, output_size);
        let damage = damage(&spans, size, surface.orientation);
        surface.previous = Some(next);
        Ok(Some(RenderedFrame {
            width: output_size.0,
            height: output_size.1,
            pixels,
            damage,
        }))
    }
    fn draw(&mut self, target: &Target, size: (u32, u32), list: &DisplayList) -> Result<()> {
        self.clock = self.clock.wrapping_add(1);
        let gl = &self.device.gl;
        // SAFETY: validated target and owned objects, current context for this complete pass.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.fbo));
            gl.viewport(0, 0, size.0 as i32, size.1 as i32);
            gl.disable(glow::SCISSOR_TEST);
            let color = list.clear.premultiplied();
            gl.clear_color(color.red, color.green, color.blue, color.alpha);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.bind_vertex_array(None);
        }
        let viewport = Rect::new(0.0, 0.0, size.0 as f32, size.1 as f32);
        let mut clips = vec![viewport];
        let mut opacity = vec![1.0];
        let mut transforms = vec![Affine::IDENTITY];
        for command in &list.commands {
            let alpha = *opacity.last().unwrap();
            let transform = *transforms.last().unwrap();
            let clip = *clips.last().unwrap();
            // SAFETY: DisplayList validates stack balance/types/numbers; all draws use owned objects.
            unsafe {
                match command {
                    DrawCommand::PushOpacity { opacity: o } => opacity.push(alpha * o),
                    DrawCommand::PopOpacity => {
                        opacity.pop();
                    }
                    DrawCommand::PushTransform {
                        origin,
                        transform: t,
                    } => {
                        transforms.push(transform.then(*origin, *t));
                        opacity.push(alpha * t.opacity);
                    }
                    DrawCommand::PopTransform => {
                        transforms.pop();
                        opacity.pop();
                    }
                    DrawCommand::PushClip { rect } => {
                        clips.push(intersect(clip, transform.rect(*rect)))
                    }
                    DrawCommand::PopClip => {
                        clips.pop();
                    }
                    DrawCommand::RoundedRect {
                        rect,
                        radius,
                        color,
                    } => {
                        self.common("shape", size, clip);
                        let gl = &self.device.gl;
                        let p = &self.programs["shape"];
                        uniform_rect(gl, p, transform.rect(*rect));
                        gl.uniform_1_f32(p.uniform("u_radius"), radius * transform.scale);
                        uniform_color(gl, p, "u_color", *color, alpha);
                        gl.draw_arrays(glow::TRIANGLES, 0, 3);
                    }
                    DrawCommand::LinearGradientRect {
                        rect,
                        radius,
                        start,
                        end,
                        start_color,
                        end_color,
                    } => {
                        self.common("gradient", size, clip);
                        let gl = &self.device.gl;
                        let p = &self.programs["gradient"];
                        uniform_rect(gl, p, transform.rect(*rect));
                        gl.uniform_1_f32(p.uniform("u_radius"), radius * transform.scale);
                        uniform_point(gl, p, "u_start", transform.point(*start));
                        uniform_point(gl, p, "u_end", transform.point(*end));
                        uniform_color(gl, p, "u_start_color", *start_color, alpha);
                        uniform_color(gl, p, "u_end_color", *end_color, alpha);
                        gl.draw_arrays(glow::TRIANGLES, 0, 3);
                    }
                    DrawCommand::Line {
                        start,
                        end,
                        width,
                        color,
                    } => {
                        self.common("line", size, clip);
                        let gl = &self.device.gl;
                        let p = &self.programs["line"];
                        uniform_point(gl, p, "u_start", transform.point(*start));
                        uniform_point(gl, p, "u_end", transform.point(*end));
                        gl.uniform_1_f32(p.uniform("u_width"), width * transform.scale);
                        uniform_color(gl, p, "u_color", *color, alpha);
                        gl.draw_arrays(glow::TRIANGLES, 0, 3);
                    }
                    DrawCommand::TriangleMesh { triangles, color } => {
                        self.common("mesh", size, clip);
                        let gl = &self.device.gl;
                        let p = &self.programs["mesh"];
                        let mut bytes = Vec::with_capacity(triangles.len() * 8);
                        for point in triangles.iter() {
                            let point = transform.point(*point);
                            bytes.extend_from_slice(&point.x.to_ne_bytes());
                            bytes.extend_from_slice(&point.y.to_ne_bytes());
                        }
                        gl.bind_vertex_array(Some(self.mesh_vao));
                        gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.mesh_buffer));
                        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STREAM_DRAW);
                        gl.enable_vertex_attrib_array(0);
                        gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
                        uniform_color(gl, p, "u_color", *color, alpha);
                        gl.draw_arrays(glow::TRIANGLES, 0, i32::try_from(triangles.len())?);
                        gl.bind_vertex_array(None);
                    }
                    DrawCommand::Image {
                        rect,
                        resource,
                        opacity: o,
                        coloring,
                    } => {
                        let texture = self.image(resource, &list.resources[resource])?;
                        self.common("image", size, clip);
                        let gl = &self.device.gl;
                        let p = &self.programs["image"];
                        uniform_rect(gl, p, transform.rect(*rect));
                        gl.uniform_1_f32(p.uniform("u_opacity"), alpha * o);
                        let (mode, tint) = match coloring {
                            u::ImageColoring::Original => (0, Color::WHITE),
                            u::ImageColoring::Multiply(color) => (1, *color),
                            u::ImageColoring::Mask(color) => (2, *color),
                        };
                        uniform_color(gl, p, "u_tint", tint, 1.0);
                        gl.uniform_1_i32(p.uniform("u_tint_mode"), mode);
                        gl.active_texture(glow::TEXTURE0);
                        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                        gl.uniform_1_i32(p.uniform("u_image"), 0);
                        gl.draw_arrays(glow::TRIANGLES, 0, 3);
                    }
                    DrawCommand::ShaderEffect {
                        rect,
                        resource,
                        uniforms,
                    } => {
                        let program = self.effect(resource, &list.resources[resource])?;
                        let gl = &self.device.gl;
                        let rect = transform.rect(*rect);
                        let clip = intersect(clip, rect);
                        scissor(gl, clip, size);
                        gl.use_program(Some(program));
                        gl.bind_vertex_array(None);
                        let theme = uniforms.theme;
                        let mut values = vec![
                            rect.x,
                            rect.y,
                            rect.width,
                            rect.height,
                            size.0 as f32,
                            size.1 as f32,
                            uniforms.time,
                            uniforms.opacity * alpha,
                        ];
                        for color in [
                            theme.background,
                            theme.control,
                            theme.control_pressed,
                            theme.track,
                            theme.foreground,
                            theme.muted,
                            theme.accent,
                            theme.destructive,
                            theme.accent.contrasting_foreground(),
                            theme.destructive.contrasting_foreground(),
                        ] {
                            values.extend_from_slice(&[
                                color.red,
                                color.green,
                                color.blue,
                                color.alpha,
                            ]);
                        }
                        values.extend_from_slice(&uniforms.parameters);
                        let bytes = values
                            .into_iter()
                            .flat_map(f32::to_ne_bytes)
                            .collect::<Vec<_>>();
                        gl.bind_buffer(glow::UNIFORM_BUFFER, Some(self.uniforms));
                        gl.buffer_data_u8_slice(glow::UNIFORM_BUFFER, &bytes, glow::STREAM_DRAW);
                        gl.bind_buffer_base(glow::UNIFORM_BUFFER, 0, Some(self.uniforms));
                        gl.draw_arrays(glow::TRIANGLES, 0, 3);
                    }
                }
            }
        }
        Ok(())
    }
    unsafe fn common(&self, name: &str, size: (u32, u32), clip: Rect) {
        // SAFETY: the caller owns the complete current-context draw pass.
        unsafe {
            let gl = &self.device.gl;
            let p = &self.programs[name];
            gl.use_program(Some(p.handle));
            gl.uniform_2_f32(p.uniform("u_resolution"), size.0 as f32, size.1 as f32);
            gl.bind_vertex_array(None);
            scissor(gl, clip, size);
        }
    }
    fn image(&mut self, key: &str, resource: &Resource) -> Result<glow::Texture> {
        if let Some(image) = self.images.get_mut(key) {
            image.used = self.clock;
            return Ok(image.texture);
        }
        let Resource::Raster {
            width,
            height,
            pixels,
        } = resource
        else {
            bail!("image command requires raster")
        };
        while self.images.values().map(|i| i.bytes).sum::<usize>() + pixels.len() > 64 * 1024 * 1024
        {
            let key = self
                .images
                .iter()
                .min_by_key(|(_, image)| image.used)
                .map(|(key, _)| key.clone())
                .context("raster exceeds texture cache budget")?;
            let image = self.images.remove(&key).unwrap();
            // SAFETY: cache entry belongs to this current context; GL retains pending command references.
            unsafe {
                self.device.gl.delete_texture(image.texture);
            }
        }
        // SAFETY: resource dimensions/payload were validated before rendering; current context owns texture.
        let texture = unsafe {
            let gl = &self.device.gl;
            let texture = gl.create_texture().map_err(|e| anyhow!(e))?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            texture_params(gl);
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                *width as i32,
                *height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(pixels)),
            );
            texture
        };
        self.images.insert(
            key.into(),
            Image {
                texture,
                bytes: pixels.len(),
                used: self.clock,
            },
        );
        Ok(texture)
    }
    fn effect(&mut self, key: &str, resource: &Resource) -> Result<glow::Program> {
        if let Some(effect) = self.effects.get_mut(key) {
            effect.used = self.clock;
            return Ok(effect.program);
        }
        let Resource::Shader { body } = resource else {
            bail!("effect command requires shader")
        };
        let (source, block) = crate::effect_glsl::translate(body)?;
        // SAFETY: current context owns compiled program and its uniform block.
        let program = unsafe {
            let gl = &self.device.gl;
            let program = compile_program(gl, shaders::VERTEX, &source)?;
            let Some(index) = gl.get_uniform_block_index(program, &block) else {
                gl.delete_program(program);
                bail!("effect uniform block missing")
            };
            gl.uniform_block_binding(program, index, 0);
            program
        };
        if self.effects.len() >= 64 {
            let old = self
                .effects
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(key, _)| key.clone())
                .unwrap();
            let old = self.effects.remove(&old).unwrap();
            // SAFETY: evicted program belongs to this current context.
            unsafe {
                self.device.gl.delete_program(old.program);
            }
        }
        self.effects.insert(
            key.into(),
            Effect {
                program,
                used: self.clock,
            },
        );
        Ok(program)
    }
    fn compare(&self, surface: &Surface, previous: usize, next: usize) -> Result<Vec<(u32, u32)>> {
        let gl = &self.device.gl;
        let size = surface.size;
        let mut flags = vec![0; size.0 as usize * 4];
        // SAFETY: one RGBA texel per column, with matching owned framebuffer/texture dimensions.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(surface.compare.fbo));
            gl.viewport(0, 0, size.0 as i32, 1);
            gl.disable(glow::BLEND);
            gl.disable(glow::SCISSOR_TEST);
            gl.bind_vertex_array(None);
            let program = &self.programs["compare"];
            gl.use_program(Some(program.handle));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(surface.logical[previous].texture));
            gl.uniform_1_i32(program.uniform("u_old"), 0);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(surface.logical[next].texture));
            gl.uniform_1_i32(program.uniform("u_new"), 1);
            gl.uniform_1_i32(program.uniform("u_height"), size.1 as i32);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.active_texture(glow::TEXTURE0);
            gl.read_pixels(
                0,
                0,
                size.0 as i32,
                1,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut flags)),
            );
        }
        let mut spans = vec![];
        for (column, pixel) in flags.as_chunks::<4>().0.iter().enumerate() {
            if pixel[0] == 0 {
                continue;
            }
            let x = u32::try_from(column)?;
            if let Some((_, end)) = spans.last_mut().filter(|(_, end)| *end == x) {
                *end = x + 1;
            } else {
                spans.push((x, x + 1));
            }
        }
        Ok(spans)
    }
    pub fn remove_surface(&mut self, id: SurfaceId) {
        if self.device.owner.current().is_ok()
            && let Some(surface) = self.surfaces.remove(&id)
        {
            self.destroy_surface(surface);
        }
    }
    fn destroy_surface(&self, surface: Surface) {
        // SAFETY: caller made this owner's context current, targets are no longer referenced by cache.
        unsafe {
            for t in surface
                .logical
                .into_iter()
                .chain(surface.physical)
                .chain([surface.compare])
            {
                self.device.gl.delete_framebuffer(t.fbo);
                self.device.gl.delete_texture(t.texture);
            }
        }
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        if self.device.owner.current().is_err() {
            return;
        }
        for (_, surface) in std::mem::take(&mut self.surfaces) {
            self.destroy_surface(surface);
        }
        // SAFETY: all objects are owned by this current context; EGL context is destroyed afterwards.
        unsafe {
            let gl = &self.device.gl;
            for program in self.programs.values() {
                gl.delete_program(program.handle);
            }
            for image in self.images.values() {
                gl.delete_texture(image.texture);
            }
            for effect in self.effects.values() {
                gl.delete_program(effect.program);
            }
            gl.delete_buffer(self.mesh_buffer);
            gl.delete_buffer(self.uniforms);
            gl.delete_vertex_array(self.mesh_vao);
        }
    }
}
fn validate_size((w, h): (u32, u32)) -> Result<()> {
    ensure!(
        w > 0 && h > 0 && w <= 8192 && h <= 8192 && u64::from(w) * u64::from(h) <= 4_194_304,
        "offscreen target exceeds dimension/pixel budget"
    );
    Ok(())
}
unsafe fn target(gl: &glow::Context, (w, h): (u32, u32)) -> Result<Target> {
    // SAFETY: dimensions validated; caller's context current.
    unsafe {
        let texture = gl.create_texture().map_err(|e| anyhow!(e))?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        texture_params(gl);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            w as i32,
            h as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        let fbo = match gl.create_framebuffer() {
            Ok(fbo) => fbo,
            Err(e) => {
                gl.delete_texture(texture);
                bail!(e)
            }
        };
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(texture),
            0,
        );
        if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
            gl.delete_framebuffer(fbo);
            gl.delete_texture(texture);
            bail!("incomplete RGBA framebuffer");
        }
        Ok(Target { texture, fbo })
    }
}
unsafe fn texture_params(gl: &glow::Context) {
    // SAFETY: current context and texture bound by caller.
    unsafe {
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_S,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_T,
            glow::CLAMP_TO_EDGE as i32,
        );
    }
}
unsafe fn compile_program(
    gl: &glow::Context,
    vertex: &str,
    fragment: &str,
) -> Result<glow::Program> {
    // SAFETY: only this current-context helper creates/deletes these temporary handles.
    unsafe {
        let vert = compile_shader(gl, glow::VERTEX_SHADER, vertex)?;
        let frag = match compile_shader(gl, glow::FRAGMENT_SHADER, fragment) {
            Ok(shader) => shader,
            Err(e) => {
                gl.delete_shader(vert);
                return Err(e);
            }
        };
        let program = match gl.create_program() {
            Ok(program) => program,
            Err(e) => {
                gl.delete_shader(vert);
                gl.delete_shader(frag);
                bail!(e)
            }
        };
        gl.attach_shader(program, vert);
        gl.attach_shader(program, frag);
        gl.link_program(program);
        gl.detach_shader(program, vert);
        gl.detach_shader(program, frag);
        gl.delete_shader(vert);
        gl.delete_shader(frag);
        if !gl.get_program_link_status(program) {
            let log = gl.get_program_info_log(program);
            gl.delete_program(program);
            bail!("GLES link failed: {log}");
        }
        Ok(program)
    }
}
unsafe fn compile_shader(gl: &glow::Context, kind: u32, source: &str) -> Result<glow::Shader> {
    // SAFETY: caller owns a current context; on failure its temporary shader is deleted.
    unsafe {
        let shader = gl.create_shader(kind).map_err(|e| anyhow!(e))?;
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
        if !gl.get_shader_compile_status(shader) {
            let log = gl.get_shader_info_log(shader);
            gl.delete_shader(shader);
            bail!("GLES shader compile failed: {log}");
        }
        Ok(shader)
    }
}
unsafe fn uniform_rect(gl: &glow::Context, p: &Program, r: Rect) {
    unsafe {
        gl.uniform_4_f32(p.uniform("u_rect"), r.x, r.y, r.width, r.height);
    }
}
unsafe fn uniform_point(gl: &glow::Context, p: &Program, name: &str, point: Point) {
    unsafe {
        gl.uniform_2_f32(p.uniform(name), point.x, point.y);
    }
}
unsafe fn uniform_color(gl: &glow::Context, p: &Program, name: &str, color: Color, opacity: f32) {
    unsafe {
        gl.uniform_4_f32(
            p.uniform(name),
            color.red,
            color.green,
            color.blue,
            color.alpha * opacity,
        );
    }
}
fn intersect(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    Rect::new(
        x,
        y,
        (a.x + a.width).min(b.x + b.width).max(x) - x,
        (a.y + a.height).min(b.y + b.height).max(y) - y,
    )
}
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
unsafe fn scissor(gl: &glow::Context, rect: Rect, size: (u32, u32)) {
    // SAFETY: clip is intersected with validated viewport before integer conversion.
    unsafe {
        let x = rect.x.floor().clamp(0.0, size.0 as f32) as i32;
        let y = rect.y.floor().clamp(0.0, size.1 as f32) as i32;
        let right = (rect.x + rect.width).ceil().clamp(x as f32, size.0 as f32) as i32;
        let bottom = (rect.y + rect.height).ceil().clamp(y as f32, size.1 as f32) as i32;
        gl.enable(glow::SCISSOR_TEST);
        gl.scissor(x, size.1 as i32 - bottom, right - x, bottom - y);
    }
}
fn flip_rows(pixels: &mut [u8], size: (u32, u32)) {
    let stride = size.0 as usize * 4;
    for y in 0..size.1 as usize / 2 {
        let (first, last) = pixels.split_at_mut((size.1 as usize - 1 - y) * stride);
        first[y * stride..(y + 1) * stride].swap_with_slice(&mut last[..stride]);
    }
}
fn damage(spans: &[(u32, u32)], size: (u32, u32), orientation: Orientation) -> Vec<DamageRect> {
    let mut spans = spans.to_vec();
    while spans.len() > 8 {
        let i = (1..spans.len())
            .min_by_key(|i| spans[*i].0 - spans[*i - 1].1)
            .unwrap();
        let (_, end) = spans.remove(i);
        spans[i - 1].1 = end;
    }
    spans
        .into_iter()
        .map(|(start, end)| match orientation {
            Orientation::Landscape => DamageRect {
                x: start,
                y: 0,
                width: end - start,
                height: size.1,
            },
            Orientation::Clockwise => DamageRect {
                x: 0,
                y: start,
                width: size.1,
                height: end - start,
            },
            Orientation::CounterClockwise => DamageRect {
                x: 0,
                y: size.0 - end,
                width: size.1,
                height: end - start,
            },
        })
        .collect()
}
