//! The shadow textures: the installed blob picture for blob shadows, and the 1024-texel coverage
//! page the render-to-texture silhouettes are drawn into (fragments handed out by [`Allocator`]; one
//! page texel is the blur's offset).
use super::{
    BLOB_MATERIAL,
    material::{ShadowMaterial, ShadowParameters},
};
use crate::assets::LoadedMap;
use bevy::{
    asset::{Assets, Handle, RenderAssetUsages},
    image::{Image, ImageSampler},
    math::{Vec2, Vec4},
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use glam::UVec2;
use hl2_simulation::shadows::{
    allocator::{Allocator, PAGE},
    frame::Frame,
    silhouette,
};
use modkit_core::{Surface, Vertex};
use source_assets::vtf;
use std::{collections::BTreeMap, sync::Arc};

/// Where a caster vertex lies in the world.
pub(super) type Place<'a> = &'a dyn Fn(&Vertex) -> glam::Vec3;

pub(super) struct Textures {
    picture: Option<Arc<vtf::GpuImage>>,
    /// The blob shadows' material, made with the first shadow.
    pub blob: Option<Handle<ShadowMaterial>>,
    pub page: Allocator,
    pub(super) atlas_image: Option<Handle<Image>>,
    pub(super) batch: Option<super::gpu::Batch>,
    pub(super) submitted: Option<Arc<std::sync::atomic::AtomicU64>>,
    /// The render-to-texture shadows' material, made with the first silhouette.
    pub atlas_material: Option<Handle<ShadowMaterial>>,
    /// Base texture and opacity of the translucent and alpha-tested materials: their silhouettes
    /// take the texture's alpha; every other material covers fully.
    alphas: BTreeMap<String, (Arc<vtf::GpuImage>, f32)>,
    cell: Vec<f32>,
    /// A surface's vertices in cell texels, while its silhouette is drawn.
    points: Vec<glam::Vec2>,
    /// A silhouette was drawn since the atlas material was last refreshed.
    dirty: bool,
    color: Vec4,
}

impl Textures {
    pub fn new(loaded: &LoadedMap, color: [f32; 3]) -> Self {
        Self {
            picture: loaded
                .materials
                .get(BLOB_MATERIAL)
                .and_then(|m| m.base.clone()),
            blob: None,
            page: Allocator::new(),
            atlas_image: None,
            batch: None,
            submitted: None,
            atlas_material: None,
            alphas: loaded
                .materials
                .iter()
                .filter(|(_, m)| m.translucent || m.additive || m.alpha_cutoff.is_some())
                .filter_map(|(name, m)| Some((name.clone(), (m.base.clone()?, m.opacity))))
                .collect(),
            cell: Vec::new(),
            points: Vec::new(),
            dirty: false,
            color: Vec4::new(color[0], color[1], color[2], 1.),
        }
    }

    #[cfg(test)]
    pub(super) fn batch_only() -> Self {
        Self {
            picture: None,
            blob: None,
            page: Allocator::new(),
            atlas_image: None,
            batch: Some(super::gpu::Batch::default()),
            submitted: None,
            atlas_material: None,
            alphas: BTreeMap::new(),
            cell: Vec::new(),
            points: Vec::new(),
            dirty: false,
            color: Vec4::ONE,
        }
    }

    /// Hands silhouette drawing to the GPU pass; called before the atlas exists.
    pub(super) fn use_gpu(&mut self) {
        if self.batch.is_none() {
            self.batch = Some(super::gpu::Batch::default());
            self.submitted = Some(Arc::default());
        }
    }

    /// Makes the blob and atlas materials on first use.
    pub fn prepare(&mut self, materials: &mut Assets<ShadowMaterial>, images: &mut Assets<Image>) {
        if self.blob.is_none()
            && let Some(picture) = &self.picture
        {
            let texture = images.add(crate::rendering::gpu_image_filtered(
                picture.as_ref().clone(),
                false,
                false,
                hl2_ui::config::Filtering::Trilinear,
            ));
            let size = Vec2::new(f32::from(picture.width), f32::from(picture.height));
            self.blob = Some(materials.add(self.material(texture, size, Vec4::W)));
        }
        if self.atlas_material.is_none() {
            let mut image = Image::new(
                Extent3d {
                    width: PAGE,
                    height: PAGE,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                vec![0; (PAGE * PAGE) as usize],
                TextureFormat::R8Unorm,
                RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
            );
            if self.batch.is_some() {
                image.texture_descriptor.usage |=
                    bevy::render::render_resource::TextureUsages::RENDER_ATTACHMENT;
            }
            image.sampler = ImageSampler::Descriptor(crate::rendering::sampler_descriptor(
                false,
                hl2_ui::config::Filtering::Bilinear,
            ));
            let texture = images.add(image);
            self.atlas_image = Some(texture.clone());
            let material = self.material(texture, Vec2::splat(PAGE as f32), Vec4::X);
            self.atlas_material = Some(materials.add(material));
        }
    }

    /// Marks the atlas material changed after silhouettes were drawn: its bind group holds the
    /// atlas texture as it was when the material was prepared.
    pub fn refresh(&mut self, materials: &mut Assets<ShadowMaterial>) {
        if std::mem::take(&mut self.dirty)
            && let Some(handle) = &self.atlas_material
        {
            materials.get_mut(handle);
        }
    }

    fn material(&self, texture: Handle<Image>, size: Vec2, channel: Vec4) -> ShadowMaterial {
        ShadowMaterial {
            texture,
            parameters: ShadowParameters {
                color: self.color,
                jitter: (Vec2::ONE / size).extend(0.).extend(0.),
                channel,
            },
            verts: super::draw::SHADOW_VERTS,
        }
    }

    /// Draws a caster's silhouette into its cell (`n` texels at `at`): its `surfaces`, each vertex
    /// placed in the world by `place`, seen through `frame`.
    pub fn draw(
        &mut self,
        images: &mut Assets<Image>,
        (n, at): (u32, UVec2),
        frame: &Frame,
        (surfaces, place): (&[Surface], Place),
    ) {
        if let Some(batch) = &mut self.batch {
            batch.cell(n, at, frame, (surfaces, place), &self.alphas);
            return;
        }
        silhouette::clear(&mut self.cell, n);
        for surface in surfaces {
            let decoded = self
                .alphas
                .get(&surface.material)
                .map(|(picture, opacity)| {
                    let rgba = match picture.pixel_format {
                        vtf::GpuFormat::Rgba8 => {
                            std::borrow::Cow::Borrowed(picture.data.as_slice())
                        }
                        _ => std::borrow::Cow::Owned(picture.to_rgba8()),
                    };
                    (picture.width, picture.height, rgba, *opacity)
                });
            let alpha = decoded.as_ref().map(|(w, h, rgba, opacity)| {
                move |uv: glam::Vec2| sample_rgba_alpha(*w, *h, rgba, uv) * *opacity
            });
            // Each vertex is placed (skinned) and projected once, not once per triangle corner.
            self.points.clear();
            let project = |v: &Vertex| silhouette::project(frame, n, place(v));
            self.points.extend(surface.vertices.iter().map(project));
            let points = &self.points;
            let corner = |i: u32| Some((*points.get(i as usize)?, surface.vertices[i as usize].uv));
            for t in surface.indices.as_chunks::<3>().0 {
                let [Some(a), Some(b), Some(c)] = t.map(corner) else {
                    continue;
                };
                let triangle = ([a.0, b.0, c.0], [a.1, b.1, c.1]);
                let alpha = alpha.as_ref().map(|f| f as &dyn Fn(glam::Vec2) -> f32);
                silhouette::draw_projected(&mut self.cell, n, triangle, alpha);
            }
        }
        let Some(mut image) = self.atlas_image.as_ref().and_then(|h| images.get_mut(h)) else {
            return;
        };
        let Some(data) = image.data.as_mut() else {
            return;
        };
        self.dirty = true;
        for (y, row) in self.cell.chunks_exact(n as usize).enumerate() {
            let start = ((at.y + y as u32) * PAGE + at.x) as usize;
            for (texel, coverage) in data[start..start + n as usize].iter_mut().zip(row) {
                *texel = (coverage * 255.).round() as u8;
            }
        }
    }
}

fn sample_rgba_alpha(width: u16, height: u16, rgba: &[u8], uv: glam::Vec2) -> f32 {
    let (w, h) = (u32::from(width).max(1), u32::from(height).max(1));
    let x = (uv.x.rem_euclid(1.) * w as f32) as u32 % w;
    let y = (uv.y.rem_euclid(1.) * h as f32) as u32 % h;
    rgba.get(((y * w + x) * 4 + 3) as usize)
        .map_or(1., |a| f32::from(*a) / 255.)
}

/// A texture's alpha at a repeating texture coordinate (nearest texel).
#[cfg(test)]
pub(super) fn texel_alpha(picture: &vtf::GpuImage, uv: glam::Vec2) -> f32 {
    let rgba = match picture.pixel_format {
        vtf::GpuFormat::Rgba8 => std::borrow::Cow::Borrowed(picture.data.as_slice()),
        _ => std::borrow::Cow::Owned(picture.to_rgba8()),
    };
    sample_rgba_alpha(picture.width, picture.height, &rgba, uv)
}
