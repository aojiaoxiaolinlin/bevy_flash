#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput
#import bevy_flash::common::srgb_to_linear

struct Filter {
    color: vec4<f32>,
    strength: f32,
    inner: u32,
    knockout: u32,
    composite_source: u32,
}

@group(0) @binding(0) var texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;
@group(0) @binding(2) var<uniform> filter_args: Filter;
@group(0) @binding(3) var blurred: texture_2d<f32>;


@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let inner = filter_args.inner > 0u;
    let knockout = filter_args.knockout > 0u;
    let composite_source = filter_args.composite_source > 0u;
    var blur = textureSample(blurred, texture_sampler, in.uv).a;
    var dest = textureSample(texture, texture_sampler, in.uv);

    if in.uv.x < 0.0 || in.uv.x > 1.0 || in.uv.y < 0.0 || in.uv.y > 1.0 {
        blur = 0.0;
    }

    // [NA] It'd be nice to use hardware blending but the operation is too complex :( Only knockouts would work.

    // Start with 1 alpha because we'll be multiplying the whole thing
    // TODO: 这里是因为虽然当前滤镜效果是在线性色彩空间中，但是会将中间纹理渲染到SRGB的纹理上去，
    // 所以需要先将其转换为线性空间。
    // 如果以后要对混合模式实现在线性色彩空间中，这里将不再需要转换。
    // var color = vec4<f32>(filter_args.color.r, filter_args.color.g, filter_args.color.b, 1.0);
    var color = srgb_to_linear(vec4<f32>(filter_args.color.r, filter_args.color.g, filter_args.color.b, 1.0));
    if inner {
        let alpha = filter_args.color.a * saturate((1.0 - blur) * filter_args.strength);
        if knockout {
            color = color * alpha * dest.a;
        } else if composite_source {
            color = color * alpha * dest.a + dest * (1.0 - alpha);
        } else {
            // [NA] Yes it's intentional that the !composite_source is different for inner/outer. Just Flash things.
            color = color * alpha * dest.a;
        }
    } else {
        let alpha = filter_args.color.a * saturate(blur * filter_args.strength);
        if knockout {
            color = color * alpha * (1.0 - dest.a);
        } else if composite_source {
            color = color * alpha * (1.0 - dest.a) + dest;
        } else {
            color = color * alpha;
        }
    }

    return color;
}
