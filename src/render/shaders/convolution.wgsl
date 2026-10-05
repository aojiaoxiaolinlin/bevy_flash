struct Params {
    source_size: vec2<f32>,
    divisor: f32,
    bias: f32,
    rows: u32,
    cols: u32,
    preserve_alpha: u32,
    clamp_edges: u32,
    default_color: vec4<f32>,
    padding0: vec4<f32>,
    padding1: vec4<f32>,
    padding2: vec4<f32>,
    padding3: vec4<f32>,
    padding4: vec4<f32>,
};

struct Kernel {
    weights: array<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(1) @binding(0) var source: texture_2d<f32>;
@group(1) @binding(1) var source_sampler: sampler;
@group(2) @binding(0) var<storage, read> kernel: Kernel;

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

fn straight_sample(position: vec2<i32>, dimensions: vec2<i32>) -> vec4<f32> {
    var sample_position = position;
    if position.x < 0 || position.y < 0 || position.x >= dimensions.x || position.y >= dimensions.y {
        if params.clamp_edges == 0u {
            return params.default_color;
        }
        sample_position = clamp(position, vec2(0), dimensions - vec2(1));
    }
    let color = textureLoad(source, sample_position, 0);
    if color.a <= 0.000001 {
        return vec4(0.0);
    }
    return vec4(color.rgb / color.a, color.a);
}

@fragment
fn fragment(in: Out) -> @location(0) vec4<f32> {
    let dimensions = vec2<i32>(textureDimensions(source));
    let position = min(vec2<i32>(in.uv * params.source_size), dimensions - vec2(1));
    let center = vec2<i32>(i32(params.cols / 2u), i32(params.rows / 2u));
    var result = vec4(0.0);
    for (var row = 0u; row < params.rows; row += 1u) {
        for (var col = 0u; col < params.cols; col += 1u) {
            let offset = vec2<i32>(i32(col), i32(row)) - center;
            result += straight_sample(position + offset, dimensions)
                * kernel.weights[row * params.cols + col];
        }
    }
    result = result / params.divisor + vec4(params.bias);
    let original = textureLoad(source, position, 0);
    if params.preserve_alpha != 0u {
        result.a = original.a;
    }
    result = clamp(result, vec4(0.0), vec4(1.0));
    return vec4(result.rgb * result.a, result.a);
}
