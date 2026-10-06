struct Params {
    highlight_color: vec4<f32>,
    shadow_color: vec4<f32>,
    strength: f32,
    bevel_type: u32,
    knockout: u32,
    composite_source: u32,
    blur_offset: vec2<f32>,
    offset_padding: vec2<f32>,
    padding0: vec4<f32>,
    padding1: vec4<f32>,
    padding2: vec4<f32>,
    padding3: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(1) @binding(0) var source: texture_2d<f32>;
@group(1) @binding(1) var source_sampler: sampler;
@group(2) @binding(0) var blurred: texture_2d<f32>;
@group(2) @binding(1) var blurred_sampler: sampler;

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

fn sample_blur_alpha(uv: vec2<f32>) -> f32 {
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        return 0.0;
    }
    // Explicit LOD permits sampling after the per-fragment bounds check on WebGPU.
    return textureSampleLevel(blurred, blurred_sampler, uv, 0.0).a;
}

@fragment
fn fragment(in: Out) -> @location(0) vec4<f32> {
    let dest = textureSample(source, source_sampler, in.uv);
    let blur_left = sample_blur_alpha(in.uv + params.blur_offset);
    let blur_right = sample_blur_alpha(in.uv - params.blur_offset);
    let highlight_alpha = clamp((blur_left - blur_right) * params.strength, 0.0, 1.0);
    let shadow_alpha = clamp((blur_right - blur_left) * params.strength, 0.0, 1.0);
    let highlight = vec4(
        params.highlight_color.rgb * params.highlight_color.a,
        params.highlight_color.a
    );
    let shadow = vec4(
        params.shadow_color.rgb * params.shadow_color.a,
        params.shadow_color.a
    );
    let bevel = highlight * highlight_alpha + shadow * shadow_alpha;
    let outer = params.bevel_type == 0u || params.bevel_type == 2u;
    let inner = params.bevel_type == 1u || params.bevel_type == 2u;
    let knockout = params.knockout != 0u;

    if inner && outer {
        if knockout || params.composite_source == 0u {
            return bevel;
        }
        return dest - dest * bevel.a + bevel;
    }
    if inner {
        if knockout || params.composite_source == 0u {
            return bevel * dest.a;
        }
        return bevel * dest.a + dest * (1.0 - bevel.a);
    }
    if knockout {
        return bevel * (1.0 - dest.a);
    }
    if params.composite_source != 0u {
        return dest + bevel * (1.0 - dest.a);
    }
    return bevel;
}
