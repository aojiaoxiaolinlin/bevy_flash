use bevy::{
    asset::{Handle, RenderAssetUsages},
    color::ColorToComponents,
    log::warn,
    math::{Mat3, Mat4},
    mesh::{Indices, Mesh, PrimitiveTopology},
    platform::collections::HashMap,
};
use copyless::VecHelper;
use std::sync::Arc;
use swf::{CharacterId, Color, Fixed8, Fixed16, Point, Rectangle, Twips};

use crate::{
    assets::MeshDraw,
    swf_runtime::{
        shape_utils::calculate_shape_bounds,
        tessellator::{DrawType, ShapeTessellator},
    },
};
use crate::{
    assets::{MaterialType, Shape, create_gradient_textures},
    render::material::GradientMaterial,
};

use super::{
    display_object::{DisplayObject, DisplayObjectBase, TDisplayObject},
    tag_utils::SwfMovie,
};

/// 为变形形状预先计算的中间框架。
#[derive(Debug, Clone)]

pub struct Frame {
    handle: Option<Handle<Shape>>,
    shape: swf::Shape,
    bounds: Rectangle<Twips>,
}

#[derive(Debug, Clone)]
pub struct MorphShape {
    id: CharacterId,
    start: swf::MorphShape,
    end: swf::MorphShape,
    // frames: fnv::FnvHashMap<u16, Frame>,
    base: DisplayObjectBase,
    ratio: u16,
    movie: Arc<SwfMovie>,
}

impl MorphShape {
    pub fn from_swf_tag(swf_tag: &swf::DefineMorphShape, movie: Arc<SwfMovie>) -> Self {
        Self {
            id: swf_tag.id,
            start: swf_tag.start.clone(),
            end: swf_tag.end.clone(),
            // frames: fnv::FnvHashMap::default(),
            base: DisplayObjectBase::default(),
            ratio: 0,
            movie,
        }
    }

    /// 延迟初始化该变形形状的中间帧
    fn get_frame<'a>(
        &self,
        ratio: u16,
        morph_shape_cache: &'a mut HashMap<CharacterId, fnv::FnvHashMap<u16, Frame>>,
    ) -> &'a mut Frame {
        morph_shape_cache
            .entry(self.id())
            .or_default()
            .entry(ratio)
            .or_insert_with(|| Self::build_morph_frame(&self.start, &self.end, ratio))
    }

    fn get_shape(&mut self, ratio: u16, context: &mut crate::RenderContext) -> Handle<Shape> {
        let frame = self.get_frame(ratio, context.morph_shape_cache);
        if let Some(handle) = &frame.handle {
            handle.clone()
        } else {
            let bitmaps = HashMap::new();
            let mut tessellator = ShapeTessellator::default();
            let shape = &frame.shape;
            let lyon_mesh = tessellator.tessellate_shape(shape.into(), &bitmaps);
            let mut gradient_texture = Vec::new();
            for (texture, gradient_uniforms) in create_gradient_textures(lyon_mesh.gradients) {
                gradient_texture.push((context.images.add(texture), gradient_uniforms));
            }
            let mut shape = Vec::new();
            for draw in lyon_mesh.draws {
                match &draw.draw_type {
                    DrawType::Color => {
                        let mut positions = Vec::with_capacity(draw.vertices.len());
                        let mut colors = Vec::with_capacity(draw.vertices.len());
                        for vertex in &draw.vertices {
                            positions.alloc().init([vertex.x, vertex.y, 0.0]);
                            let linear_color = bevy::color::Color::srgba_u8(
                                vertex.color.r,
                                vertex.color.g,
                                vertex.color.b,
                                vertex.color.a,
                            )
                            .to_linear();
                            colors.alloc().init(linear_color.to_f32_array());
                        }
                        let mesh = Mesh::new(
                            PrimitiveTopology::TriangleList,
                            RenderAssetUsages::RENDER_WORLD,
                        )
                        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
                        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
                        .with_inserted_indices(Indices::U32(draw.indices.into_iter().collect()));
                        let mesh = context.meshes.add(mesh);
                        shape.push(MeshDraw {
                            mesh,
                            material_type: MaterialType::Color(context.color_material.clone()),
                        });
                    }
                    DrawType::Gradient { matrix, gradient } => {
                        let Some((handle, gradient)) = gradient_texture.get(*gradient).cloned()
                        else {
                            continue;
                        };
                        let mut positions = Vec::with_capacity(draw.vertices.len());
                        for vertex in &draw.vertices {
                            positions.alloc().init([vertex.x, vertex.y, 0.0]);
                        }
                        let mesh = Mesh::new(
                            PrimitiveTopology::TriangleList,
                            RenderAssetUsages::default(),
                        )
                        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
                        .with_inserted_indices(Indices::U32(draw.indices.into_iter().collect()));
                        let mesh = context.meshes.add(mesh);
                        let material = context.gradients.add(GradientMaterial {
                            gradient,
                            texture: handle,
                            texture_transform: Mat4::from_mat3(Mat3::from_cols_array_2d(matrix)),
                            ..Default::default()
                        });
                        shape.push(MeshDraw {
                            mesh,
                            material_type: MaterialType::Gradient(material),
                        });
                    }
                    _ => {}
                }
            }
            let handle = context.shapes.add(Shape(shape));
            frame.handle = Some(handle.clone());
            handle
        }
    }

    fn build_morph_frame(start: &swf::MorphShape, end: &swf::MorphShape, ratio: u16) -> Frame {
        use swf::{FillStyle, LineStyle, ShapeRecord, ShapeStyles};
        let b = f32::from(ratio) / 65535.0;
        let a = 1.0 - b;
        let fill_styles: Vec<FillStyle> = start
            .fill_styles
            .iter()
            .zip(end.fill_styles.iter())
            .map(|(start, end)| lerp_fill(start, end, a, b))
            .collect();
        let line_styles: Vec<LineStyle> = start
            .line_styles
            .iter()
            .zip(end.line_styles.iter())
            .map(|(start, end)| {
                start
                    .clone()
                    .with_width(lerp_twips(start.width(), end.width(), a, b))
                    .with_fill_style(lerp_fill(start.fill_style(), end.fill_style(), a, b))
            })
            .collect();
        let mut shape = Vec::with_capacity(start.shape.len());
        let mut start_iter = start.shape.iter();
        let mut end_iter = end.shape.iter();
        let mut start = start_iter.next();
        let mut end = end_iter.next();
        let mut start_x = Twips::ZERO;
        let mut start_y = Twips::ZERO;
        let mut end_x = Twips::ZERO;
        let mut end_y = Twips::ZERO;
        // TODO: Feels like this could be cleaned up a bit.
        // We step through both the start records and end records, interpolating edges pairwise.
        // Fill style/line style changes should only appear in the start records.
        // However, StyleChangeRecord move_to can appear it both start and end records,
        // and not necessarily in matching pairs; therefore, we have to keep track of the pen position
        // in case one side is missing a move_to; it will implicitly use the last pen position.
        while let (Some(s), Some(e)) = (start, end) {
            match (s, e) {
                (ShapeRecord::StyleChange(start_change), ShapeRecord::StyleChange(end_change)) => {
                    let mut style_change = start_change.clone();
                    if start_change.move_to.is_some() || end_change.move_to.is_some() {
                        if let Some(move_to) = &start_change.move_to {
                            start_x = move_to.x;
                            start_y = move_to.y;
                        }
                        if let Some(move_to) = &end_change.move_to {
                            end_x = move_to.x;
                            end_y = move_to.y;
                        }
                        style_change.move_to = Some(Point::new(
                            lerp_twips(start_x, end_x, a, b),
                            lerp_twips(start_y, end_y, a, b),
                        ));
                    }
                    shape.push(ShapeRecord::StyleChange(style_change));
                    start = start_iter.next();
                    end = end_iter.next();
                }
                (ShapeRecord::StyleChange(start_change), _) => {
                    let mut style_change = start_change.clone();
                    if let Some(move_to) = &start_change.move_to {
                        start_x = move_to.x;
                        start_y = move_to.y;
                        style_change.move_to = Some(Point::new(
                            lerp_twips(start_x, end_x, a, b),
                            lerp_twips(start_y, end_y, a, b),
                        ));
                    }
                    shape.push(ShapeRecord::StyleChange(style_change));
                    Self::update_pos(&mut start_x, &mut start_y, s);
                    start = start_iter.next();
                }
                (_, ShapeRecord::StyleChange(end_change)) => {
                    let mut style_change = end_change.clone();
                    if let Some(move_to) = &end_change.move_to {
                        end_x = move_to.x;
                        end_y = move_to.y;
                        style_change.move_to = Some(Point::new(
                            lerp_twips(start_x, end_x, a, b),
                            lerp_twips(start_y, end_y, a, b),
                        ));
                    }
                    shape.push(ShapeRecord::StyleChange(style_change));
                    Self::update_pos(&mut end_x, &mut end_y, s);
                    end = end_iter.next();
                    continue;
                }
                _ => {
                    shape.push(lerp_edges(
                        Point::new(start_x, start_y),
                        Point::new(end_x, end_y),
                        s,
                        e,
                        a,
                        b,
                    ));
                    Self::update_pos(&mut start_x, &mut start_y, s);
                    Self::update_pos(&mut end_x, &mut end_y, e);
                    start = start_iter.next();
                    end = end_iter.next();
                }
            }
        }

        let styles = ShapeStyles {
            fill_styles,
            line_styles,
        };

        let bounds = calculate_shape_bounds(&shape);
        let shape = swf::Shape {
            version: 4,
            id: 0,
            shape_bounds: bounds.clone(),
            edge_bounds: bounds.clone(),
            flags: swf::ShapeFlag::HAS_SCALING_STROKES,
            styles,
            shape,
        };

        Frame {
            handle: None,
            shape,
            bounds,
        }
    }

    fn update_pos(x: &mut Twips, y: &mut Twips, record: &swf::ShapeRecord) {
        use swf::ShapeRecord;
        match record {
            ShapeRecord::StraightEdge { delta } => {
                *x += delta.dx;
                *y += delta.dy;
            }
            ShapeRecord::CurvedEdge {
                control_delta,
                anchor_delta,
            } => {
                *x += control_delta.dx + anchor_delta.dx;
                *y += control_delta.dy + anchor_delta.dy;
            }
            ShapeRecord::StyleChange(style_change) => {
                if let Some(move_to) = &style_change.move_to {
                    *x = move_to.x;
                    *y = move_to.y;
                }
            }
        }
    }

    pub fn set_ratio(&mut self, ratio: u16) {
        self.ratio = ratio;
    }
}

impl TDisplayObject for MorphShape {
    fn base(&self) -> &DisplayObjectBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut DisplayObjectBase {
        &mut self.base
    }

    fn movie(&self) -> Arc<SwfMovie> {
        self.movie.clone()
    }

    fn self_bounds(&mut self, context: &mut crate::RenderContext) -> Rectangle<Twips> {
        self.get_frame(self.ratio, context.morph_shape_cache)
            .bounds
            .clone()
    }

    fn id(&self) -> CharacterId {
        self.id
    }

    fn render_self(&mut self, context: &mut crate::RenderContext, blend_mode: swf::BlendMode) {
        let handle = self.get_shape(self.ratio, context);
        context.render_shape(
            handle,
            context.transform_stack.transform(),
            blend_mode.into(),
        );
    }

    fn as_morph_shape(&mut self) -> Option<&mut MorphShape> {
        Some(self)
    }
}

impl From<MorphShape> for DisplayObject {
    fn from(value: MorphShape) -> Self {
        Self::MorphShape(value)
    }
}

// Interpolation functions
// These interpolate between two SWF shape structures.
// a + b should = 1.0

fn lerp_color(start: &Color, end: &Color, a: f32, b: f32) -> Color {
    // f32 -> u8 cast is defined to saturate for out of bounds values,
    // so we don't have to worry about clamping.
    Color {
        r: (a * f32::from(start.r) + b * f32::from(end.r)) as u8,
        g: (a * f32::from(start.g) + b * f32::from(end.g)) as u8,
        b: (a * f32::from(start.b) + b * f32::from(end.b)) as u8,
        a: (a * f32::from(start.a) + b * f32::from(end.a)) as u8,
    }
}

fn lerp_twips(start: Twips, end: Twips, a: f32, b: f32) -> Twips {
    Twips::new((start.get() as f32 * a + end.get() as f32 * b).round() as i32)
}

fn lerp_point_twips(start: Point<Twips>, end: Point<Twips>, a: f32, b: f32) -> Point<Twips> {
    Point::new(
        lerp_twips(start.x, end.x, a, b),
        lerp_twips(start.y, end.y, a, b),
    )
}

fn lerp_fill(start: &swf::FillStyle, end: &swf::FillStyle, a: f32, b: f32) -> swf::FillStyle {
    use swf::FillStyle;
    match (start, end) {
        // Color-to-color
        (FillStyle::Color(start), FillStyle::Color(end)) => {
            FillStyle::Color(lerp_color(start, end, a, b))
        }

        // Bitmap-to-bitmap
        // ID should be the same.
        (
            FillStyle::Bitmap {
                id: start_id,
                matrix: start,
                is_smoothed,
                is_repeating,
            },
            FillStyle::Bitmap { matrix: end, .. },
        ) => FillStyle::Bitmap {
            id: *start_id,
            matrix: lerp_matrix(start, end, a, b),
            is_smoothed: *is_smoothed,
            is_repeating: *is_repeating,
        },

        // Linear-to-linear
        (FillStyle::LinearGradient(start), FillStyle::LinearGradient(end)) => {
            FillStyle::LinearGradient(lerp_gradient(start, end, a, b))
        }

        // Radial-to-radial
        (FillStyle::RadialGradient(start), FillStyle::RadialGradient(end)) => {
            FillStyle::RadialGradient(lerp_gradient(start, end, a, b))
        }

        // Focal gradients also interpolate focal point.
        (
            FillStyle::FocalGradient {
                gradient: start,
                focal_point: start_focal,
            },
            FillStyle::FocalGradient {
                gradient: end,
                focal_point: end_focal,
            },
        ) => FillStyle::FocalGradient {
            gradient: lerp_gradient(start, end, a, b),
            focal_point: *start_focal * Fixed8::from_f32(a) + *end_focal * Fixed8::from_f32(b),
        },

        // All other combinations should not occur, because SWF stores the start/end fill as the same type, always.
        // If you happened to make, say, a solid color-to-radial gradient tween in the IDE, this would get baked down into
        // a radial-to-radial gradient on export.
        _ => {
            warn!(
                "Unexpected morph shape fill style combination: {:#?}, {:#?}",
                start, end
            );
            start.clone()
        }
    }
}

fn lerp_edges(
    start_pen: Point<Twips>,
    end_pen: Point<Twips>,
    start: &swf::ShapeRecord,
    end: &swf::ShapeRecord,
    a: f32,
    b: f32,
) -> swf::ShapeRecord {
    use swf::ShapeRecord;
    let pen = lerp_point_twips(start_pen, end_pen, a, b);
    match (start, end) {
        (
            ShapeRecord::StraightEdge { delta: start_delta },
            ShapeRecord::StraightEdge { delta: end_delta },
        ) => {
            let start_anchor = start_pen + *start_delta;
            let end_anchor = end_pen + *end_delta;

            let anchor = lerp_point_twips(start_anchor, end_anchor, a, b);

            ShapeRecord::StraightEdge {
                delta: anchor - pen,
            }
        }

        (
            ShapeRecord::CurvedEdge {
                control_delta: start_control_delta,
                anchor_delta: start_anchor_delta,
            },
            ShapeRecord::CurvedEdge {
                control_delta: end_control_delta,
                anchor_delta: end_anchor_delta,
            },
        ) => {
            let start_control = start_pen + *start_control_delta;
            let start_anchor = start_control + *start_anchor_delta;

            let end_control = end_pen + *end_control_delta;
            let end_anchor = end_control + *end_anchor_delta;

            let control = lerp_point_twips(start_control, end_control, a, b);
            let anchor = lerp_point_twips(start_anchor, end_anchor, a, b);

            ShapeRecord::CurvedEdge {
                control_delta: control - pen,
                anchor_delta: anchor - control,
            }
        }

        (
            ShapeRecord::StraightEdge { delta: start_delta },
            ShapeRecord::CurvedEdge {
                control_delta: end_control_delta,
                anchor_delta: end_anchor_delta,
            },
        ) => {
            let start_control = start_pen + *start_delta / 2;
            let start_anchor = start_pen + *start_delta;

            let end_control = end_pen + *end_control_delta;
            let end_anchor = end_control + *end_anchor_delta;

            let control = lerp_point_twips(start_control, end_control, a, b);
            let anchor = lerp_point_twips(start_anchor, end_anchor, a, b);

            ShapeRecord::CurvedEdge {
                control_delta: control - pen,
                anchor_delta: anchor - control,
            }
        }

        (
            ShapeRecord::CurvedEdge {
                control_delta: start_control_delta,
                anchor_delta: start_anchor_delta,
            },
            ShapeRecord::StraightEdge { delta: end_delta },
        ) => {
            let start_control = start_pen + *start_control_delta;
            let start_anchor = start_control + *start_anchor_delta;

            let end_control = end_pen + *end_delta / 2;
            let end_anchor = end_pen + *end_delta;

            let control = lerp_point_twips(start_control, end_control, a, b);
            let anchor = lerp_point_twips(start_anchor, end_anchor, a, b);

            ShapeRecord::CurvedEdge {
                control_delta: control - pen,
                anchor_delta: anchor - control,
            }
        }
        _ => unreachable!("{:?} {:?}", start, end),
    }
}

fn lerp_matrix(start: &swf::Matrix, end: &swf::Matrix, a: f32, b: f32) -> swf::Matrix {
    // TODO: Lerping a matrix element-wise is geometrically wrong,
    // but I doubt Flash is decomposing the matrix into scale-rotate-translate?
    let af = Fixed16::from_f32(a);
    let bf = Fixed16::from_f32(b);
    swf::Matrix {
        a: start.a * af + end.a * bf,
        b: start.b * af + end.b * bf,
        c: start.c * af + end.c * bf,
        d: start.d * af + end.d * bf,
        tx: lerp_twips(start.tx, end.tx, a, b),
        ty: lerp_twips(start.ty, end.ty, a, b),
    }
}

fn lerp_gradient(start: &swf::Gradient, end: &swf::Gradient, a: f32, b: f32) -> swf::Gradient {
    use swf::{Gradient, GradientRecord};
    // Morph gradients are guaranteed to have the same number of records in the start/end gradient.
    debug_assert_eq!(start.records.len(), end.records.len());
    let records: Vec<GradientRecord> = start
        .records
        .iter()
        .zip(end.records.iter())
        .map(|(start, end)| swf::GradientRecord {
            ratio: (f32::from(start.ratio) * a + f32::from(end.ratio) * b) as u8,
            color: lerp_color(&start.color, &end.color, a, b),
        })
        .collect();

    Gradient {
        matrix: lerp_matrix(&start.matrix, &end.matrix, a, b),
        spread: start.spread,
        interpolation: start.interpolation,
        records,
    }
}
