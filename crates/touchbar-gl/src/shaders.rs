// GLES programs ported from the original touchbar-ui renderer.
pub(crate) const VERTEX: &str = r#"#version 300 es
precision highp float;
void main() {
    vec2 positions[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    gl_Position = vec4(positions[gl_VertexID], 0.0, 1.0);
}
"#;
pub(crate) const MESH_VERTEX: &str = r#"#version 300 es
precision highp float;
layout(location = 0) in vec2 a_position;
uniform vec2 u_resolution;
void main() {
    vec2 normalized = a_position / u_resolution;
    vec2 clip = normalized * 2.0 - 1.0;
    gl_Position = vec4(clip.x, -clip.y, 0.0, 1.0);
}
"#;
pub(crate) const MESH_FRAGMENT: &str = r#"#version 300 es
precision highp float;
uniform vec4 u_color;
out vec4 output_color;
void main() {
    output_color = vec4(u_color.rgb * u_color.a, u_color.a);
}
"#;
pub(crate) const SHAPE_FRAGMENT: &str = r#"#version 300 es
precision highp float;
uniform vec2 u_resolution;
uniform vec4 u_rect;
uniform float u_radius;
uniform vec4 u_color;
out vec4 output_color;

void main() {
    vec2 point = vec2(gl_FragCoord.x, u_resolution.y - gl_FragCoord.y);
    vec2 center = u_rect.xy + u_rect.zw * 0.5;
    vec2 half_size = u_rect.zw * 0.5;
    float radius = min(u_radius, min(half_size.x, half_size.y));
    vec2 distance_to_edge = abs(point - center) - (half_size - vec2(radius));
    float signed_distance = length(max(distance_to_edge, 0.0))
        + min(max(distance_to_edge.x, distance_to_edge.y), 0.0) - radius;
    float coverage = 1.0 - smoothstep(-0.75, 0.75, signed_distance);
    float alpha = u_color.a * coverage;
    output_color = vec4(u_color.rgb * alpha, alpha);
}
"#;
pub(crate) const LINE_FRAGMENT: &str = r#"#version 300 es
precision highp float;
uniform vec2 u_resolution;
uniform vec2 u_start;
uniform vec2 u_end;
uniform float u_width;
uniform vec4 u_color;
out vec4 output_color;

void main() {
    vec2 point = vec2(gl_FragCoord.x, u_resolution.y - gl_FragCoord.y);
    vec2 segment = u_end - u_start;
    float squared_length = max(dot(segment, segment), 0.000001);
    float amount = clamp(dot(point - u_start, segment) / squared_length, 0.0, 1.0);
    float distance_to_line = length(point - (u_start + segment * amount));
    float coverage = 1.0 - smoothstep(u_width * 0.5 - 0.75, u_width * 0.5 + 0.75, distance_to_line);
    float alpha = u_color.a * coverage;
    output_color = vec4(u_color.rgb * alpha, alpha);
}
"#;
pub(crate) const GRADIENT_FRAGMENT: &str = r#"#version 300 es
precision highp float;
uniform vec2 u_resolution;
uniform vec4 u_rect;
uniform float u_radius;
uniform vec2 u_start;
uniform vec2 u_end;
uniform vec4 u_start_color;
uniform vec4 u_end_color;
out vec4 output_color;

void main() {
    vec2 point = vec2(gl_FragCoord.x, u_resolution.y - gl_FragCoord.y);
    vec2 center = u_rect.xy + u_rect.zw * 0.5;
    vec2 half_size = u_rect.zw * 0.5;
    float radius = min(u_radius, min(half_size.x, half_size.y));
    vec2 distance_to_edge = abs(point - center) - (half_size - vec2(radius));
    float signed_distance = length(max(distance_to_edge, 0.0))
        + min(max(distance_to_edge.x, distance_to_edge.y), 0.0) - radius;
    float coverage = 1.0 - smoothstep(-0.75, 0.75, signed_distance);
    vec2 axis = u_end - u_start;
    float amount = clamp(dot(point - u_start, axis) / max(dot(axis, axis), 0.000001), 0.0, 1.0);
    vec4 color = mix(u_start_color, u_end_color, amount);
    float alpha = color.a * coverage;
    output_color = vec4(color.rgb * alpha, alpha);
}
"#;
pub(crate) const IMAGE_FRAGMENT: &str = r#"#version 300 es
precision highp float;
uniform vec2 u_resolution;
uniform vec4 u_rect;
uniform sampler2D u_image;
uniform float u_opacity;
uniform vec4 u_tint;
uniform int u_tint_mode;
out vec4 output_color;

void main() {
    vec2 point = vec2(gl_FragCoord.x, u_resolution.y - gl_FragCoord.y);
    vec2 uv = (point - u_rect.xy) / u_rect.zw;
    if (uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0) {
        discard;
    }
    vec4 sampled = texture(u_image, uv);
    vec3 rgb = sampled.rgb;
    float tint_alpha = 1.0;
    if (u_tint_mode == 1) {
        rgb *= u_tint.rgb;
        tint_alpha = u_tint.a;
    } else if (u_tint_mode == 2) {
        rgb = u_tint.rgb;
        tint_alpha = u_tint.a;
    }
    float alpha = sampled.a * tint_alpha * u_opacity;
    output_color = vec4(rgb * alpha, alpha);
}
"#;
pub(crate) const COMPARE: &str = r#"#version 300 es
precision highp float;
uniform sampler2D u_old;
uniform sampler2D u_new;
uniform int u_height;
out vec4 color;
void main() {
    int x = int(gl_FragCoord.x);
    float differs = 0.0;
    for (int y = 0; y < u_height; y++) {
        if (any(notEqual(texelFetch(u_old, ivec2(x, y), 0), texelFetch(u_new, ivec2(x, y), 0)))) { differs=1.0; break; }
    }
    color=vec4(differs);
}
"#;
pub(crate) const COPY: &str = r#"#version 300 es
precision highp float;
uniform sampler2D u_image;
uniform ivec2 u_size;
uniform int u_orientation;
out vec4 color;
void main() {
    ivec2 p=ivec2(gl_FragCoord.xy);
    if (u_orientation==1) p=ivec2(u_size.x-1-p.y,p.x);
    else if (u_orientation==2) p=ivec2(p.y,u_size.y-1-p.x);
    else if (u_orientation==3) p=ivec2(u_size.x-1-p.y,u_size.y-1-p.x);
    color=texelFetch(u_image,p,0);
}
"#;
