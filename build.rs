// Private filters use raw wgpu pipelines. Compile their WESL ahead of time so
// import resolution and shader-source generation add no runtime work.
fn main() {
    for name in [
        "shape",
        "composite",
        "blur",
        "glow",
        "bevel",
        "color_matrix",
        "convolution",
        "gradient_filter",
        "alpha_mask",
    ] {
        let path = format!("src/render/shaders/{name}.wesl");
        let shader = wesl::Compiler::default()
            .compile(&path)
            .unwrap_or_else(|error| panic!("cannot compile {path}: {error}"));
        shader.emit_rerun_if_changed();
        shader.write_artifact(name);
    }
}
