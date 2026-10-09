import super::fullscreen::{Out, fullscreen_vertex};

struct Params {
    row0: vec4<f32>,
    row1: vec4<f32>,
    row2: vec4<f32>,
    row3: vec4<f32>,
    offsets: vec4<f32>,
    padding0: vec4<f32>,
    padding1: vec4<f32>,
    padding2: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(1) @binding(0) var source: texture_2d<f32>;
@group(1) @binding(1) var source_sampler: sampler;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> Out {
    return fullscreen_vertex(index);
}

@fragment
fn fragment(in: Out) -> @location(0) vec4<f32> {
    let premultiplied = textureSample(source, source_sampler, in.uv);
    let straight_rgb = select(
        vec3(0.0),
        premultiplied.rgb / max(premultiplied.a, 0.000001),
        premultiplied.a > 0.0,
    );
    let straight = vec4(straight_rgb, premultiplied.a);
    let transformed = clamp(vec4(
        dot(params.row0, straight) + params.offsets.r,
        dot(params.row1, straight) + params.offsets.g,
        dot(params.row2, straight) + params.offsets.b,
        dot(params.row3, straight) + params.offsets.a,
    ), vec4(0.0), vec4(1.0));
    return vec4(transformed.rgb * transformed.a, transformed.a);
}
