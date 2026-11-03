//! The receiver side of a projected shadow (Source SDK `materialsystem/stdshaders/shadow.cpp`,
//! `shadow_ps2x.fxc`): the coverage, five taps one texel apart averaged, less the vertex fade,
//! takes the surface toward the shadow colour by a multiply blend (`ZERO, SRC_COLOR`); depth is
//! tested, not written; both faces draw. `shadow.wgsl`.
use bevy::{
    asset::{Asset, Handle},
    image::Image,
    material::AlphaMode,
    math::Vec4,
    mesh::MeshVertexBufferLayoutRef,
    pbr::{Material, MaterialPipeline, MaterialPipelineKey},
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState,
        RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};

/// The shader's constants.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct ShadowParameters {
    /// The shadow colour, linear (`SetPixelShaderConstantGammaToLinear`).
    pub color: Vec4,
    /// One texel of the shadow texture (x, y): the blur taps' offset.
    pub jitter: Vec4,
    /// Which channel holds the coverage: alpha for a texture, red for the coverage atlas.
    pub channel: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct ShadowMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Handle<Image>,
    #[uniform(2)]
    pub parameters: ShadowParameters,
}

impl Material for ShadowMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/shadow.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn specialize(
        _: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        let multiply = BlendState {
            color: BlendComponent {
                src_factor: BlendFactor::Zero,
                dst_factor: BlendFactor::Src,
                operation: BlendOperation::Add,
            },
            alpha: BlendComponent {
                src_factor: BlendFactor::Zero,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            },
        };
        for target in descriptor
            .fragment
            .as_mut()
            .into_iter()
            .flat_map(|f| &mut f.targets)
            .flatten()
        {
            target.blend = Some(multiply);
        }
        Ok(())
    }
}
