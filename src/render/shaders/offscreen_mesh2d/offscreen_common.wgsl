#define_import_path bevy_flash::offscreen_common


// Offscreen mesh 2d pipeline
struct TransformUniform {
    world_matrix: mat4x4<f32>,
    mult_color: vec4<f32>,
    add_color: vec4<f32>,
}
