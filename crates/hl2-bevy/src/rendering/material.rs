//! Textures and Source materials for draws: fallback images, filtered samplers, VTF mip chains and the
//! `SourceMaterial` a material definition makes.
use super::{MaterialData, ModelLighting, SourceMaterial};
use bevy::{
    asset::{Handle, RenderAssetUsages},
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
    cube_image(1, false, vec![0; 6 * 4])
}

/// A six-layer cube texture: RGBA16F linear halves (HDR) or sRGB-encoded RGBA8 (LDR).
pub(super) fn cube_image_filtered(
    size: u16,
    hdr: bool,
    data: Vec<u8>,
    filtering: hl2_ui::config::Filtering,
) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: size.into(),
            height: size.into(),
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        data,
        if hdr {
            TextureFormat::Rgba16Float
        } else {
            TextureFormat::Rgba8UnormSrgb
        },
        RenderAssetUsages::RENDER_WORLD,
    );
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

pub(super) fn cube_image(size: u16, hdr: bool, data: Vec<u8>) -> Image {
    cube_image_filtered(size, hdr, data, hl2_ui::config::Filtering::Trilinear)
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

/// A repeating sRGB texture with its authored VTF mip chain (largest first), sampled
/// according to the configured filtering mode.
pub(super) fn mip_image_filtered(
    chain: &[source_assets::vtf::Image],
    filtering: hl2_ui::config::Filtering,
) -> Image {
    let top = &chain[0];
    let mut result = image_filtered(top.width, top.height, top.rgba.clone(), true, filtering);
    // Image::new checks one level; the remaining mips follow it in the same buffer.
    result.data = Some(
        chain
            .iter()
            .flat_map(|mip| mip.rgba.iter().copied())
            .collect(),
    );
    result.texture_descriptor.format = TextureFormat::Rgba8UnormSrgb;
    result.texture_descriptor.mip_level_count = chain.len() as u32;
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
        || matches!(alpha, AlphaMode::Add);
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
            f32::from(matches!(alpha, AlphaMode::Add)),
            if definition.camera {
                if definition.camera_vertex_color {
                    -2.
                } else {
                    -1.
                }
            } else if definition.two_texture.is_some() {
                -3.
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
        envmap: black_cube.clone(),
        envmap_tint: Vec3::from_array(definition.envmap_tint).extend(0.),
        envmap_contrast: Vec3::from_array(definition.envmap_contrast)
            .extend(definition.fresnel_reflection),
        envmap_saturation: Vec3::from_array(definition.envmap_saturation).extend(
            f32::from(u8::from(definition.base_alpha_envmap_mask))
                + if definition.envmap_mask.is_none() {
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
    if material.additive {
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
}
impl From<&SourceMaterial> for MaterialKey {
    fn from(material: &SourceMaterial) -> Self {
        Self {
            two_sided: material.two_sided,
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
        if key.bind_group_data.two_sided {
            descriptor.primitive.cull_mode = None;
        }
        Ok(())
    }
}
