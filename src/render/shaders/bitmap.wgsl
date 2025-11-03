#import bevy_flash::common::{
    get_world_from_local,
    mesh2d_position_local_to_world,
    mesh2d_position_world_to_clip,
    part_mesh2d_color_transform,
}


@group(2) @binding(0) var texture: texture_2d<f32>;
@group(2) @binding(1) var texture_sampler: sampler;
@group(2) @binding(2) var<uniform> texture_transform: mat4x4<f32>;
override late_saturate: bool = false;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
};

struct VertexOutput {
    // The clip-space position of the vertex.
    @builtin(position) position: vec4<f32>,
    // The color of the vertex.
    @location(0) uv: vec2<f32>,
    @location(1) instance_index: u32,
};


@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    out.uv = (mat3x3<f32>(texture_transform[0].xyz, texture_transform[1].xyz, texture_transform[2].xyz) * vec3<f32>(vertex.position.x, vertex.position.y, 1.0)).xy;
    var world_from_local = get_world_from_local(vertex.instance_index);
    let world_position = mesh2d_position_local_to_world(
        world_from_local,
        vec4<f32>(vertex.position, 1.0)
    );
    out.position = mesh2d_position_world_to_clip(world_position);
    out.position.x = out.position.x - out.position.w;
    out.position.y = out.position.y + out.position.w;
    out.instance_index = vertex.instance_index;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var color: vec4<f32> = textureSample(texture, texture_sampler, in.uv);

    if color.a > 0.0 {
        color = vec4<f32>(color.rgb / color.a, color.a);
        color = part_mesh2d_color_transform(in.instance_index, color);
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
