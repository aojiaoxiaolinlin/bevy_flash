#define_import_path bevy_flash::common

const view_matrix: mat4x4<f32> = mat4x4<f32>(
    vec4<f32>(1.0, 0.0, 0.0, 0.0),
    vec4<f32>(0.0, -1.0, 0.0, 0.0),
    vec4<f32>(0.0, 0.0, 1.0, 0.0),
    vec4<f32>(0.0, 0.0, 0.0, 1.0)
);

struct MaterialTransform {
    world_matrix: mat4x4<f32>,
    mult_color: vec4<f32>,
    add_color: vec4<f32>,
};
