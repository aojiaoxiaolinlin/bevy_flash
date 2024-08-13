use indexmap::IndexSet;
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, StrokeOptions, StrokeTessellator, VertexBuffers,
};

use ruffle_render::{
    shape_utils::{DistilledShape, DrawPath, GradientType},
    tessellator::{
        ruffle_path_to_lyon_path, swf_gradient_to_uniforms, swf_to_gl_matrix, Draw, DrawType,
        Gradient, Mesh, RuffleVertexCtor, Vertex,
    },
};

pub struct ShapeTessellator {
    fill_tess: FillTessellator,
    stroke_tess: StrokeTessellator,
    mesh: Vec<Draw>,
    gradients: IndexSet<Gradient>,
    lyon_mesh: VertexBuffers<Vertex, u32>,
    mask_index_count: Option<u32>,
    is_stroke: bool,
}

impl ShapeTessellator {
    pub fn new() -> Self {
        Self {
            fill_tess: FillTessellator::new(),
            stroke_tess: StrokeTessellator::new(),
            mesh: Vec::new(),
            gradients: IndexSet::new(),
            lyon_mesh: VertexBuffers::new(),
            mask_index_count: None,
            is_stroke: false,
        }
    }

    pub fn tessellate_shape(&mut self, shape: DistilledShape) -> Mesh {
        self.mesh = Vec::new();
        self.gradients = IndexSet::new();
        self.lyon_mesh = VertexBuffers::new();

        for path in shape.paths {
            let (fill_style, lyon_path, next_is_stroke) = match &path {
                DrawPath::Fill {
                    style,
                    commands,
                    winding_rule: _,
                } => (*style, ruffle_path_to_lyon_path(commands, true), false),
                DrawPath::Stroke {
                    style,
                    commands,
                    is_closed,
                } => (
                    style.fill_style(),
                    ruffle_path_to_lyon_path(commands, *is_closed),
                    true,
                ),
            };

            let (draw, color, needs_flush) = match fill_style {
                swf::FillStyle::Color(color) => (DrawType::Color, *color, false),
                swf::FillStyle::LinearGradient(gradient) => {
                    let uniform =
                        swf_gradient_to_uniforms(GradientType::Linear, gradient, swf::Fixed8::ZERO);
                    let (gradient_index, _) = self.gradients.insert_full(uniform);

                    (
                        DrawType::Gradient {
                            matrix: swf_to_gl_matrix(gradient.matrix.into()),
                            gradient: gradient_index,
                        },
                        swf::Color::WHITE,
                        true,
                    )
                }
                swf::FillStyle::RadialGradient(gradient) => {
                    let uniform =
                        swf_gradient_to_uniforms(GradientType::Radial, gradient, swf::Fixed8::ZERO);
                    let (gradient_index, _) = self.gradients.insert_full(uniform);
                    (
                        DrawType::Gradient {
                            matrix: swf_to_gl_matrix(gradient.matrix.into()),
                            gradient: gradient_index,
                        },
                        swf::Color::WHITE,
                        true,
                    )
                }
                swf::FillStyle::FocalGradient {
                    gradient,
                    focal_point,
                } => {
                    let uniform =
                        swf_gradient_to_uniforms(GradientType::Focal, gradient, *focal_point);
                    let (gradient_index, _) = self.gradients.insert_full(uniform);
                    (
                        DrawType::Gradient {
                            matrix: swf_to_gl_matrix(gradient.matrix.into()),
                            gradient: gradient_index,
                        },
                        swf::Color::WHITE,
                        true,
                    )
                }
                swf::FillStyle::Bitmap {
                    id,
                    matrix,
                    is_smoothed,
                    is_repeating,
                } => continue,
            };

            if needs_flush || (self.is_stroke && !next_is_stroke) {
                // We flush separate draw calls in these cases:
                // * Non-solid color fills which require their own shader.
                // * Strokes followed by fills, because strokes need to be omitted
                //   when using this shape as a mask.
                self.flush_draw(DrawType::Color);
            } else if !self.is_stroke && next_is_stroke {
                // Bake solid color fills followed by strokes into a single draw call, and adjust
                // the index count to omit the strokes when rendering this shape as a mask.
                assert!(self.mask_index_count.is_none());
                self.mask_index_count = Some(self.lyon_mesh.indices.len() as u32);
            }
            self.is_stroke = next_is_stroke;

            let mut buffers_builder =
                BuffersBuilder::new(&mut self.lyon_mesh, RuffleVertexCtor { color });
            let result = match path {
                DrawPath::Fill { winding_rule, .. } => self.fill_tess.tessellate_path(
                    &lyon_path,
                    &FillOptions::default().with_fill_rule(winding_rule.into()),
                    &mut buffers_builder,
                ),
                DrawPath::Stroke { style, .. } => {
                    // TODO(Herschel): 0 width indicates "hairline".
                    let width = (style.width().to_pixels() as f32).max(1.0);
                    let mut stroke_options = StrokeOptions::default()
                        .with_line_width(width)
                        .with_start_cap(match style.start_cap() {
                            swf::LineCapStyle::None => lyon_tessellation::LineCap::Butt,
                            swf::LineCapStyle::Round => lyon_tessellation::LineCap::Round,
                            swf::LineCapStyle::Square => lyon_tessellation::LineCap::Square,
                        })
                        .with_end_cap(match style.end_cap() {
                            swf::LineCapStyle::None => lyon_tessellation::LineCap::Butt,
                            swf::LineCapStyle::Round => lyon_tessellation::LineCap::Round,
                            swf::LineCapStyle::Square => lyon_tessellation::LineCap::Square,
                        });

                    let line_join = match style.join_style() {
                        swf::LineJoinStyle::Round => lyon_tessellation::LineJoin::Round,
                        swf::LineJoinStyle::Bevel => lyon_tessellation::LineJoin::Bevel,
                        swf::LineJoinStyle::Miter(limit) => {
                            // Avoid lyon assert with small miter limits.
                            let limit = limit.to_f32();
                            if limit >= StrokeOptions::MINIMUM_MITER_LIMIT {
                                stroke_options = stroke_options.with_miter_limit(limit);
                                lyon_tessellation::LineJoin::MiterClip
                            } else {
                                lyon_tessellation::LineJoin::Bevel
                            }
                        }
                    };
                    stroke_options = stroke_options.with_line_join(line_join);
                    self.stroke_tess.tessellate_path(
                        &lyon_path,
                        &stroke_options,
                        &mut buffers_builder,
                    )
                }
            };
            match result {
                Ok(_) => {
                    if needs_flush {
                        // Non-solid color fills are isolated draw calls; flush immediately.
                        self.flush_draw(draw);
                    }
                }
                Err(e) => {
                    // This may simply be a degenerate path.
                    dbg!("Tessellation failure: {:?}", e);
                }
            }
        }

        // Flush the final pending draw.
        self.flush_draw(DrawType::Color);

        self.lyon_mesh = VertexBuffers::new();
        Mesh {
            draws: std::mem::take(&mut self.mesh),
            gradients: std::mem::take(&mut self.gradients).into_iter().collect(),
        }
    }

    fn flush_draw(&mut self, draw: DrawType) {
        if self.lyon_mesh.vertices.is_empty() || self.lyon_mesh.indices.len() < 3 {
            // Ignore degenerate fills
            self.lyon_mesh = VertexBuffers::new();
            self.mask_index_count = None;
            return;
        }
        let draw_mesh = std::mem::replace(&mut self.lyon_mesh, VertexBuffers::new());
        self.mesh.push(Draw {
            draw_type: draw,
            mask_index_count: self
                .mask_index_count
                .unwrap_or(draw_mesh.indices.len() as u32),
            vertices: draw_mesh.vertices,
            indices: draw_mesh.indices,
        });
        self.mask_index_count = None;
    }
}
