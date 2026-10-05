struct Params { rect: vec4<f32>, viewport: vec4<f32>, mode: vec4<f32>, color: vec4<f32> };
@group(0) @binding(0) var<uniform> params: Params;
@group(1) @binding(0) var content: texture_2d<f32>;
@group(1) @binding(1) var content_sampler: sampler;
@group(2) @binding(0) var mask: texture_2d<f32>;
@group(2) @binding(1) var mask_sampler: sampler;

struct Out { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex fn vertex(@builtin(vertex_index) index: u32) -> Out {
    var out: Out;
    let uv = array<vec2<f32>,6>(
        vec2(0.0,0.0), vec2(1.0,0.0), vec2(0.0,1.0),
        vec2(0.0,1.0), vec2(1.0,0.0), vec2(1.0,1.0)
    )[index];
    let pixel = params.rect.xy + uv * params.rect.zw;
    let clip = (pixel - params.viewport.xy) / params.viewport.zw;
    out.position = vec4(clip.x * 2.0 - 1.0, 1.0 - clip.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment fn fragment(in: Out) -> @location(0) vec4<f32> {
    let value = textureSample(content, content_sampler, in.uv);
    let alpha = textureSample(mask, mask_sampler, in.uv).a;
    return value * alpha;
}
