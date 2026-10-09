// Shared top-left UV convention for Flash post-processing.
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

fn fullscreen_vertex(index: u32) -> Out {
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

