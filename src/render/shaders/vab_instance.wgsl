#import bevy_sprite::mesh2d_view_bindings::view

struct Draw {
    world_from_local: mat4x4<f32>,
    uv: mat4x4<f32>,
    multiply: vec4<f32>,
    add: vec4<f32>,
    material: vec4<f32>,
};

@group(1) @binding(0) var<storage, read> draws: array<Draw>;
@group(2) @binding(0) var tex: texture_2d<f32>;
@group(2) @binding(1) var samp: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) local_position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) instance_index: u32,
};

fn to_srgb(v: vec3<f32>) -> vec3<f32> {
    return select(12.92 * v, 1.055 * pow(max(v, vec3(0.0)), vec3(1.0 / 2.4)) - 0.055, v > vec3(0.0031308));
}

fn to_linear(v: vec3<f32>) -> vec3<f32> {
    return select(v / 12.92, pow((max(v, vec3(0.0)) + 0.055) / 1.055, vec3(2.4)), v > vec3(0.04045));
}

fn encoded_premultiplied_to_output(encoded: vec4<f32>) -> vec4<f32> {
#ifdef SRGB_COMPOSITING
    // The Camera2d main target is Rgba8Unorm. Bevy converts the completed
    // camera texture to the display surface in its final blit.
    return encoded;
#else
    let straight_rgb = select(
        vec3(0.0),
        encoded.rgb / max(encoded.a, 0.000001),
        encoded.a > 0.0,
    );
    return vec4(to_linear(straight_rgb) * encoded.a, encoded.a);
#endif
}

@vertex
fn vertex(
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>
) -> VertexOutput {
    var out: VertexOutput;
    let draw = draws[instance_index];
    let world = draw.world_from_local * vec4(position, 1.0);
    out.position = view.clip_from_world * world;
    out.local_position = position.xy;
    out.color = color;
    out.instance_index = instance_index;
    return out;
}

@vertex
fn vertex_quad(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32
) -> VertexOutput {
    let positions = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0)
    );
    let local = positions[vertex_index];
    let draw = draws[instance_index];
    var out: VertexOutput;
    out.position = view.clip_from_world * draw.world_from_local * vec4(local, 0.0, 1.0);
    out.local_position = local;
    out.color = vec4(1.0);
    out.instance_index = instance_index;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let draw = draws[in.instance_index];
    // The VATF material transform is a 2D affine mat3 embedded in the first
    // three columns of Mat4. Its translation therefore lives in column 2,
    // matching the original bevy_flash shaders.
    let uv_matrix = mat3x3<f32>(draw.uv[0].xyz, draw.uv[1].xyz, draw.uv[2].xyz);
    let uv = (uv_matrix * vec3(in.local_position, 1.0)).xy;
    if draw.material.w == 3.0 {
        return encoded_premultiplied_to_output(textureSample(tex, samp, uv));
    }
    var color = vec4(to_srgb(in.color.rgb), in.color.a);
    if draw.material.w == 1.0 {
        color = textureSample(tex, samp, uv);
        if color.a > 0.0 {
            color = vec4(color.rgb / color.a, color.a);
        }
    } else if draw.material.w == 2.0 {
        var t = uv.x;
        if draw.material.y == 2.0 {
            t = length(uv * 2.0 - 1.0);
        }
        if draw.material.y == 3.0 {
            let delta = vec2(draw.material.x, 0.0) - (uv * 2.0 - 1.0);
            let len = length(delta);
            let direction = delta / max(len, 0.000001);
            t = len / max(sqrt(max(0.0, 1.0 - draw.material.x * draw.material.x * direction.y * direction.y)) + draw.material.x * direction.x, 0.000001);
        }
        if draw.material.z == 1.0 {
            t = clamp(t, 0.0, 1.0);
        } else if draw.material.z == 2.0 {
            t = 1.0 - abs(fract(t * 0.5) * 2.0 - 1.0);
        } else {
            t = fract(t);
        }
        color = textureSample(tex, samp, vec2((t * 255.0 + 0.5) / 256.0, 0.5));
    }
    color = clamp(color * draw.multiply + draw.add, vec4(0.0), vec4(1.0));
    return encoded_premultiplied_to_output(vec4(color.rgb * color.a, color.a));
}
