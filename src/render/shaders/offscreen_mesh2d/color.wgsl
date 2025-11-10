#import bevy_flash::offscreen_common::TransformUniform

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(2) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> view_matrix: mat4x4<f32>;
@group(1) @binding(0) var<uniform> transform_uniform: TransformUniform;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    out.position = view_matrix * transform_uniform.world_matrix * vec4<f32>(vertex.position, 1.0);
    out.position.x = out.position.x - out.position.w;
    out.position.y = out.position.y + out.position.w;
    let color = saturate(vertex.color * transform_uniform.mult_color + transform_uniform.add_color);
    out.color = vec4<f32>(color.rgb * color.a, color.a);
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
