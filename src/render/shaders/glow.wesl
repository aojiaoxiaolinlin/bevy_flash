import super::fullscreen::{Out, fullscreen_vertex};

struct Params {
    color: vec4<f32>,
    strength: f32,
    inner: u32,
    knockout: u32,
    composite_source: u32,
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

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> Out {
    return fullscreen_vertex(index);
}

@fragment
fn fragment(in: Out) -> @location(0) vec4<f32> {
    let inner = params.inner != 0u;
    let knockout = params.knockout != 0u;
    let composite_source = params.composite_source != 0u;
    let blur_uv = in.uv + params.blur_offset;
    var blur_alpha = textureSample(blurred, blurred_sampler, blur_uv).a;
    if blur_uv.x < 0.0 || blur_uv.x > 1.0 || blur_uv.y < 0.0 || blur_uv.y > 1.0 {
        blur_alpha = 0.0;
    }
    let dest = textureSample(source, source_sampler, in.uv);
    let glow_rgb = params.color.rgb;
    var alpha: f32;
    if inner {
        alpha = params.color.a * clamp((1.0 - blur_alpha) * params.strength, 0.0, 1.0);
        let glow = vec4(glow_rgb * alpha, alpha) * dest.a;
        if knockout || !composite_source {
            return glow;
        }
        return glow + dest * (1.0 - alpha);
    }
    alpha = params.color.a * clamp(blur_alpha * params.strength, 0.0, 1.0);
    let glow = vec4(glow_rgb * alpha, alpha);
    if knockout {
        return glow * (1.0 - dest.a);
    }
    if composite_source {
        return glow * (1.0 - dest.a) + dest;
    }
    return glow;
}
