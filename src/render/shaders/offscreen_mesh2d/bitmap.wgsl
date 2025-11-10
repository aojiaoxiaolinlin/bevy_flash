#import bevy_flash::offscreen_common::{TransformUniform}


@group(0) @binding(0) var<uniform> view_matrix: mat4x4<f32>;
@group(1) @binding(0) var<uniform> transform_uniform: TransformUniform;

@group(2) @binding(0) var texture: texture_2d<f32>;
@group(2) @binding(1) var texture_sampler: sampler;
@group(2) @binding(2) var<uniform> texture_transform: mat4x4<f32>;
override late_saturate: bool = false;

struct Vertex {
    @location(0) position: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};


@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    out.uv = (mat3x3<f32>(texture_transform[0].xyz, texture_transform[1].xyz, texture_transform[2].xyz) * vec3<f32>(vertex.position.x, vertex.position.y, 1.0)).xy;
    out.position = view_matrix * transform_uniform.world_matrix * vec4<f32>(vertex.position, 1.0);
    out.position.x = out.position.x - out.position.w;
    out.position.y = out.position.y + out.position.w;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var color: vec4<f32> = textureSample(texture, texture_sampler, in.uv);

    if color.a > 0.0 {
        color = vec4<f32>(color.rgb / color.a, color.a);
        color = color * transform_uniform.mult_color + transform_uniform.add_color;
        if !late_saturate {
            color = saturate(color);
        }
        color = vec4<f32>(color.rgb * color.a, color.a);
        if late_saturate {
            color = saturate(color);
        }
    }
    return color;
}
