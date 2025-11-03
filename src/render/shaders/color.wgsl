#import bevy_sprite::{mesh2d_vertex_output::VertexOutput}
#import bevy_flash::common::{
    view_matrix,
    get_world_from_local,
    mesh2d_position_local_to_world,
    mesh2d_position_world_to_clip,
    part_mesh2d_color_transform,
}

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(4) color: vec4<f32>,
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    var world_from_local = get_world_from_local(vertex.instance_index);
    out.world_position = mesh2d_position_local_to_world(
        world_from_local,
        vec4<f32>(vertex.position, 1.0)
    );
    out.position = mesh2d_position_world_to_clip(out.world_position);
    out.position.x = out.position.x - out.position.w;
    out.position.y = out.position.y + out.position.w;
    let color = saturate(part_mesh2d_color_transform(vertex.instance_index, vertex.color));
    out.color = vec4<f32>(color.rgb * color.a, color.a);
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}