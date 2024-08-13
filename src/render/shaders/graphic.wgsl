#import bevy_sprite::{
    mesh2d_vertex_output::VertexOutput,
}
fn sRGB_OETF(a: f32) -> f32 {
    if .04045f < a {
        return pow((a + .055f) / 1.055f, 2.4f);
    } else {
        return  a / 12.92f;
    }
}

fn linear_from_srgba(srgba: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(
        sRGB_OETF(srgba.r),
        sRGB_OETF(srgba.g),
        sRGB_OETF(srgba.b),
        srgba.a
    );
}

@fragment
fn fragment(
    mesh: VertexOutput,
) -> @location(0) vec4<f32> {
    return mesh.color;
}