use bevy::{
    asset::{Asset, Handle},
    prelude::{Image, Mesh},
    reflect::TypePath,
    render::render_resource::{AsBindGroup, ShaderType},
    sprite::Material2d,
};
use glam::Mat4;

#[derive(AsBindGroup, TypePath, Asset, Debug, Clone)]
pub struct GradientMaterial {
    #[uniform(0)]
    pub gradient: Gradient,
    #[uniform(3)]
    pub texture_transform: Mat4,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Option<Handle<Image>>,
}

impl Material2d for GradientMaterial {
    fn vertex_shader() -> bevy::render::render_resource::ShaderRef {
        "shaders/gradient.wgsl".into()
    }
    fn fragment_shader() -> bevy::render::render_resource::ShaderRef {
        "shaders/gradient.wgsl".into()
    }
    // fn specialize(
    //     descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
    //     layout: &bevy::render::mesh::MeshVertexBufferLayoutRef,
    //     _key: bevy::sprite::Material2dKey<Self>,
    // ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
    //     let vertex_layout = layout
    //         .0
    //         .get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?;
    //     descriptor.vertex.buffers = vec![vertex_layout];
    //     Ok(())
    // }
}

#[derive(Debug, Clone, Default, ShaderType)]
pub struct Gradient {
    pub focal_point: f32,
    pub interpolation: i32,
    pub shape: i32,
    pub repeat: i32,
}

// impl ShaderType for Gradient {
//     type ExtraMetadata = ();

//     const METADATA: encase::private::Metadata<Self::ExtraMetadata> = {
//         let size =
//             encase::private::SizeValue::from(<f32 as encase::private::ShaderSize>::SHADER_SIZE)
//                 .mul(4);
//         let alignment = encase::private::AlignmentValue::from_next_power_of_two_size(size);

//         encase::private::Metadata {
//             alignment,
//             has_uniform_min_alignment: false,
//             is_pod: true,
//             min_size: size,
//             extra: (),
//         }
//     };

//     const UNIFORM_COMPAT_ASSERT: fn() = || {};
// }

// impl encase::private::WriteInto for Gradient {
//     fn write_into<B: encase::private::BufferMut>(&self, writer: &mut encase::internal::Writer<B>) {
//         for el in &[self.focal_point, self.interpolation] {
//             encase::private::WriteInto::write_into(el, writer)
//         }
//         for el in &[self.shape, self.repeat] {
//             encase::private::WriteInto::write_into(el, writer)
//         }
//     }
// }

// impl encase::private::ReadFrom for Gradient {
//     fn read_from<B>(&mut self, reader: &mut encase::internal::Reader<B>)
//     where
//         B: encase::internal::BufferRef,
//     {
//         let mut buffer = [0.0f32; 2];
//         for el in &mut buffer {
//             encase::private::ReadFrom::read_from(el, reader)
//         }
//         let mut buffer_2 = [0i32; 2];
//         for el in &mut buffer_2 {
//             encase::private::ReadFrom::read_from(el, reader)
//         }
//         *self = Gradient {
//             focal_point: buffer[0],
//             interpolation: buffer[1],
//             shape: buffer_2[0],
//             repeat: buffer_2[1],
//         };
//     }
// }

// impl encase::private::CreateFrom for Gradient {
//     fn create_from<B: encase::internal::BufferRef>(
//         reader: &mut encase::internal::Reader<B>,
//     ) -> Self {
//         let focal_point = encase::private::CreateFrom::create_from(reader);
//         let interpolation = encase::private::CreateFrom::create_from(reader);
//         let shape = encase::private::CreateFrom::create_from(reader);
//         let repeat = encase::private::CreateFrom::create_from(reader);
//         Gradient {
//             focal_point,
//             interpolation,
//             shape,
//             repeat,
//         }
//     }
// }

// impl encase::ShaderSize for Gradient {}
