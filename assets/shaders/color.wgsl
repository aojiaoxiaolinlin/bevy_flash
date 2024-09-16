#import bevy_sprite::{mesh2d_functions as mesh_functions, mesh2d_vertex_output::VertexOutput}

/// 暂时定为固定值
const view_matrix: mat4x4<f32> = mat4x4<f32>(
    vec4<f32>(1.0, 0.0, 0.0, 0.0),
    vec4<f32>(0.0, -1.0, 0.0, 0.0),
    vec4<f32>(0.0, 0.0, 1.0, 0.0),
    vec4<f32>(-1.0, 1.0, 0.0, 1.0)
);

struct SWFTransform {
    world_matrix: mat4x4<f32>,
    mult_color: vec4<f32>,
    add_color: vec4<f32>,
}

@group(2) @binding(0) var<uniform> swf_transform: SWFTransform;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(4) color: vec4<f32>,
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    let position: vec4<f32> = view_matrix * swf_transform.world_matrix * vec4<f32>(vertex.position, 1.0);
    var out: VertexOutput;
    var world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    out.world_position = mesh_functions::mesh2d_position_local_to_world(
        world_from_local,
        vec4<f32>(position)
    );
    out.position = mesh_functions::mesh2d_position_world_to_clip(out.world_position);
    out.color = vertex.color;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var output_color: vec4<f32> = in.color;
    return output_color;
}