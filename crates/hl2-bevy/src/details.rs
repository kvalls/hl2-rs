//! Detail sprites (SDK CDetailObjectSystem fast-sprite path): authored grass billboards from
//! the BSP 'dprp' lump. The vertex shader faces each sprite toward the view that draws it, in
//! the horizontal plane (orientation 2), and fades it by that view's squared distance.
use bevy::{
    asset::RenderAssetUsages,
    camera::primitives::Aabb,
    mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology},
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};
use modkit_core::{DetailInstance, DetailSprite};
use source_assets::{vpk::Vfs, vtf};
use std::collections::BTreeMap;

/// cl_detaildist and cl_detailfade defaults: opaque to 800 units, gone at 1200.
const DETAIL_DIST: f32 = 1200.;
const DETAIL_FADE: f32 = 400.;

pub struct PreparedDetails {
    atlas: Vec<vtf::Image>,
    instances: Vec<DetailInstance>,
    sprites: Vec<DetailSprite>,
    error: Option<String>,
}
impl PreparedDetails {
    pub fn load(vfs: &Vfs, world: &modkit_core::World) -> Self {
        let details = &world.details;
        let mut prepared = Self {
            atlas: Vec::new(),
            instances: details.instances.clone(),
            sprites: details.sprites.clone(),
            error: None,
        };
        if details.instances.is_empty() {
            return prepared;
        }
        let result = (|| -> anyhow::Result<Vec<vtf::Image>> {
            use anyhow::Context;
            let base = vfs
                .base_texture(&details.material)?
                .context("detail material has no base texture")?;
            let data = vfs
                .read(&format!("materials/{}.vtf", base.trim_end_matches(".vtf")))?
                .context("detail atlas texture missing")?;
            vtf::decode_frame_mips(&data, 1024, 64 * 1024 * 1024)?
                .into_iter()
                .next()
                .context("detail atlas has no frames")
        })();
        match result {
            Ok(atlas) => prepared.atlas = atlas,
            Err(e) => prepared.error = Some(format!("{}: {e:#}", details.material)),
        }
        prepared
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct DetailMaterial {
    #[texture(0)]
    #[sampler(1)]
    texture: Handle<Image>,
    /// (maximum distance squared, fade start distance squared, unused, unused).
    #[uniform(2)]
    fade: Vec4,
}
impl Material for DetailMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/detail.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/detail.wgsl".into()
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
        // $nocull: both faces are drawn.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Detail sprites waiting for the next frame's spawn (keeps install_map's asset tuple small).
#[derive(Resource)]
pub struct PendingDetails(pub PreparedDetails);

#[derive(Resource, Default, Clone, serde::Serialize)]
pub struct DetailReport {
    pub sprites: usize,
    pub leaf_meshes: usize,
    pub unsupported: usize,
    pub error: Option<String>,
}

/// TexLightToLinear, engine LinearToGamma and byte quantization, then the vertex shader's
/// GammaToLinear: the linear color the fast-sprite path draws with.
pub fn fast_sprite_color(rgbexp: [u8; 4]) -> [f32; 3] {
    let exponent = rgbexp[3] as i8;
    let scale = 2f32.powi(i32::from(exponent)) / 255.;
    [0, 1, 2].map(|i| {
        let linear = f32::from(rgbexp[i]) * scale;
        let gamma = linear.max(0.).powf(1. / 2.2).min(1.);
        ((gamma * 255.).round() / 255.).powf(2.2)
    })
}
/// Fade alpha for a squared distance (linear in squared distance, quantized to a byte);
/// the reference for the vertex shader's per-view fade.
#[cfg(test)]
pub fn fade_alpha(distance_squared: f32) -> f32 {
    let max = DETAIL_DIST * DETAIL_DIST;
    let start = (DETAIL_DIST - DETAIL_FADE).powi(2).min(max - 1.);
    (((max - distance_squared) / (max - start)).clamp(0., 1.) * 255.).floor() / 255.
}
/// Corner offsets (right, up) and UVs in native order: bottom-right, top-right, top-left,
/// bottom-left. `mirrored` swaps U (alternates per authored record, the first mirrored).
pub fn sprite_corners(sprite: &DetailSprite, scale: f32, mirrored: bool) -> [(Vec2, Vec2); 4] {
    let width = scale * (sprite.lr.x - sprite.ul.x);
    let height = scale * (sprite.ul.y - sprite.lr.y);
    let (mut u0, mut u1) = (sprite.tex_ul.x, sprite.tex_lr.x);
    if mirrored {
        std::mem::swap(&mut u0, &mut u1);
    }
    let (v_top, v_bottom) = (sprite.tex_ul.y, sprite.tex_lr.y);
    let bottom = scale * sprite.lr.y;
    let centre = scale * (sprite.ul.x + sprite.lr.x) * 0.5;
    let (l, r) = (centre - width * 0.5, centre + width * 0.5);
    [
        (Vec2::new(r, bottom), Vec2::new(u1, v_bottom)),
        (Vec2::new(r, bottom + height), Vec2::new(u1, v_top)),
        (Vec2::new(l, bottom + height), Vec2::new(u0, v_top)),
        (Vec2::new(l, bottom), Vec2::new(u0, v_bottom)),
    ]
}

pub fn spawn_pending(
    mut commands: Commands,
    pending: Option<ResMut<PendingDetails>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DetailMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(mut pending) = pending else { return };
    commands.remove_resource::<PendingDetails>();
    let prepared = std::mem::replace(
        &mut pending.0,
        PreparedDetails {
            atlas: Vec::new(),
            instances: Vec::new(),
            sprites: Vec::new(),
            error: None,
        },
    );
    let mut report = DetailReport {
        error: prepared.error.clone(),
        ..Default::default()
    };
    if prepared.atlas.is_empty() {
        report.unsupported = prepared.instances.len();
        commands.insert_resource(report);
        return;
    }
    let texture = images.add(crate::rendering::mip_image(&prepared.atlas));
    let material = materials.add(DetailMaterial {
        texture,
        fade: Vec4::new(
            DETAIL_DIST * DETAIL_DIST,
            (DETAIL_DIST - DETAIL_FADE).powi(2),
            0.,
            0.,
        ),
    });
    // One mesh per authored leaf: Bevy sorts these translucent meshes per view.
    let mut leaves: BTreeMap<u16, (Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<[f32; 2]>, Vec<[f32; 4]>)> =
        BTreeMap::new();
    let mut extent: BTreeMap<u16, (Vec3, Vec3)> = BTreeMap::new();
    for (record, instance) in prepared.instances.iter().enumerate() {
        // Only the fast path: vertical screen-aligned sprites without supplemental styles.
        let sprite = prepared.sprites.get(usize::from(instance.index));
        let (true, Some(sprite)) = (
            instance.kind == 1 && instance.orientation == 2 && instance.style_count == 0,
            sprite,
        ) else {
            report.unsupported += 1;
            continue;
        };
        let origin = crate::source_to_bevy(Vec3::from_array(instance.origin.to_array()));
        let color = fast_sprite_color(instance.lighting);
        let corners = sprite_corners(sprite, instance.scale, record % 2 == 0);
        let (positions, uvs, offsets, colors) = leaves.entry(instance.leaf).or_default();
        for (offset, uv) in corners {
            positions.push(origin.to_array());
            uvs.push(uv.to_array());
            offsets.push(offset.to_array());
            colors.push([color[0], color[1], color[2], 1.]);
        }
        let reach =
            Vec2::new(sprite.ul.x.abs().max(sprite.lr.x.abs()), 0.).length() * instance.scale + 1.;
        let height = instance.scale * sprite.ul.y.max(sprite.lr.y).abs() + 1.;
        let lo = origin - Vec3::new(reach, 0., reach);
        let hi = origin + Vec3::new(reach, height, reach);
        let e = extent.entry(instance.leaf).or_insert((lo, hi));
        e.0 = e.0.min(lo);
        e.1 = e.1.max(hi);
        report.sprites += 1;
    }
    for (leaf, (positions, uvs, offsets, colors)) in leaves {
        let quads = positions.len() / 4;
        let mut indices = Vec::with_capacity(quads * 6);
        for q in 0..quads as u32 {
            let b = q * 4;
            indices.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
        }
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, offsets);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        mesh.insert_indices(Indices::U32(indices));
        let (lo, hi) = extent[&leaf];
        commands.spawn((
            crate::campaign::MapOwned,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material.clone()),
            Transform::IDENTITY,
            // Every billboard direction fits inside the leaf's padded origin bounds.
            Aabb::from_min_max(lo, hi),
        ));
        report.leaf_meshes += 1;
    }
    commands.insert_resource(report);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sprite() -> DetailSprite {
        DetailSprite {
            ul: glam::Vec2::new(-9., 21.),
            lr: glam::Vec2::new(9., 0.),
            tex_ul: glam::Vec2::new(0.25, 0.5),
            tex_lr: glam::Vec2::new(0.5, 0.75),
        }
    }

    #[test]
    fn fade_is_linear_in_squared_distance() {
        assert_eq!(fade_alpha(800. * 800.), 1.);
        assert_eq!(fade_alpha(1200. * 1200.), 0.);
        // At 1000 units: (1200^2 - 1000^2) / (1200^2 - 800^2) = 0.55, byte-quantized.
        assert!((fade_alpha(1000. * 1000.) - 0.55).abs() < 1. / 255.);
        assert_eq!(fade_alpha(0.), 1.);
    }

    #[test]
    fn corners_are_bottom_anchored_and_alternate_u() {
        let c = sprite_corners(&sprite(), 0.5, false);
        // Bottom-right, top-right, top-left, bottom-left around the origin.
        assert_eq!(c[0].0, Vec2::new(4.5, 0.));
        assert_eq!(c[1].0, Vec2::new(4.5, 10.5));
        assert_eq!(c[2].0, Vec2::new(-4.5, 10.5));
        assert_eq!(c[3].0, Vec2::new(-4.5, 0.));
        assert_eq!(c[0].1, Vec2::new(0.5, 0.75));
        assert_eq!(c[2].1, Vec2::new(0.25, 0.5));
        let m = sprite_corners(&sprite(), 0.5, true);
        assert_eq!(m[0].1, Vec2::new(0.25, 0.75));
        assert_eq!(m[2].1, Vec2::new(0.5, 0.5));
    }

    #[test]
    fn fast_lighting_round_trips_through_gamma_bytes() {
        // 128 * 2^0 / 255 is about 0.502 linear; the byte round trip keeps it within 1%.
        let c = fast_sprite_color([128, 0, 255, 0]);
        assert!((c[0] - 128. / 255.).abs() < 0.01);
        assert_eq!(c[1], 0.);
        assert_eq!(c[2], 1.);
        // Overbright light clamps at the gamma byte.
        assert_eq!(fast_sprite_color([255, 255, 255, 2])[0], 1.);
        // A negative exponent darkens.
        assert!(fast_sprite_color([200, 200, 200, (-2i8) as u8])[0] < 0.2);
    }
}
