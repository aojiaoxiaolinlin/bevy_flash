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

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> Out {
    let positions = array<vec2<f32>, 6>(
        vec2(-1.0, 1.0), vec2(1.0, 1.0), vec2(-1.0, -1.0),
        vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(1.0, -1.0)
    );
    let uvs = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0)
    );
    var out: Out;
    out.position = vec4(positions[index], 0.0, 1.0);
    out.uv = uvs[index];
    return out;
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
