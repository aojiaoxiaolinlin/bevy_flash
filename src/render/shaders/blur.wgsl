struct Params {
    direction: vec2<f32>,
    full_size: f32,
    m: f32,
    m2: f32,
    first_weight: f32,
    last_offset: f32,
    last_weight: f32,
    padding0: vec4<f32>,
    padding1: vec4<f32>,
    padding2: vec4<f32>,
    padding3: vec4<f32>,
    padding4: vec4<f32>,
    padding5: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

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
    var uv = in.uv - params.direction * params.m;
    var total = textureSample(tex, samp, uv - params.direction) * params.first_weight;
    var center = vec4(0.0);
    for (var i = 0.5; i < params.m2; i += 2.0) {
        center += textureSample(tex, samp, uv + params.direction * i);
    }
    total += center * 2.0;
    let last_location = uv + params.direction * (params.m2 + params.last_offset);
    total += textureSample(tex, samp, last_location) * params.last_weight;
    return floor(total / params.full_size * 255.0) / 255.0;
}
