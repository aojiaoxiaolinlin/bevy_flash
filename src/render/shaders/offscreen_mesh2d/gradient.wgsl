#import bevy_flash::offscreen_common::{TransformUniform};
#import bevy_flash::common::{linear_to_srgb};

struct Gradient {
    focal_point: f32,
    interpolation: i32,
    shape: i32,
    repeat: i32,
};
struct Vertex {
    @location(0) position: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

struct TextureTransforms {
    texture_matrix: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> view_matrix: mat4x4<f32>;
@group(1) @binding(0) var<uniform> transform_uniform: TransformUniform;

@group(2) @binding(0) var texture: texture_2d<f32>;
@group(2) @binding(1) var texture_sampler: sampler;
@group(2) @binding(2) var<uniform> gradient: Gradient;
@group(2) @binding(3) var<uniform> texture_transforms: TextureTransforms;





@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let matrix_ = texture_transforms.texture_matrix;
    out.uv = (mat3x3<f32>(matrix_[0].xyz, matrix_[1].xyz, matrix_[2].xyz) * vec3<f32>(vertex.position.xy, 1.0)).xy;
    out.position = view_matrix * transform_uniform.world_matrix * vec4<f32>(vertex.position, 1.0);
    out.position.x = out.position.x - out.position.w;
    out.position.y = out.position.y + out.position.w;
    return out;
}

fn find_t(uv: vec2<f32>) -> f32 {
    if gradient.shape == 1 {
        // linear
        return uv.x;
    } if gradient.shape == 2 {
        // radial
        return length(uv * 2.0 - 1.0);
    } else {
        // focal
        let uv = uv * 2.0 - 1.0;
        var d: vec2<f32> = vec2<f32>(gradient.focal_point, 0.0) - uv;
        let l = length(d);
        d = d / l;
        return l / (sqrt(1.0 - gradient.focal_point * gradient.focal_point * d.y * d.y) + gradient.focal_point * d.x);
    }
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {

    // Calculate normalized `t` position in gradient, [0.0, 1.0] being the bounds of the ratios.
    var t: f32 = find_t(in.uv);

    if gradient.repeat == 1 {
        // Pad
        t = saturate(t);
    } else if gradient.repeat == 2 {
        // Reflect
        if t < 0.0 {
            t = -t;
        }
        if (i32(t) & 1) == 0 {
            t = fract(t);
        } else {
            t = 1.0 - fract(t);
        }
    } else if gradient.repeat == 3 {
        // Repeat
        t = fract(t);
    }

    var color = textureSample(texture, texture_sampler, vec2<f32>(t, 0.0));
    if gradient.interpolation != 0 {
        color = linear_to_srgb(color);
    }
    let out = saturate(color * transform_uniform.mult_color + transform_uniform.add_color);
    let alpha = saturate(out.a);
    return vec4<f32>(out.rgb * alpha, alpha);
}
