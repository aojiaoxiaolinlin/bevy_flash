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