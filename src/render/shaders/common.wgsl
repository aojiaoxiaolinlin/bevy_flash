#define_import_path bevy_flash::common

#import bevy_sprite::{
    mesh2d_view_bindings::view
}
#import bevy_render::maths::{affine3_to_square}


struct PartMesh2d {
    // Affine 4x3 matrix transposed to 3x4
    // Use bevy_render::maths::affine3_to_square to unpack
    world_from_local: mat3x4<f32>,
    // 3x3 matrix packed in mat2x4 and f32 as:
    // [0].xyz, [1].x,
    // [1].yz, [2].xy
    // [2].z
    // Use bevy_render::maths::mat2x4_f32_to_mat3x3_unpack to unpack
    local_from_world_transpose_a: mat2x4<f32>,
    local_from_world_transpose_b: f32,
    // 'flags' is a bit field indicating various options. u32 is 32 bits so we have up to 32 options.
    flags: u32,
    tag: u32,

    /// Color transform
    mult_color: vec4<f32>,
    add_color: vec4<f32>,
};
@group(1) @binding(0) var<storage> mesh: array<PartMesh2d>;

fn get_world_from_local(instance_index: u32) -> mat4x4<f32> {
    return affine3_to_square(mesh[instance_index].world_from_local);
}

fn mesh2d_position_local_to_world(world_from_local: mat4x4<f32>, vertex_position: vec4<f32>) -> vec4<f32> {
    return world_from_local * vertex_position;
}

fn mesh2d_position_world_to_clip(world_position: vec4<f32>) -> vec4<f32> {
    return view.clip_from_world * world_position;
}

fn part_mesh2d_color_transform(instance_index: u32, color: vec4<f32>) -> vec4<f32> {
    return color * mesh[instance_index].mult_color + mesh[instance_index].add_color;
}

const view_matrix: mat4x4<f32> = mat4x4<f32>(
    vec4<f32>(1.0, 0.0, 0.0, 0.0),
    vec4<f32>(0.0, -1.0, 0.0, 0.0),
    vec4<f32>(0.0, 0.0, 1.0, 0.0),
    vec4<f32>(0.0, 0.0, 0.0, 1.0)
);


/// Converts a color from linear to sRGB color space.
fn linear_to_srgb(linear_: vec4<f32>) -> vec4<f32> {
    var rgb: vec3<f32> = linear_.rgb;
    if linear_.a > 0.0 {
        rgb = rgb / linear_.a;
    }
    let a = 12.92 * rgb;
    let b = 1.055 * pow(rgb, vec3<f32>(1.0 / 2.4)) - 0.055;
    let c = step(vec3<f32>(0.0031308), rgb);
    return vec4<f32>(mix(a, b, c) * linear_.a, linear_.a);
}

fn srgb_to_linear(srgb: vec4<f32>) -> vec4<f32> {
    var rgb: vec3<f32> = srgb.rgb;
    if srgb.a > 0.0 {
        rgb = rgb / srgb.a;
    }
    let a = rgb / 12.92;
    let b = pow((rgb + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    let c = step(vec3<f32>(0.04045), rgb);
    return vec4<f32>(mix(a, b, c) * srgb.a, srgb.a);
}
