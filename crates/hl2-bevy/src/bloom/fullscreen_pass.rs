//! What the fullscreen passes that rewrite a view's main texture share (the motion blur, the
//! intro composite): their linear sampler, one pipeline per target format and sample count,
//! and the draw into the other main texture.
use bevy::{
    core_pipeline::FullscreenShader,
    prelude::{Handle, Shader},
    render::{
        render_resource::{
            BindGroup, BindGroupLayoutDescriptor, CachedRenderPipelineId, ColorTargetState,
            ColorWrites, FilterMode, FragmentState, LoadOp, MultisampleState, Operations,
            PipelineCache, RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline,
            RenderPipelineDescriptor, Sampler, SamplerDescriptor, StoreOp, TextureFormat,
            TextureView, TextureViewId,
        },
        renderer::{RenderContext, RenderDevice},
        view::ViewTarget,
    },
};

/// The bind group made for the main texture `view` (a pass binds both main textures, since
/// passes before it may swap them).
pub(crate) fn group_of<'a>(
    groups: &'a [(TextureViewId, BindGroup); 2],
    view: &TextureView,
) -> Option<&'a BindGroup> {
    groups
        .iter()
        .find_map(|(id, group)| (*id == view.id()).then_some(group))
}

pub(crate) fn linear_sampler(device: &RenderDevice) -> Sampler {
    device.create_sampler(&SamplerDescriptor {
        min_filter: FilterMode::Linear,
        mag_filter: FilterMode::Linear,
        ..Default::default()
    })
}

/// A fullscreen pass's shader, entry point and bind group layout.
pub(crate) struct Fullscreen<'a> {
    pub label: &'static str,
    pub entry: &'static str,
    pub layout: &'a BindGroupLayoutDescriptor,
    pub vertex: &'a FullscreenShader,
    pub shader: &'a Handle<Shader>,
}

/// The pass's pipeline for a target format and sample count (the view's attachment).
pub(crate) fn queue_pipeline(
    cache: &PipelineCache,
    pass: &Fullscreen,
    (format, samples): (TextureFormat, u32),
) -> CachedRenderPipelineId {
    cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some(pass.label.into()),
        layout: vec![pass.layout.clone()],
        vertex: pass.vertex.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: pass.shader.clone(),
            shader_defs: Vec::new(),
            entry_point: Some(pass.entry.into()),
            targets: vec![Some(ColorTargetState {
                format,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
        }),
        multisample: MultisampleState {
            count: samples,
            ..Default::default()
        },
        ..Default::default()
    })
}

/// Draws one fullscreen triangle. The frame stays readable in the texture it was resolved to;
/// the attachment now resolves into the other one, so no copy of the frame is needed. The
/// triangle overwrites every sample, so loading the old image would only cost bandwidth; the
/// multisampled image must be stored: the viewmodel camera continues from it.
pub(crate) fn draw(
    ctx: &mut RenderContext,
    label: &'static str,
    target: &ViewTarget,
    (pipeline, group): (&RenderPipeline, &BindGroup),
) {
    let destination = target.post_process_write().destination;
    let (view, resolve_target) = match target.sampled_main_texture_view() {
        Some(sampled) => (sampled, Some(destination)),
        None => (destination, None),
    };
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: resolve_target.map(|view| &**view),
            ops: Operations {
                load: LoadOp::Clear(Default::default()),
                store: StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_render_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.draw(0..3, 0..1);
}
