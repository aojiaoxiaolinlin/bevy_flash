use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Default, Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
}

impl Vertex {
    pub fn new(position: [f32; 3]) -> Self {
        Self { position }
    }
}

#[repr(C)]
#[derive(Default, Clone, Copy, Pod, Zeroable)]
pub struct VertexColor {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

impl VertexColor {
    pub fn new(position: [f32; 3], color: [f32; 4]) -> Self {
        Self { position, color }
    }
}
