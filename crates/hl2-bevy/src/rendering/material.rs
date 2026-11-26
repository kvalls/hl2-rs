//! Textures and Source materials for draws: fallback images, filtered samplers, VTF mip chains and the
//! `SourceMaterial` a material definition makes.
use super::{MaterialData, ModelLighting, SourceMaterial};
use bevy::pbr::MeshPipelineKey;
use bevy::render::render_resource::Face;
use bevy::{
    asset::{Assets, Handle, RenderAssetUsages},
    image::{Image, ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    material::AlphaMode,
    math::{Mat3, Vec3, Vec4},
    render::render_resource::{
        Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
    },
};

/// Fallback for materials without an envmap (never sampled; envmap_tint.w is 0). Held by a
/// strong handle like the white fallback: a UUID handle does not keep the image alive, and
/// materials re-prepared later (per-frame model lighting) would then fail to bind.
pub(crate) fn black_cube() -> Image {
    let cube = source_assets::vtf::Cube {
        size: 1,
        hdr: false,
        faces: std::array::from_fn(|_| vec![0; 4]),
        mips: Vec::new(),
    };
    cube_image_filtered(&cube, hl2_ui::config::Filtering::Trilinear)
}

/// A six-layer cube texture: RGBA16F linear halves (HDR) or sRGB-encoded RGBA8 (LDR).
pub(super) fn cube_image_filtered(
    cube: &source_assets::vtf::Cube,
    filtering: hl2_ui::config::Filtering,
) -> Image {
    let mut image = Image::new_uninit(
        Extent3d {
            width: cube.size.into(),
            height: cube.size.into(),
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        if cube.hdr {
            TextureFormat::Rgba16Float
        } else {
            TextureFormat::Rgba8UnormSrgb
        },
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(cube.layers());
    image.texture_descriptor.mip_level_count = cube.levels();
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..Default::default()
    });
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: if filtering.is_trilinear() {
            ImageFilterMode::Linear
        } else {
            ImageFilterMode::Nearest
        },
        anisotropy_clamp: filtering.aniso(),
        ..Default::default()
    });
    image
}

pub(crate) fn sampler_descriptor(
    repeat: bool,
    filtering: hl2_ui::config::Filtering,
) -> ImageSamplerDescriptor {
    let mode = if repeat {
        ImageAddressMode::Repeat
    } else {
        ImageAddressMode::ClampToEdge
    };
    let mipmap_filter = if filtering.is_trilinear() {
        ImageFilterMode::Linear
    } else {
        ImageFilterMode::Nearest
    };
    ImageSamplerDescriptor {
        address_mode_u: mode,
        address_mode_v: mode,
        address_mode_w: mode,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter,
        anisotropy_clamp: filtering.aniso(),
        ..Default::default()
    }
}

pub(crate) fn gpu_image_filtered(
    texture: source_assets::vtf::GpuImage,
    repeat: bool,
    srgb: bool,
    filtering: hl2_ui::config::Filtering,
) -> Image {
    use source_assets::vtf::GpuFormat;
    let format = match (texture.pixel_format, srgb) {
        (GpuFormat::Rgba8, false) => TextureFormat::Rgba8Unorm,
        (GpuFormat::Rgba8, true) => TextureFormat::Rgba8UnormSrgb,
        (GpuFormat::Bc1, false) => TextureFormat::Bc1RgbaUnorm,
        (GpuFormat::Bc1, true) => TextureFormat::Bc1RgbaUnormSrgb,
        (GpuFormat::Bc2, false) => TextureFormat::Bc2RgbaUnorm,
        (GpuFormat::Bc2, true) => TextureFormat::Bc2RgbaUnormSrgb,
        (GpuFormat::Bc3, false) => TextureFormat::Bc3RgbaUnorm,
        (GpuFormat::Bc3, true) => TextureFormat::Bc3RgbaUnormSrgb,
    };
    let mut image = Image::new(
        Extent3d {
            width: texture.width.into(),
            height: texture.height.into(),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        texture.data,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(sampler_descriptor(repeat, filtering));
    image
}

pub(super) fn image_filtered(
    width: u16,
    height: u16,
    rgba: Vec<u8>,
    repeat: bool,
    filtering: hl2_ui::config::Filtering,
) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: width.into(),
            height: height.into(),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(sampler_descriptor(repeat, filtering));
    image
}

/// Samples these textures without filtering when a video policy's render budget asks.
pub(super) fn unfilter<'a>(
    images: &mut Assets<Image>,
    handles: impl Iterator<Item = &'a Handle<Image>>,
    nearest: bool,
) {
    for handle in handles.filter(|_| nearest) {
        let Some(mut image) = images.get_mut(handle) else {
            continue;
        };
        if let ImageSampler::Descriptor(descriptor) = &mut image.sampler {
            descriptor.mag_filter = ImageFilterMode::Nearest;
            descriptor.min_filter = ImageFilterMode::Nearest;
            descriptor.mipmap_filter = ImageFilterMode::Nearest;
            descriptor.anisotropy_clamp = 1;
        }
    }
}

/// A repeating sRGB texture with its authored VTF mip chain (largest first), sampled
/// according to the configured filtering mode.
pub(super) fn mip_image_filtered(
    chain: &[source_assets::vtf::Image],
    filtering: hl2_ui::config::Filtering,
) -> Image {
    let top = &chain[0];
    let total_bytes: usize = chain.iter().map(|mip| mip.rgba.len()).sum();
    let mut data = Vec::with_capacity(total_bytes);
    for mip in chain {
        data.extend_from_slice(&mip.rgba);
    }
    let mut result = Image::new_uninit(
        Extent3d {
            width: top.width.into(),
            height: top.height.into(),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    result.data = Some(data);
    result.texture_descriptor.mip_level_count = chain.len() as u32;
    result.sampler = ImageSampler::Descriptor(sampler_descriptor(true, filtering));
    result
}

#[allow(dead_code)]
pub(super) fn mip_image(chain: &[source_assets::vtf::Image]) -> Image {
    mip_image_filtered(chain, hl2_ui::config::Filtering::Trilinear)
}

pub(crate) fn image(width: u16, height: u16, rgba: Vec<u8>, repeat: bool) -> Image {
    image_filtered(
        width,
        height,
        rgba,
        repeat,
        hl2_ui::config::Filtering::Trilinear,
    )
}

/// The white, black cube and flashlight cookie textures every material binds.
pub(crate) type Fallbacks<'a> = (&'a Handle<Image>, &'a Handle<Image>, &'a Handle<Image>);

/// `parameters.w` of a WorldVertexTransition material: lerp the base toward the second texture
/// slot (`iris`) by vertex alpha.
pub(crate) const BLEND_SECOND_BASE: f32 = 2.;
/// `parameters.z` of a DecalModulate material: output twice the texture for the multiply blend.
pub(crate) const MODULATE_OUTPUT: f32 = 2.;

pub(crate) fn make_material(
    definition: &MaterialData,
    base: Handle<Image>,
    lightmap: Handle<Image>,
    iris: Option<Handle<Image>>,
    (white, black_cube, flashlight): Fallbacks,
) -> SourceMaterial {
    let alpha = alpha_mode(definition);
    let unlit = definition.unlit
        || definition.camera
        || definition.two_texture.is_some()
        || matches!(alpha, AlphaMode::Add | AlphaMode::Multiply);
    SourceMaterial {
        tint: Vec4::new(
            definition.tint[0],
            definition.tint[1],
            definition.tint[2],
            definition.opacity,
        ),
        parameters: Vec4::new(
            definition
                .alpha_cutoff
                .unwrap_or(if matches!(alpha, AlphaMode::Opaque) {
                    0.
                } else {
                    0.001
                }),
            f32::from(matches!(alpha, AlphaMode::Opaque)),
            // z: 1 additive output, 2 DecalModulate's mod2x.
            match alpha {
                AlphaMode::Add => 1.,
                AlphaMode::Multiply => MODULATE_OUTPUT,
                _ => 0.,
            },
            if definition.camera {
                if definition.camera_vertex_color {
                    -2.
                } else {
                    -1.
                }
            } else if definition.two_texture.is_some() {
                -3.
            } else if definition.base2.is_some() {
                BLEND_SECOND_BASE
            } else {
                f32::from(iris.is_some())
            },
        ),
        iris: iris.unwrap_or_else(|| white.clone()),
        secondary_uv: Mat3::IDENTITY,
        lighting: ModelLighting {
            params: Vec4::new(0., 0., 0., f32::from(unlit)),
            ..Default::default()
        },
        flashlight: flashlight.clone(),
        viewmodel_flashlight: Default::default(),
        eye_rows: crate::eyes::EYE_ROWS,
        material_rows: crate::material_slots::MATERIAL_ROWS,
        light_rows: crate::material_slots::LIGHT_ROWS,
        dynamic: bevy::math::Vec4::new(-1., 0., -1., 0.),
        envmap: black_cube.clone(),
        envmap_tint: Vec3::from_array(definition.envmap_tint).extend(0.),
        envmap_contrast: Vec3::from_array(definition.envmap_contrast)
            .extend(definition.fresnel_reflection),
        envmap_saturation: Vec3::from_array(definition.envmap_saturation).extend(
            f32::from(u8::from(definition.base_alpha_envmap_mask))
                + if definition.envmap_mask.is_none() && definition.envmap_mask_path.is_none() {
                    0.
                } else {
                    match (definition.envmap_mask_alpha, definition.envmap_bump) {
                        (true, true) => 12.,
                        (true, false) => 4.,
                        (false, true) => 8.,
                        (false, false) => 2.,
                    }
                },
        ),
        envmap_mask: white.clone(),
        self_illum: definition
            .self_illum
            .map_or(Vec4::ZERO, |tint| Vec3::from_array(tint).extend(1.)),
        base,
        lightmap: if definition.unlit {
            white.clone()
        } else {
            lightmap
        },
        alpha,
        two_sided: definition.two_sided,
    }
}

pub(super) fn alpha_mode(material: &MaterialData) -> AlphaMode {
    if material.modulate {
        // Bevy Multiply blends Dst x Src + Dst x (1 - SrcAlpha): opaque output is Dst x Src.
        AlphaMode::Multiply
    } else if material.additive {
        AlphaMode::Add
    } else if let Some(cutoff) = material.alpha_cutoff {
        AlphaMode::Mask(cutoff)
    } else if material.translucent || material.opacity < 1. {
        AlphaMode::Blend
    } else {
        AlphaMode::Opaque
    }
}

/// The images every map's materials fall back to: white, a black cube, the flashlight cookie and the
/// magenta-and-grey missing texture.
pub(crate) fn fallback_images(
    images: &mut bevy::asset::Assets<Image>,
    cookie: Option<&source_assets::vtf::Image>,
    filtering: hl2_ui::config::Filtering,
) -> (Handle<Image>, Handle<Image>, Handle<Image>, Handle<Image>) {
    let missing = vec![
        255, 0, 255, 255, 30, 30, 30, 255, 30, 30, 30, 255, 255, 0, 255, 255,
    ];
    (
        images.add(image_filtered(1, 1, vec![255; 4], false, filtering)),
        images.add(black_cube()),
        images.add(crate::flashlight::cookie_image(cookie)),
        images.add(image_filtered(2, 2, missing, true, filtering)),
    )
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct MaterialKey {
    two_sided: bool,
    may_discard: bool,
    model_lit: bool,
    envmap: bool,
}
impl From<&SourceMaterial> for MaterialKey {
    fn from(material: &SourceMaterial) -> Self {
        Self {
            two_sided: material.two_sided,
            may_discard: !matches!(material.alpha, AlphaMode::Opaque) || material.parameters.x > 0.,
            model_lit: material.lighting.params.x > 0.5
                || material.dynamic.z >= 0.
                || (material.parameters.w > 0.5 && material.parameters.w < 1.5),
            envmap: material.envmap_tint.w > 0.5,
        }
    }
}

impl bevy::pbr::Material for SourceMaterial {
    fn fragment_shader() -> bevy::shader::ShaderRef {
        "shaders/source.wgsl".into()
    }
    fn prepass_fragment_shader() -> bevy::shader::ShaderRef {
        "shaders/source_depth.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        self.alpha
    }
    fn specialize(
        _: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        // BSP render triangles are normalized before model append; vmdl already
        // supplies CCW model triangles. Both now use Bevy's CCW front face.
        descriptor.primitive.front_face = bevy::render::render_resource::FrontFace::Ccw;
        descriptor.primitive.cull_mode = faces_culled(
            descriptor.primitive.cull_mode,
            key.bind_group_data.two_sided,
            key.mesh_key,
        );
        if let Some(fragment) = &mut descriptor.fragment {
            if key.bind_group_data.may_discard {
                fragment.shader_defs.push("SOURCE_MAY_DISCARD".into());
            }
            if key.bind_group_data.model_lit {
                fragment.shader_defs.push("SOURCE_MODEL_LIT".into());
            }
            if key.bind_group_data.envmap {
                fragment.shader_defs.push("SOURCE_ENVMAP".into());
            }
        }
        Ok(())
    }
}

/// Bevy's prepass culls nothing unless the material says so. A motion prepass must match the
/// colour pass: back-face depth it alone writes hides the colour behind it and exposes the sky
/// through one-sided cutouts. Shadow casters share that pipeline, depth-only, and keep both faces.
fn faces_culled(current: Option<Face>, two_sided: bool, mesh_key: MeshPipelineKey) -> Option<Face> {
    if two_sided {
        None
    } else if mesh_key.contains(MeshPipelineKey::MOTION_VECTOR_PREPASS) {
        Some(Face::Back)
    } else {
        current
    }
}

/// A texture's GPU image, uploaded once per texture path; bases repeat, the iris does not.
pub(super) fn texture_handle(
    handles: &mut std::collections::BTreeMap<String, Handle<Image>>,
    images: &mut Assets<Image>,
    (path, image, repeat): (&str, &source_assets::vtf::GpuImage, bool),
    filtering: hl2_ui::config::Filtering,
) -> Handle<Image> {
    handles
        .entry(path.to_owned())
        .or_insert_with(|| images.add(gpu_image_filtered(image.clone(), repeat, false, filtering)))
        .clone()
}

/// Owned variant of [`texture_handle`], moving `GpuImage` into Bevy without cloning.
pub(super) fn texture_handle_owned(
    handles: &mut std::collections::BTreeMap<String, Handle<Image>>,
    images: &mut Assets<Image>,
    (path, image, repeat, srgb): (&str, source_assets::vtf::GpuImage, bool, bool),
    filtering: hl2_ui::config::Filtering,
) -> Handle<Image> {
    if let Some(handle) = handles.get(path) {
        return handle.clone();
    }
    let handle = images.add(gpu_image_filtered(image, repeat, srgb, filtering));
    handles.insert(path.to_owned(), handle.clone());
    handle
}

pub(super) struct UploadedTextures {
    pub handles: std::collections::BTreeMap<String, Handle<Image>>,
    pub bases: std::collections::BTreeMap<String, Handle<Image>>,
    pub seconds: std::collections::BTreeMap<String, Handle<Image>>,
    pub irises: std::collections::BTreeMap<String, Handle<Image>>,
    pub overlays: std::collections::BTreeMap<String, Handle<Image>>,
    pub two_textures: super::map_draw::TwoTextures,
    pub envmaps: std::collections::BTreeMap<String, Handle<Image>>,
    pub envmap_masks: std::collections::BTreeMap<String, Handle<Image>>,
}

type PendingGpuImages =
    std::collections::BTreeMap<String, (std::sync::Arc<source_assets::vtf::GpuImage>, bool, bool)>;

struct DrainedSlots {
    pending: PendingGpuImages,
    bases: std::collections::BTreeMap<String, Handle<Image>>,
    base_keys: Vec<(String, String)>,
    second_keys: Vec<(String, String)>,
    iris_keys: Vec<(String, String)>,
    overlay_keys: Vec<(String, String)>,
    mask_keys: Vec<(String, String)>,
}

fn drain_material_gpu_images(
    materials: &mut std::collections::BTreeMap<String, MaterialData>,
    camera_target: &Handle<Image>,
    missing: &Handle<Image>,
) -> DrainedSlots {
    let mut pending: PendingGpuImages = std::collections::BTreeMap::new();
    let mut queue =
        |key: &str, arc: std::sync::Arc<source_assets::vtf::GpuImage>, repeat: bool, srgb: bool| {
            pending
                .entry(key.to_owned())
                .and_modify(|(_, existing_repeat, _)| *existing_repeat |= repeat)
                .or_insert((arc, repeat, srgb));
        };
    let (mut bases, mut base_keys, mut second_keys, mut iris_keys, mut overlay_keys, mut mask_keys) = (
        std::collections::BTreeMap::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    for (name, material) in materials {
        if material.camera {
            bases.insert(name.clone(), camera_target.clone());
            let _ = material.base.take();
        } else if let (Some(arc), Some(path)) = (material.base.take(), material.base_path.as_ref())
        {
            queue(path, arc, true, false);
            base_keys.push((name.clone(), path.clone()));
        } else {
            bases.insert(name.clone(), missing.clone());
        }
        if let Some((path, arc)) = material.base2.take() {
            queue(&path, arc, true, false);
            second_keys.push((name.clone(), path));
        }
        if let (Some(arc), Some(path)) = (material.iris.take(), material.iris_path.as_ref()) {
            queue(path, arc, false, false);
            iris_keys.push((name.clone(), path.clone()));
        }
        if let (Some(arc), Some(path)) = (
            material.camera_overlay.take(),
            material.camera_overlay_path.as_ref(),
        ) {
            let key = format!("{path}#srgb");
            queue(&key, arc, true, true);
            overlay_keys.push((name.clone(), key));
        }
        if let (Some(arc), Some(path)) = (
            material.envmap_mask.take(),
            material.envmap_mask_path.as_ref(),
        ) {
            queue(path, arc, true, false);
            mask_keys.push((name.clone(), path.clone()));
        }
    }
    DrainedSlots {
        pending,
        bases,
        base_keys,
        second_keys,
        iris_keys,
        overlay_keys,
        mask_keys,
    }
}

pub(super) fn upload_map_textures(
    loaded: &mut crate::assets::LoadedMap,
    images: &mut Assets<Image>,
    camera_target: &Handle<Image>,
    (filtering, nearest): (hl2_ui::config::Filtering, bool),
    missing: &Handle<Image>,
) -> UploadedTextures {
    let mut handles = std::collections::BTreeMap::new();
    let mut drained = drain_material_gpu_images(&mut loaded.materials, camera_target, missing);
    for (key, (arc, repeat, srgb)) in drained.pending {
        match std::sync::Arc::try_unwrap(arc) {
            Ok(owned) => {
                texture_handle_owned(&mut handles, images, (&key, owned, repeat, srgb), filtering);
            }
            Err(shared) if !srgb => {
                texture_handle(&mut handles, images, (&key, &shared, repeat), filtering);
            }
            Err(shared) => {
                texture_handle_owned(
                    &mut handles,
                    images,
                    (&key, (*shared).clone(), repeat, true),
                    filtering,
                );
            }
        }
    }
    for (name, key) in drained.base_keys {
        drained.bases.insert(name, handles[&key].clone());
    }
    let resolve =
        |keys: Vec<(String, String)>| -> std::collections::BTreeMap<String, Handle<Image>> {
            keys.into_iter()
                .map(|(name, key)| (name, handles[&key].clone()))
                .collect()
        };
    let seconds = resolve(drained.second_keys);
    let irises = resolve(drained.iris_keys);
    let overlays = resolve(drained.overlay_keys);
    let envmap_masks = resolve(drained.mask_keys);
    let two_textures = upload_two_textures(&loaded.materials, &mut handles, images, filtering);
    let envmaps = upload_envmaps(&loaded.materials, &mut handles, images, filtering);
    unfilter(images, handles.values(), nearest);
    UploadedTextures {
        handles,
        bases: drained.bases,
        seconds,
        irises,
        overlays,
        two_textures,
        envmaps,
        envmap_masks,
    }
}

fn upload_two_textures(
    materials: &std::collections::BTreeMap<String, MaterialData>,
    handles: &mut std::collections::BTreeMap<String, Handle<Image>>,
    images: &mut Assets<Image>,
    filtering: hl2_ui::config::Filtering,
) -> super::map_draw::TwoTextures {
    materials
        .iter()
        .filter_map(|(name, m)| {
            let textures = m.two_texture.as_ref()?;
            let mut upload = |path: &str, frames: &crate::assets::TextureFrames| {
                frames
                    .iter()
                    .enumerate()
                    .map(|(i, chain)| {
                        handles
                            .entry(format!("{path}#frame{i}#mips#srgb"))
                            .or_insert_with(|| images.add(mip_image_filtered(chain, filtering)))
                            .clone()
                    })
                    .collect::<Vec<_>>()
            };
            let base = upload(m.base_path.as_deref()?, &textures.base_frames);
            let second = upload(&textures.second_path, &textures.second_frames);
            Some((name.clone(), (base, second)))
        })
        .collect()
}

fn upload_envmaps(
    materials: &std::collections::BTreeMap<String, MaterialData>,
    handles: &mut std::collections::BTreeMap<String, Handle<Image>>,
    images: &mut Assets<Image>,
    filtering: hl2_ui::config::Filtering,
) -> std::collections::BTreeMap<String, Handle<Image>> {
    materials
        .iter()
        .filter_map(|(name, m)| {
            let (cube, path) = m.envmap.as_ref().zip(m.envmap_path.as_ref())?;
            let handle = handles
                .entry(format!("{path}#cube"))
                .or_insert_with(|| images.add(cube_image_filtered(cube, filtering)))
                .clone();
            Some((name.clone(), handle))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decal_modulate_multiplies_the_frame_unlit() {
        let mut images = Assets::<Image>::default();
        let white = images.add(image(1, 1, vec![255; 4], false));
        let (cube, cookie) = (Handle::default(), Handle::default());
        let definition = MaterialData {
            modulate: true,
            ..Default::default()
        };
        let decal = make_material(
            &definition,
            white.clone(),
            white.clone(),
            None,
            (&white, &cube, &cookie),
        );
        assert_eq!(decal.alpha, AlphaMode::Multiply);
        assert_eq!(decal.parameters.z, MODULATE_OUTPUT);
        // Alpha-tested above zero, and no flashlight on it.
        assert!(decal.parameters.x > 0. && decal.parameters.x < 0.01);
        assert_eq!(decal.lighting.params.w, 1.);
    }

    #[test]
    fn a_second_base_selects_the_vertex_alpha_blend_and_binds_the_second_slot() {
        let mut images = Assets::<Image>::default();
        let (white, cube, cookie) = (
            images.add(image(1, 1, vec![255; 4], false)),
            Handle::default(),
            Handle::default(),
        );
        let second = images.add(image(1, 1, vec![9; 4], true));
        let mut definition = MaterialData::default();
        let plain = make_material(
            &definition,
            white.clone(),
            white.clone(),
            None,
            (&white, &cube, &cookie),
        );
        assert_eq!(plain.parameters.w, 0.);
        definition.base2 = Some((
            "b.vtf".into(),
            std::sync::Arc::new(source_assets::vtf::GpuImage {
                width: 1,
                height: 1,
                data: vec![9; 4],
                pixel_format: source_assets::vtf::GpuFormat::Rgba8,
                vtf_format: 0,
                translucent: false,
            }),
        ));
        let blended = make_material(
            &definition,
            white.clone(),
            white.clone(),
            Some(second.clone()),
            (&white, &cube, &cookie),
        );
        assert_eq!(blended.parameters.w, BLEND_SECOND_BASE);
        assert_eq!(blended.iris, second);
        // Lightmapped like any world surface (the eye mode alone drops the lightmap).
        assert_eq!(blended.lighting.params.w, 0.);
    }

    #[test]
    fn motion_prepass_culls_like_the_colour_pass_and_shadows_keep_both_faces() {
        let motion = MeshPipelineKey::DEPTH_PREPASS | MeshPipelineKey::MOTION_VECTOR_PREPASS;
        assert_eq!(faces_culled(None, false, motion), Some(Face::Back));
        assert_eq!(faces_culled(None, true, motion), None);
        assert_eq!(
            faces_culled(None, false, MeshPipelineKey::DEPTH_PREPASS),
            None
        );
        let colour = MeshPipelineKey::NONE;
        assert_eq!(
            faces_culled(Some(Face::Back), false, colour),
            Some(Face::Back)
        );
        assert_eq!(faces_culled(Some(Face::Back), true, colour), None);
    }

    #[test]
    fn material_key_only_enables_discard_for_non_opaque_or_alpha_tested_materials() {
        let mut images = Assets::<Image>::default();
        let white = images.add(image(1, 1, vec![255; 4], false));
        let (cube, cookie) = (Handle::default(), Handle::default());
        let opaque = make_material(
            &MaterialData::default(),
            white.clone(),
            white.clone(),
            None,
            (&white, &cube, &cookie),
        );
        assert!(!MaterialKey::from(&opaque).may_discard);

        let mut custom_cutoff = opaque.clone();
        custom_cutoff.parameters.x = 0.25;
        assert!(MaterialKey::from(&custom_cutoff).may_discard);

        for (alpha_cutoff, translucent, additive, modulate) in [
            (Some(0.5), false, false, false),
            (None, true, false, false),
            (None, false, true, false),
            (None, false, false, true),
        ] {
            let mat = make_material(
                &MaterialData {
                    alpha_cutoff,
                    translucent,
                    additive,
                    modulate,
                    ..Default::default()
                },
                white.clone(),
                white.clone(),
                None,
                (&white, &cube, &cookie),
            );
            assert!(MaterialKey::from(&mat).may_discard);
        }
    }

    #[test]
    fn gpu_image_filtered_maps_compressed_and_rgba_formats() {
        use source_assets::vtf::{GpuFormat, GpuImage};
        let cases = [
            (GpuFormat::Rgba8, 4, 4, 64, false, TextureFormat::Rgba8Unorm),
            (
                GpuFormat::Rgba8,
                4,
                4,
                64,
                true,
                TextureFormat::Rgba8UnormSrgb,
            ),
            (GpuFormat::Bc1, 4, 4, 8, false, TextureFormat::Bc1RgbaUnorm),
            (
                GpuFormat::Bc1,
                4,
                4,
                8,
                true,
                TextureFormat::Bc1RgbaUnormSrgb,
            ),
            (GpuFormat::Bc2, 4, 4, 16, false, TextureFormat::Bc2RgbaUnorm),
            (
                GpuFormat::Bc2,
                4,
                4,
                16,
                true,
                TextureFormat::Bc2RgbaUnormSrgb,
            ),
            (GpuFormat::Bc3, 4, 4, 16, false, TextureFormat::Bc3RgbaUnorm),
            (
                GpuFormat::Bc3,
                4,
                4,
                16,
                true,
                TextureFormat::Bc3RgbaUnormSrgb,
            ),
        ];
        for (pixel_format, width, height, bytes, srgb, expected) in cases {
            let img = gpu_image_filtered(
                GpuImage {
                    width,
                    height,
                    data: vec![0; bytes],
                    pixel_format,
                    vtf_format: 0,
                    translucent: false,
                },
                true,
                srgb,
                hl2_ui::config::Filtering::Trilinear,
            );
            assert_eq!(img.texture_descriptor.format, expected);
            assert_eq!(img.data.as_ref().map(Vec::len), Some(bytes));
        }
    }

    #[test]
    fn mip_and_cube_images_preserve_mip_levels_and_byte_layout() {
        let chain = [
            source_assets::vtf::Image {
                width: 2,
                height: 2,
                rgba: vec![1; 16],
                format: 0,
                translucent: false,
            },
            source_assets::vtf::Image {
                width: 1,
                height: 1,
                rgba: vec![2; 4],
                format: 0,
                translucent: false,
            },
        ];
        let mip = mip_image_filtered(&chain, hl2_ui::config::Filtering::Trilinear);
        assert_eq!(mip.texture_descriptor.mip_level_count, 2);
        assert_eq!(mip.texture_descriptor.format, TextureFormat::Rgba8UnormSrgb);
        assert_eq!(mip.data.as_ref().map(Vec::len), Some(20));

        let cube = source_assets::vtf::Cube {
            size: 2,
            hdr: false,
            faces: std::array::from_fn(|_| vec![3; 16]),
            mips: vec![std::array::from_fn(|_| vec![4; 4])],
        };
        let cube_img = cube_image_filtered(&cube, hl2_ui::config::Filtering::Trilinear);
        assert_eq!(cube_img.texture_descriptor.mip_level_count, 2);
        assert_eq!(cube_img.data, Some(cube.layers()));
    }

    #[test]
    fn material_key_specializes_model_lit_and_envmap_independently() {
        let mut images = Assets::<Image>::default();
        let white = images.add(image(1, 1, vec![255; 4], false));
        let (cube, cookie) = (Handle::default(), Handle::default());

        let world = make_material(
            &MaterialData::default(),
            white.clone(),
            white.clone(),
            None,
            (&white, &cube, &cookie),
        );
        let world_key = MaterialKey::from(&world);
        assert!(!world_key.model_lit);
        assert!(!world_key.envmap);

        let mut world_refl = world.clone();
        world_refl.envmap_tint.w = 1.;
        let world_refl_key = MaterialKey::from(&world_refl);
        assert!(!world_refl_key.model_lit);
        assert!(world_refl_key.envmap);

        let mut model = world.clone();
        model.lighting = ModelLighting::fallback(false);
        let model_key = MaterialKey::from(&model);
        assert!(model_key.model_lit);
        assert!(!model_key.envmap);

        let eye = make_material(
            &MaterialData::default(),
            white.clone(),
            white.clone(),
            Some(white.clone()),
            (&white, &cube, &cookie),
        );
        assert!(MaterialKey::from(&eye).model_lit);

        let mut sky = world;
        sky.parameters.w = -2.;
        sky.lighting.params = Vec4::W;
        let sky_key = MaterialKey::from(&sky);
        assert!(!sky_key.model_lit);
        assert!(!sky_key.envmap);
    }
}
