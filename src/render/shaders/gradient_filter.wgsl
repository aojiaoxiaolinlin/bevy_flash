struct Params {
    strength: f32,
    filter_type: u32,
    knockout: u32,
    composite_source: u32,
    kind: vec4<u32>,
    blur_offset: vec2<f32>,
    offset_padding: vec2<f32>,
    padding0: vec4<f32>,
    padding1: vec4<f32>,
    padding2: vec4<f32>,
    padding3: vec4<f32>,
    padding4: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(1) @binding(0) var source: texture_2d<f32>;
@group(1) @binding(1) var source_sampler: sampler;
@group(2) @binding(0) var blurred: texture_2d<f32>;
@group(2) @binding(1) var blurred_sampler: sampler;
@group(3) @binding(0) var ramp: texture_2d<f32>;
@group(3) @binding(1) var ramp_sampler: sampler;

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

fn blur_alpha(uv: vec2<f32>) -> f32 {
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        return 0.0;
    }
    // Explicit LOD permits sampling after the per-fragment bounds check on WebGPU.
    return textureSampleLevel(blurred, blurred_sampler, uv, 0.0).a;
}

fn gradient_color(value: f32) -> vec4<f32> {
    let index = i32(round(clamp(value, 0.0, 1.0) * 255.0));
    let color = textureLoad(ramp, vec2(index, 0), 0);
    return vec4(color.rgb * color.a, color.a);
}

fn composite(effect: vec4<f32>, dest: vec4<f32>) -> vec4<f32> {
    let outer = params.filter_type == 0u || params.filter_type == 2u;
    let inner = params.filter_type == 1u || params.filter_type == 2u;
    let knockout = params.knockout != 0u;
    let keep_source = params.composite_source != 0u;
    if inner && outer {
        if knockout || !keep_source {
            return effect;
        }
        return effect + dest * (1.0 - effect.a);
    }
    if inner {
        let masked = effect * dest.a;
        if knockout || !keep_source {
            return masked;
        }
        return masked + dest * (1.0 - effect.a);
    }
    let masked = effect * (1.0 - dest.a);
    if knockout {
        return masked;
    }
    if keep_source {
        return masked + dest;
    }
    return effect;
}

@fragment
fn fragment(in: Out) -> @location(0) vec4<f32> {
    let dest = textureSample(source, source_sampler, in.uv);
    var value: f32;
    if params.kind.x == 0u {
        value = blur_alpha(in.uv + params.blur_offset) * params.strength;
    } else {
        let left = blur_alpha(in.uv + params.blur_offset);
        let right = blur_alpha(in.uv - params.blur_offset);
        // GradientBevel arrays conventionally run highlight → base → shadow.
        value = 0.5 - (left - right) * params.strength * 0.5;
    }
    return composite(gradient_color(value), dest);
}
