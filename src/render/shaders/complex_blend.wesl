struct Params { rect: vec4<f32>, viewport: vec4<f32>, mode: vec4<f32>, color: vec4<f32> };
@group(0) @binding(0) var<uniform> params: Params;
@group(1) @binding(0) var src_tex: texture_2d<f32>;
@group(1) @binding(1) var src_samp: sampler;
@group(2) @binding(0) var dst_tex: texture_2d<f32>;
@group(2) @binding(1) var dst_samp: sampler;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) src_uv: vec2<f32>,
    @location(1) dst_uv: vec2<f32>,
};

@vertex fn vertex(@builtin(vertex_index) index: u32) -> Out {
    var out: Out;
    let uv = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    )[index];
    let draw_rect = select(params.rect, params.viewport, params.mode.y > 0.5);
    let pixel = draw_rect.xy + uv * draw_rect.zw;
    let dst_uv = (pixel - params.viewport.xy) / params.viewport.zw;
    let src_uv = (pixel - params.rect.xy) / params.rect.zw;
    out.position = vec4(dst_uv.x * 2.0 - 1.0, 1.0 - dst_uv.y * 2.0, 0.0, 1.0);
    out.src_uv = src_uv;
    out.dst_uv = dst_uv;
    return out;
}

@fragment fn fragment(in: Out) -> @location(0) vec4<f32> {
    let dst = textureSample(dst_tex, dst_samp, in.dst_uv);
    let in_src = all(in.src_uv >= vec2(0.0)) && all(in.src_uv <= vec2(1.0));
    let sampled_src = textureSample(src_tex, src_samp, clamp(in.src_uv, vec2(0.0), vec2(1.0)));
    let src = select(vec4(0.0), sampled_src, in_src);

    let sa = src.a;
    let da = dst.a;
    let mode = i32(params.mode.x + 0.5);

    // Alpha / Erase modulate premultiplied destination directly by source alpha.
    if mode == 6 {
        return dst * sa;
    }
    if mode == 7 {
        return dst * (1.0 - sa);
    }
    if mode == 5 {
        // Flash Invert: invert destination RGB weighted by source alpha, keeping dst alpha.
        let rgb = dst.rgb * (1.0 - sa) + max(vec3(da) - dst.rgb, vec3(0.0)) * sa;
        return vec4(clamp(rgb, vec3(0.0), vec3(1.0)), da);
    }

    let out_a = sa + da * (1.0 - sa);
    let base = src.rgb * (1.0 - da) + dst.rgb * (1.0 - sa);

    var blended = vec3(0.0);
    if mode == 1 {
        // Multiply: sa * da * (sc * dc) = src.rgb * dst.rgb
        blended = src.rgb * dst.rgb;
    } else if mode == 2 {
        // Exact Lighten: sa * da * max(sc, dc) = max(src.rgb * da, dst.rgb * sa)
        blended = max(src.rgb * da, dst.rgb * sa);
    } else if mode == 3 {
        // Darken: sa * da * min(sc, dc) = min(src.rgb * da, dst.rgb * sa)
        blended = min(src.rgb * da, dst.rgb * sa);
    } else if mode == 4 {
        // Difference: sa * da * |dc - sc| = |dst.rgb * sa - src.rgb * da|
        blended = abs(dst.rgb * sa - src.rgb * da);
    } else {
        let sc = select(vec3(0.0), clamp(src.rgb / sa, vec3(0.0), vec3(1.0)), sa > 0.0);
        let dc = select(vec3(0.0), clamp(dst.rgb / da, vec3(0.0), vec3(1.0)), da > 0.0);
        var b = sc;
        if mode == 8 {
            // Overlay: conditional on destination color
            b = select(
                1.0 - 2.0 * (1.0 - dc) * (1.0 - sc),
                2.0 * dc * sc,
                dc <= vec3(0.5),
            );
        } else if mode == 9 {
            // HardLight: conditional on source color
            b = select(
                1.0 - 2.0 * (1.0 - dc) * (1.0 - sc),
                2.0 * dc * sc,
                sc <= vec3(0.5),
            );
        }
        blended = sa * da * b;
    }

    return vec4(clamp(base + blended, vec3(0.0), vec3(1.0)), clamp(out_a, 0.0, 1.0));
}
