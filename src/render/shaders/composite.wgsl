struct Params { rect: vec4<f32>, viewport: vec4<f32>, mode: vec4<f32>, color: vec4<f32> };
@group(0) @binding(0) var<uniform> params: Params;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;
struct Out { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
fn to_linear(v: vec3<f32>) -> vec3<f32> {
    return select(v/12.92, pow((max(v,vec3(0.0))+0.055)/1.055,vec3(2.4)), v > vec3(0.04045));
}
@vertex fn vertex(@builtin(vertex_index) index: u32) -> Out {
    var out: Out;
    let uv = array<vec2<f32>,6>(vec2(0.0,0.0),vec2(1.0,0.0),vec2(0.0,1.0),vec2(0.0,1.0),vec2(1.0,0.0),vec2(1.0,1.0))[index];
    let pixel = params.rect.xy + uv*params.rect.zw;
    let clip = (pixel-params.viewport.xy)/params.viewport.zw;
    out.position = vec4(clip.x*2.0-1.0,1.0-clip.y*2.0,0.0,1.0);
    out.uv = uv;
    return out;
}
@fragment fn fragment(in: Out) -> @location(0) vec4<f32> {
    var c = textureSample(tex,samp,in.uv);
    if params.mode.x == 1.0 {
        // Public Image uses straight alpha for Bevy Sprite/UI material consumers.
        if c.a > 0.0 { c = vec4(to_linear(c.rgb/c.a),c.a); }
    }
    return c;
}
