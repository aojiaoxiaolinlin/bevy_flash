struct Draw {
    row0: vec4<f32>, row1: vec4<f32>,
    uv0: vec4<f32>, uv1: vec4<f32>,
    multiply: vec4<f32>, add: vec4<f32>,
    material: vec4<f32>, viewport: vec4<f32>,
};
@group(0) @binding(0) var<uniform> draw: Draw;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>, @location(1) color: vec4<f32>,
};
fn to_srgb(v: vec3<f32>) -> vec3<f32> {
    return select(12.92*v, 1.055*pow(max(v,vec3(0.0)),vec3(1.0/2.4))-0.055, v > vec3(0.0031308));
}
@vertex fn vertex(@location(0) position: vec3<f32>, @location(1) color: vec4<f32>) -> Out {
    var out: Out;
    let p = vec3(position.xy,1.0);
    let pixel = vec2(dot(draw.row0.xyz,p),dot(draw.row1.xyz,p));
    let clip = (pixel-draw.viewport.xy)/draw.viewport.zw;
    out.position = vec4(clip.x*2.0-1.0,1.0-clip.y*2.0,0.0,1.0);
    out.uv = vec2(dot(draw.uv0.xyz,p),dot(draw.uv1.xyz,p));
    out.color = color;
    return out;
}
@fragment fn fragment(in: Out) -> @location(0) vec4<f32> {
    var color = vec4(to_srgb(in.color.rgb),in.color.a);
    if draw.material.w == 1.0 {
        // VATF bitmap bytes are premultiplied in encoded sRGB, sampled as Unorm.
        color = textureSample(tex,samp,in.uv);
        if color.a > 0.0 { color = vec4(color.rgb/color.a,color.a); }
    } else if draw.material.w == 2.0 {
        var t = in.uv.x;
        if draw.material.y == 2.0 { t = length(in.uv*2.0-1.0); }
        if draw.material.y == 3.0 {
            let delta = vec2(draw.material.x,0.0)-(in.uv*2.0-1.0);
            let len = length(delta);
            let direction = delta/max(len,0.000001);
            t = len / max(sqrt(max(0.0,1.0-draw.material.x*draw.material.x*direction.y*direction.y))+draw.material.x*direction.x,0.000001);
        }
        if draw.material.z == 1.0 { t = clamp(t,0.0,1.0); }
        else if draw.material.z == 2.0 { t = 1.0-abs(fract(t*0.5)*2.0-1.0); }
        else { t = fract(t); }
        // The compiler stores a 256-entry encoded-color ramp.
        color = textureSample(tex,samp,vec2((t*255.0+0.5)/256.0,0.5));
    }
    color = clamp(color*draw.multiply+draw.add,vec4(0.0),vec4(1.0));
    // Flash/Ruffle render their working surface as Rgba8Unorm and perform
    // filters and blending on encoded sRGB channel values. Keep this private
    // offscreen target in that same legacy working space; conversion to the
    // Bevy view's linear space happens only when the layer is composited.
    return vec4(color.rgb*color.a,color.a);
}
