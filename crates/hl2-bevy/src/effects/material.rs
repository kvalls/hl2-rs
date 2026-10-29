//! Material blending and opaque/masked effect prepass contract.
use super::EffectMaterial;
use bevy::{
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::{AlphaMode, Material},
    render::render_resource::{
        BlendComponent, BlendFactor, BlendOperation, BlendState, CompareFunction,
        RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};
impl Material for EffectMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/effects.wgsl".into()
    }
    fn prepass_fragment_shader() -> ShaderRef {
        "shaders/effects_depth.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        match self.kind {
            0 => AlphaMode::Opaque,
            1 => AlphaMode::Mask(0.5),
            _ => AlphaMode::Blend,
        }
    }
    fn specialize(
        _: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        if key.bind_group_data.ignore_z
            && let Some(depth) = descriptor.depth_stencil.as_mut()
        {
            depth.depth_compare = Some(CompareFunction::Always);
            depth.depth_write_enabled = Some(false);
        }
        if let Some(blend) = effect_blend(key.bind_group_data.kind) {
            for target in descriptor
                .fragment
                .as_mut()
                .into_iter()
                .flat_map(|f| &mut f.targets)
                .flatten()
            {
                target.blend = Some(blend);
            }
        }
        Ok(())
    }
}
fn effect_blend(kind: usize) -> Option<BlendState> {
    match kind {
        3 => Some(BlendState {
            color: BlendComponent {
                src_factor: BlendFactor::SrcAlpha,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            },
            alpha: BlendComponent::OVER,
        }),
        hl2_simulation::projectile_visuals::KIND_ALPHA_ADD => Some(BlendState {
            color: BlendComponent {
                src_factor: BlendFactor::One,
                dst_factor: BlendFactor::OneMinusSrcAlpha,
                operation: BlendOperation::Add,
            },
            alpha: BlendComponent::OVER,
        }),
        4 => Some(BlendState {
            color: BlendComponent {
                src_factor: BlendFactor::Dst,
                dst_factor: BlendFactor::Zero,
                operation: BlendOperation::Add,
            },
            alpha: BlendComponent {
                src_factor: BlendFactor::Zero,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            },
        }),
        _ => None,
    }
}
