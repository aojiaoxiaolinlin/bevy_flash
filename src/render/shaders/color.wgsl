#import bevy_sprite::{mesh2d_functions as mesh_functions, mesh2d_vertex_output::VertexOutput}
#import bevy_flash::common::{view_matrix, MaterialTransform}

@group(2) @binding(0) var<uniform> material_transform: MaterialTransform;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(4) color: vec4<f32>,
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    let position: vec4<f32> = view_matrix * material_transform.world_matrix * vec4<f32>(vertex.position, 1.0);
    var out: VertexOutput;
    var world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    out.world_position = mesh_functions::mesh2d_position_local_to_world(
        world_from_local,
        position
    );
    out.position = mesh_functions::mesh2d_position_world_to_clip(out.world_position);
    out.position.x = out.position.x - out.position.w;
    out.position.y = out.position.y + out.position.w;
    let color = saturate(vertex.color * material_transform.mult_color + material_transform.add_color);
    out.color = vec4<f32>(color.rgb * color.a, color.a);
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}