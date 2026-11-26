use super::batch::Batch;
use bevy::{
    prelude::default,
    render::{
        render_resource::{
            BindGroup, BindGroupEntries, BindGroupLayout, BlendComponent, BlendFactor,
            BlendOperation, BlendState, Buffer, BufferDescriptor, BufferUsages, ColorTargetState,
            ColorWrites, CommandEncoder, Extent3d, IndexFormat, LoadOp, Operations, Origin3d,
            RawFragmentState, RawRenderPipelineDescriptor, RawVertexBufferLayout, RawVertexState,
            RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline,
            ShaderModuleDescriptor, ShaderSource, StoreOp, TexelCopyBufferInfo,
            TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureAspect, TextureDescriptor,
            TextureDimension, TextureFormat, TextureUsages, TextureView, VertexAttribute,
            VertexFormat, VertexStepMode,
        },
        renderer::{RenderDevice, RenderQueue},
    },
};
use source_assets::vtf;
use std::{collections::HashMap, sync::Arc};

pub(super) struct Raster {
    pipeline: RenderPipeline,
    layout: BindGroupLayout,
    vertices: Option<Buffer>,
    indices: Option<Buffer>,
    zeros: Buffer,
    // Keep the source owner alive for as long as its pointer is a cache key.
    groups: HashMap<usize, (Option<Arc<vtf::GpuImage>>, BindGroup)>,
}

impl Raster {
    pub(super) fn new(device: &RenderDevice) -> Self {
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("shadow silhouette"),
            source: ShaderSource::Wgsl(
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/assets/shaders/shadow_silhouette.wgsl"
                ))
                .into(),
            ),
        });
        let add = BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        };
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("shadow silhouette"),
            layout: None,
            vertex: RawVertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: default(),
                buffers: &[RawVertexBufferLayout {
                    array_stride: 20,
                    step_mode: VertexStepMode::Vertex,
                    attributes: &[
                        VertexAttribute {
                            format: VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        VertexAttribute {
                            format: VertexFormat::Float32x2,
                            offset: 8,
                            shader_location: 1,
                        },
                        VertexAttribute {
                            format: VertexFormat::Float32,
                            offset: 16,
                            shader_location: 2,
                        },
                    ],
                }],
            },
            fragment: Some(RawFragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: default(),
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::R8Unorm,
                    blend: Some(BlendState {
                        color: add,
                        alpha: add,
                    }),
                    write_mask: ColorWrites::RED,
                })],
            }),
            primitive: default(),
            depth_stencil: None,
            multisample: default(),
            multiview_mask: None,
            cache: None,
        });
        let layout = pipeline.get_bind_group_layout(0).into();
        let zeros = device.create_buffer(&BufferDescriptor {
            label: Some("shadow cell zeroes"),
            size: 256 * 256,
            usage: BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            zeros,
            vertices: None,
            indices: None,
            groups: HashMap::new(),
        }
    }

    pub(super) fn reset_materials(&mut self) {
        self.groups.clear();
    }

    #[cfg(test)]
    pub(super) fn draw(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        atlas: &Texture,
        view: &TextureView,
        batch: &Batch,
    ) {
        let mut enc = device.create_command_encoder(&default());
        self.encode(device, queue, &mut enc, atlas, view, batch);
        queue.submit([enc.finish()]);
    }

    pub(super) fn encode(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        enc: &mut CommandEncoder,
        atlas: &Texture,
        view: &TextureView,
        batch: &Batch,
    ) {
        for cell in &batch.cells {
            enc.copy_buffer_to_texture(
                TexelCopyBufferInfo {
                    buffer: &self.zeros,
                    layout: TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: None,
                    },
                },
                TexelCopyTextureInfo {
                    texture: atlas,
                    mip_level: 0,
                    origin: Origin3d {
                        x: cell.at.x,
                        y: cell.at.y,
                        z: 0,
                    },
                    aspect: TextureAspect::All,
                },
                Extent3d {
                    width: cell.n,
                    height: cell.n,
                    depth_or_array_layers: 1,
                },
            );
        }
        if !batch.indices.is_empty() {
            for draw in &batch.draws {
                self.groups
                    .entry(key(draw.picture.as_ref()))
                    .or_insert_with(|| {
                        let group =
                            alpha_group(device, queue, &self.layout, draw.picture.as_deref());
                        (draw.picture.clone(), group)
                    });
            }
            let verts = upload(
                &mut self.vertices,
                device,
                queue,
                BufferUsages::VERTEX,
                &batch.vertices,
            );
            let indices = upload(
                &mut self.indices,
                device,
                queue,
                BufferUsages::INDEX,
                &batch.indices,
            );
            let mut pass = enc.begin_render_pass(&RenderPassDescriptor {
                label: Some("dirty shadow cells"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    },
                })],
                ..default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, *verts.slice(..));
            pass.set_index_buffer(*indices.slice(..), IndexFormat::Uint32);
            for cell in &batch.cells {
                pass.set_viewport(
                    cell.at.x as f32,
                    cell.at.y as f32,
                    cell.n as f32,
                    cell.n as f32,
                    0.,
                    1.,
                );
                pass.set_scissor_rect(cell.at.x, cell.at.y, cell.n, cell.n);
                for draw in &batch.draws[cell.draws.clone()] {
                    let group = &self.groups[&key(draw.picture.as_ref())].1;
                    pass.set_bind_group(0, &**group, &[]);
                    pass.draw_indexed(draw.indices.clone(), 0, 0..1);
                }
            }
        }
    }
}

fn key(image: Option<&Arc<vtf::GpuImage>>) -> usize {
    image.map_or(0, |p| Arc::as_ptr(p) as usize)
}

fn upload<'a>(
    slot: &'a mut Option<Buffer>,
    device: &RenderDevice,
    queue: &RenderQueue,
    usage: BufferUsages,
    bytes: &[u8],
) -> &'a Buffer {
    let len = bytes.len() as u64;
    if slot.as_ref().is_none_or(|b| b.size() < len) {
        *slot = Some(device.create_buffer(&BufferDescriptor {
            label: Some("shadow geometry"),
            size: len.next_power_of_two(),
            usage: usage | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }
    let buffer = slot.as_ref().expect("allocated shadow buffer");
    queue.write_buffer(buffer, 0, bytes);
    buffer
}

fn alpha_group(
    device: &RenderDevice,
    queue: &RenderQueue,
    layout: &BindGroupLayout,
    image: Option<&vtf::GpuImage>,
) -> BindGroup {
    let (w, h, format, bytes_per_row, data) =
        image.map_or((1, 1, TextureFormat::Rgba8Unorm, 4, &[255u8; 4][..]), |i| {
            let w = u32::from(i.width);
            let h = u32::from(i.height);
            let (format, bytes_per_row) = match i.pixel_format {
                vtf::GpuFormat::Rgba8 => (TextureFormat::Rgba8Unorm, 4 * w),
                vtf::GpuFormat::Bc1 => (TextureFormat::Bc1RgbaUnorm, 8 * w.div_ceil(4)),
                vtf::GpuFormat::Bc2 => (TextureFormat::Bc2RgbaUnorm, 16 * w.div_ceil(4)),
                vtf::GpuFormat::Bc3 => (TextureFormat::Bc3RgbaUnorm, 16 * w.div_ceil(4)),
            };
            (w, h, format, bytes_per_row, i.data.as_slice())
        });
    let size = Extent3d {
        width: w,
        height: h,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&TextureDescriptor {
        label: Some("silhouette alpha"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        data,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(bytes_per_row),
            rows_per_image: None,
        },
        size,
    );
    let view = texture.create_view(&default());
    device.create_bind_group(
        Some("silhouette alpha"),
        layout,
        &BindGroupEntries::single(&view),
    )
}
