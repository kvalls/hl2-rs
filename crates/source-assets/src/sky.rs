//! Static LDR sky assets only; GPU drawing, leaf visibility and HDR are separate.
use crate::{bytes, keyvalues, u16le, u32le, vpk, vtf};
use anyhow::{bail, Context, Result};
use glam::Vec2;
use modkit_core::Entity;

/// Source sky material order, distinct from VTF cubemap face order.
pub const SUFFIXES: [&str; 6] = ["rt", "bk", "lf", "ft", "up", "dn"];

/// Direction and planar texture mapping used by the owned retail sky cube.
pub fn face_vector(face: usize, s: f32, t: f32) -> glam::Vec3 {
    use glam::Vec3;
    match face {
        0 => Vec3::new(1., -s, t),
        1 => Vec3::new(s, 1., t),
        2 => Vec3::new(-1., s, t),
        3 => Vec3::new(-s, -1., t),
        4 => Vec3::new(-t, -s, 1.),
        5 => Vec3::new(t, -s, -1.),
        _ => panic!("sky requires exactly six faces"),
    }
}
pub fn face_uv(s: f32, t: f32, transform: UvTransform) -> Vec2 {
    let edge = 1. / 512.;
    transform.apply(Vec2::new(
        ((s + 1.) * 0.5).clamp(edge, 1. - edge),
        1. - ((t + 1.) * 0.5).clamp(edge, 1. - edge),
    ))
}

#[derive(Debug)]
pub struct Skybox {
    pub name: String,
    /// +X, +Y, -X, -Y, +Z, -Z respectively, in Source's Z-up coordinates.
    pub faces: [SkyFace; 6],
}

#[derive(Debug)]
pub struct SkyFace {
    pub suffix: &'static str,
    pub material: String,
    pub texture: String,
    pub image: vtf::Image,
    pub transform: UvTransform,
    /// Sky_HDR_DX9 `$hdrcompressedTexture` (RGBS) decoded per texel to linear RGB
    /// (rgb * alpha * 8) as RGBA16F bits, when the material has one that decodes.
    pub hdr: Option<HdrImage>,
}

#[derive(Debug)]
pub struct HdrImage {
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u16>,
}
impl HdrImage {
    /// sky_hdr_compressed_rgbs: InputScale 8 times rgb times alpha, read without sRGB.
    pub fn from_rgbs(image: &vtf::Image) -> Self {
        let one = half::f16::ONE.to_bits();
        let rgba = image
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| {
                let scale = 8. * p[3] as f32 / 255.;
                [p[0], p[1], p[2]]
                    .map(|c| half::f16::from_f32(c as f32 / 255. * scale).to_bits())
                    .into_iter()
                    .chain([one])
            })
            .collect();
        Self {
            width: image.width,
            height: image.height,
            rgba,
        }
    }
}

/// Affine UV rows. Sampling must clamp after applying the material transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvTransform {
    rows: [[f32; 3]; 2],
}
impl Default for UvTransform {
    fn default() -> Self {
        Self {
            rows: [[1., 0., 0.], [0., 1., 0.]],
        }
    }
}
impl UvTransform {
    /// Parse named center/scale/rotate/translate clauses; omitted clauses use
    /// center (.5,.5), scale (1,1), rotation 0 degrees and translation (0,0).
    /// Duplicate or unsupported clauses and nonfinite matrices are rejected.
    pub fn parse(text: &str) -> Result<Self> {
        if text.is_empty() || text.len() > 256 {
            bail!("sky UV transform must contain 1..256 bytes");
        }
        let tokens = text.split_whitespace().collect::<Vec<_>>();
        let mut seen = [false; 4];
        let mut center = Vec2::splat(0.5);
        let mut scale = Vec2::ONE;
        let mut rotate = 0.;
        let mut translate = Vec2::ZERO;
        let mut i = 0;
        while i < tokens.len() {
            let clause = tokens[i].to_ascii_lowercase();
            let (id, count) = match clause.as_str() {
                "center" => (0, 2),
                "scale" => (1, 2),
                "rotate" => (2, 1),
                "translate" => (3, 2),
                _ => bail!("unsupported sky UV transform clause: {}", tokens[i]),
            };
            if seen[id] {
                bail!("duplicate sky UV transform clause: {clause}");
            }
            seen[id] = true;
            let mut values = [0.; 2];
            for (n, value) in values.iter_mut().enumerate().take(count) {
                *value = tokens
                    .get(i + n + 1)
                    .with_context(|| format!("missing {clause} value"))?
                    .parse::<f32>()
                    .with_context(|| format!("invalid {clause} value"))?;
                if !value.is_finite() {
                    bail!("nonfinite sky UV transform value");
                }
            }
            match id {
                0 => center = Vec2::from_array(values),
                1 => scale = Vec2::from_array(values),
                2 => rotate = values[0],
                3 => translate = Vec2::from_array(values),
                _ => unreachable!(),
            }
            i += count + 1;
        }
        if !seen.iter().any(|v| *v) {
            bail!("empty sky UV transform");
        }
        // Published TextureTransform proxy: T(translation) T(center) Rz S T(-center).
        // https://github.com/ValveSoftware/source-sdk-2013/blob/master/src/game/client/matrixproxy.cpp
        let (sin, cos) = rotate.to_radians().sin_cos();
        let x = [cos * scale.x, -sin * scale.y];
        let y = [sin * scale.x, cos * scale.y];
        let rows = [
            [
                x[0],
                x[1],
                center.x + translate.x - x[0] * center.x - x[1] * center.y,
            ],
            [
                y[0],
                y[1],
                center.y + translate.y - y[0] * center.x - y[1] * center.y,
            ],
        ];
        if rows.iter().flatten().any(|v| !v.is_finite()) {
            bail!("nonfinite sky UV transform matrix");
        }
        Ok(Self { rows })
    }

    pub fn rows(&self) -> [[f32; 3]; 2] {
        self.rows
    }

    pub fn apply(&self, uv: Vec2) -> Vec2 {
        Vec2::new(
            self.rows[0][0] * uv.x + self.rows[0][1] * uv.y + self.rows[0][2],
            self.rows[1][0] * uv.x + self.rows[1][1] * uv.y + self.rows[1][2],
        )
    }
}

/// Read six static LDR faces from the map's worldspawn. No worldspawn/skyname
/// means no sky. An explicit incomplete sky returns an error, never guessed HDR
/// filenames or a silently partial cube. max_dimension is limited to 1..2048.
pub fn load(vfs: &vpk::Vfs, entities: &[Entity], max_dimension: usize) -> Result<Option<Skybox>> {
    load_using(entities, max_dimension, &|name| vfs.read(name))
}

fn asset_name(name: &str) -> Result<String> {
    if name.is_empty()
        || name.len() > 256
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '\\'))
    {
        bail!("invalid sky asset name");
    }
    vpk::normalize(name)
}

fn material_properties(
    read: &impl Fn(&str) -> Result<Option<Vec<u8>>>,
    material: &str,
    depth: usize,
) -> Result<(Option<String>, Option<String>, Option<String>)> {
    if depth > 8 {
        bail!("sky VMT include cycle/depth limit");
    }
    let name = asset_name(material)?;
    let name = name
        .trim_start_matches("materials/")
        .trim_end_matches(".vmt");
    let data = read(&format!("materials/{name}.vmt"))?.context("sky VMT not found")?;
    if data.len() > 1024 * 1024 {
        bail!("sky VMT exceeds 1 MiB limit");
    }
    let entries = keyvalues::parse(&keyvalues::decode_text(&data)?)?;
    if entries.len() != 1 || !matches!(entries[0].value, keyvalues::Value::Block(_)) {
        bail!("sky VMT must contain one material block");
    }
    let root = &entries[0];
    let mut texture = property(root, "$basetexture")?;
    let mut transform = property(root, "$basetexturetransform")?;
    let mut hdr = property(root, "$hdrcompressedtexture")?;
    if root.key.eq_ignore_ascii_case("patch") {
        let include = root
            .get("include")
            .and_then(|e| e.text())
            .context("sky patch missing include")?;
        let (base, base_transform, base_hdr) = material_properties(read, include, depth + 1)?;
        texture = texture.or(base);
        transform = transform.or(base_transform);
        hdr = hdr.or(base_hdr);
        for operation in ["insert", "replace"] {
            if let Some(block) = root.get(operation) {
                if let Some(value) = property(block, "$basetexture")? {
                    if (operation == "replace") == texture.is_some() {
                        texture = Some(value);
                    }
                }
                if let Some(value) = property(block, "$basetexturetransform")? {
                    if (operation == "replace") == transform.is_some() {
                        transform = Some(value);
                    }
                }
            }
        }
    }
    Ok((texture, transform, hdr))
}

fn property(block: &keyvalues::Entry, key: &str) -> Result<Option<String>> {
    block
        .get(key)
        .map(|entry| {
            entry
                .text()
                .with_context(|| format!("sky VMT {key} must be text"))
                .map(str::to_owned)
        })
        .transpose()
}

fn load_using(
    entities: &[Entity],
    max_dimension: usize,
    read: &impl Fn(&str) -> Result<Option<Vec<u8>>>,
) -> Result<Option<Skybox>> {
    if !(1..=2048).contains(&max_dimension) {
        bail!("sky texture dimension limit must be 1..2048");
    }
    let Some(name) = entities
        .iter()
        .find(|e| e.class().eq_ignore_ascii_case("worldspawn"))
        .and_then(|e| e.get("skyname"))
    else {
        return Ok(None);
    };
    let name = asset_name(name)?;
    let mut faces = Vec::with_capacity(6);
    for suffix in SUFFIXES {
        let material = format!("skybox/{name}{suffix}");
        let (texture, transform, hdr) = material_properties(read, &material, 0)
            .with_context(|| format!("sky {name} face {suffix}"))?;
        let texture =
            texture.with_context(|| format!("sky {name} face {suffix} has no LDR $basetexture"))?;
        let texture = asset_name(&texture)?;
        let texture = texture
            .trim_start_matches("materials/")
            .trim_end_matches(".vtf")
            .to_owned();
        let data = read(&format!("materials/{texture}.vtf"))?
            .with_context(|| format!("sky {name} face {suffix} VTF not found"))?;
        if data.len() > 64 * 1024 * 1024 {
            bail!("sky {name} face {suffix} VTF exceeds 64 MiB limit");
        }
        if u32le(&data, 20)? & 0x4000 != 0 || u16le(&data, 24)? != 1 {
            bail!("sky {name} face {suffix} requires a static 2D VTF");
        }
        // Check the available mip before decode can allocate a full-resolution
        // image: a small requested limit does not guarantee a suitable mip exists.
        let width = u16le(&data, 16)? as usize;
        let height = u16le(&data, 18)? as usize;
        let mip_count = bytes(&data, 56, 1)?[0] as usize;
        if !(1..=15).contains(&mip_count) {
            bail!("invalid sky VTF mip count");
        }
        let smallest = (width >> (mip_count - 1)).max(height >> (mip_count - 1));
        if smallest > max_dimension {
            bail!("sky {name} face {suffix} has no mip within dimension limit");
        }
        let image = vtf::decode(&data, max_dimension)
            .with_context(|| format!("decode sky {name} face {suffix}"))?;
        let transform = transform
            .as_deref()
            .map(UvTransform::parse)
            .transpose()
            .with_context(|| format!("sky {name} face {suffix} UV transform"))?
            .unwrap_or_default();
        // The HDR face is optional: a missing or undecodable one keeps the LDR face.
        let hdr = hdr.and_then(|name| {
            let name = asset_name(&name).ok()?;
            let name = name
                .trim_start_matches("materials/")
                .trim_end_matches(".vtf");
            let data = read(&format!("materials/{name}.vtf")).ok()??;
            (data.len() <= 64 * 1024 * 1024).then_some(())?;
            vtf::decode(&data, max_dimension)
                .ok()
                .map(|image| HdrImage::from_rgbs(&image))
        });
        faces.push(SkyFace {
            suffix,
            material,
            texture,
            image,
            transform,
            hdr,
        });
    }
    let faces = faces
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid sky face count"))?;
    Ok(Some(Skybox { name, faces }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn world(name: &str) -> Vec<Entity> {
        vec![Entity {
            properties: vec![
                ("classname".into(), "worldspawn".into()),
                ("skyname".into(), name.into()),
            ],
        }]
    }
    fn fixture() -> BTreeMap<String, Vec<u8>> {
        let mut data = BTreeMap::new();
        for (i, suffix) in SUFFIXES.into_iter().enumerate() {
            let transform = if i == 4 {
                "// $basetexturetransform \"scale 9 9\""
            } else {
                "$basetexturetransform \"center 0 0 scale 1 2 rotate 0 translate 0 0\""
            };
            data.insert(format!("materials/skybox/demo_hdr{suffix}.vmt"), format!("Sky {{ $hdrcompressedtexture bogus $basetexture \"textures/face{i}\" {transform}\n }}").into_bytes());
            let mut image = vec![0; 67];
            image[..4].copy_from_slice(b"VTF\0");
            image[4..8].copy_from_slice(&7u32.to_le_bytes());
            image[8..12].copy_from_slice(&1u32.to_le_bytes());
            image[12..16].copy_from_slice(&64u32.to_le_bytes());
            image[16..18].copy_from_slice(&1u16.to_le_bytes());
            image[18..20].copy_from_slice(&1u16.to_le_bytes());
            image[24..26].copy_from_slice(&1u16.to_le_bytes());
            image[52..56].copy_from_slice(&3u32.to_le_bytes());
            image[56] = 1;
            image[64..67].copy_from_slice(&[3, 2, i as u8]);
            data.insert(format!("materials/textures/face{i}.vtf"), image);
        }
        data
    }
    fn close(a: Vec2, b: Vec2) {
        assert!((a - b).length() < 0.00001, "{a:?} != {b:?}");
    }
    #[test]
    fn transform_applies_scale_rotation_center_then_translation() {
        let t = UvTransform::parse("translate 3 4 rotate 90 scale 2 3 center .5 .5").unwrap();
        close(t.apply(Vec2::new(1., 0.5)), Vec2::new(3.5, 5.5));
        close(t.apply(Vec2::new(0.5, 1.)), Vec2::new(2., 4.5));
        close(t.apply(Vec2::splat(0.5)), Vec2::new(3.5, 4.5));
        close(
            UvTransform::default().apply(Vec2::new(-1., 2.)),
            Vec2::new(-1., 2.),
        );
        close(
            UvTransform::parse("SCALE 1 2").unwrap().apply(Vec2::ZERO),
            Vec2::new(0., -0.5),
        );
    }
    #[test]
    fn transform_rejects_bad_tokens_nonfinite_and_overflow() {
        for text in [
            "",
            "   ",
            "scale 1",
            "rotate NaN",
            "center inf 0",
            "rotate 0 rotate 1",
            "scale 1 2 3",
            "translate 0 0 extra",
            "scale 1e38 1e38 center 1e38 1e38",
        ] {
            assert!(UvTransform::parse(text).is_err(), "accepted {text}");
        }
        assert!(UvTransform::parse(&" ".repeat(257)).is_err());
    }
    #[test]
    fn six_faces_use_actual_ldr_references_and_ignore_commented_transform() {
        let files = fixture();
        let sky = load_using(&world("demo_hdr"), 512, &|name| {
            Ok(files.get(name).cloned())
        })
        .unwrap()
        .unwrap();
        assert_eq!(sky.name, "demo_hdr");
        for (i, face) in sky.faces.iter().enumerate() {
            assert_eq!(face.suffix, SUFFIXES[i]);
            assert_eq!(face.texture, format!("textures/face{i}"));
            assert_eq!(face.image.rgba, [i as u8, 2, 3, 255]);
            close(
                face.transform.apply(Vec2::splat(0.5)),
                Vec2::new(0.5, if i == 4 { 0.5 } else { 1. }),
            );
        }
    }
    #[test]
    fn absent_sky_is_optional_but_explicit_missing_face_is_error() {
        assert!(load_using(&[], 512, &|_| panic!("must not read"))
            .unwrap()
            .is_none());
        let mut files = fixture();
        files.remove("materials/skybox/demo_hdrdn.vmt");
        let error = load_using(&world("demo_hdr"), 512, &|name| {
            Ok(files.get(name).cloned())
        })
        .unwrap_err();
        assert!(format!("{error:#}").contains("face dn"));
        let mut files = fixture();
        files.remove("materials/textures/face5.vtf");
        assert!(load_using(&world("demo_hdr"), 512, &|name| Ok(files
            .get(name)
            .cloned()))
        .is_err());
    }
    #[test]
    fn unsafe_names_and_missing_dimension_mip_are_rejected() {
        for name in [
            "",
            "../escape",
            "C:\\escape",
            "/absolute",
            "bad name",
            "a/./b",
        ] {
            assert!(load_using(&world(name), 512, &|_| panic!("unsafe read")).is_err());
        }
        let mut files = fixture();
        files.get_mut("materials/textures/face0.vtf").unwrap()[16..18]
            .copy_from_slice(&4096u16.to_le_bytes());
        assert!(load_using(&world("demo_hdr"), 512, &|name| Ok(files
            .get(name)
            .cloned()))
        .is_err());
        assert!(load_using(&[], 0, &|_| Ok(None)).is_err());
        assert!(load_using(&[], 2049, &|_| Ok(None)).is_err());
    }
    #[test]
    fn patch_include_replacement_and_cycle_limit() {
        let mut files = fixture();
        files.insert("materials/skybox/demo_hdrrt.vmt".into(), b"Patch { include materials/base.vmt replace { $basetexture textures/face0 $basetexturetransform \"scale 2 2\" } }".to_vec());
        files.insert(
            "materials/base.vmt".into(),
            b"Sky { $basetexture textures/unavailable $basetexturetransform \"scale 1 1\" }"
                .to_vec(),
        );
        let sky = load_using(&world("demo_hdr"), 512, &|name| {
            Ok(files.get(name).cloned())
        })
        .unwrap()
        .unwrap();
        close(sky.faces[0].transform.apply(Vec2::ZERO), Vec2::splat(-0.5));
        files.insert(
            "materials/base.vmt".into(),
            b"Patch { include materials/base.vmt }".to_vec(),
        );
        assert!(load_using(&world("demo_hdr"), 512, &|name| Ok(files
            .get(name)
            .cloned()))
        .is_err());
    }
    #[test]
    fn patch_insert_supplies_missing_property_without_overwriting_base() {
        let mut files = fixture();
        files.insert("materials/skybox/demo_hdrrt.vmt".into(), b"Patch { include base.vmt insert { $basetexture textures/face0 $basetexturetransform \"scale 2 2\" } }".to_vec());
        files.insert("materials/base.vmt".into(), b"Sky {}".to_vec());
        let sky = load_using(&world("demo_hdr"), 512, &|name| {
            Ok(files.get(name).cloned())
        })
        .unwrap()
        .unwrap();
        close(sky.faces[0].transform.apply(Vec2::ZERO), Vec2::splat(-0.5));
        files.insert(
            "materials/base.vmt".into(),
            b"Sky { $basetexture textures/face1 }".to_vec(),
        );
        let sky = load_using(&world("demo_hdr"), 512, &|name| {
            Ok(files.get(name).cloned())
        })
        .unwrap()
        .unwrap();
        assert_eq!(sky.faces[0].texture, "textures/face1");
    }
    #[test]
    fn malformed_material_properties_and_nonstatic_textures_are_rejected() {
        for material in [
            "Sky { $hdrcompressedtexture textures/face0 }",
            "Sky { $basetexture ../../outside }",
            "Sky { $basetexture textures/face0 $basetexturetransform { invalid 1 } }",
            "Sky { $basetexture textures/face0 $basetexturetransform \"rotate inf\" }",
        ] {
            let mut files = fixture();
            files.insert(
                "materials/skybox/demo_hdrrt.vmt".into(),
                material.as_bytes().to_vec(),
            );
            assert!(load_using(&world("demo_hdr"), 512, &|name| Ok(files
                .get(name)
                .cloned()))
            .is_err());
        }
        for (offset, bytes) in [(20, 0x4000u32.to_le_bytes()), (24, 2u32.to_le_bytes())] {
            let mut files = fixture();
            files.get_mut("materials/textures/face0.vtf").unwrap()[offset..offset + 4]
                .copy_from_slice(&bytes);
            assert!(load_using(&world("demo_hdr"), 512, &|name| Ok(files
                .get(name)
                .cloned()))
            .is_err());
        }
    }
}

/// The 2D sky faces are a cube 16,000 units out around the miniature-sky camera and draw in the
/// same depth-tested pass as the miniature sky: every miniature-sky vertex must stay inside that
/// cube from any camera position the playable area maps to, or the faces would cover it.
#[cfg(test)]
mod owned_sky_depth {
    const FACE_DISTANCE: f32 = 16_000.;

    #[test]
    #[ignore = "requires the user's installed HL2 content"]
    fn miniature_sky_geometry_stays_inside_the_2d_sky_faces() {
        let vfs = crate::vpk::Vfs::mount(&crate::install::discover().unwrap()).unwrap();
        let mut checked = 0;
        for map in [
            "d1_trainstation_01",
            "d1_trainstation_02",
            "d1_trainstation_03",
            "d1_trainstation_05",
            "d1_canals_01",
            "d1_canals_05",
            "d1_canals_08",
            "d1_canals_11",
            "d1_canals_13",
            "d1_town_01",
            "d2_coast_01",
            "d2_coast_03",
            "d2_coast_07",
            "d2_coast_10",
            "d2_prison_01",
            "d3_c17_01",
            "d3_c17_07",
            "d3_c17_13",
            "d3_citadel_01",
        ] {
            let Ok(Some(bytes)) = vfs.read(&format!("maps/{map}.bsp")) else {
                continue;
            };
            let world = crate::bsp::Bsp::parse(&bytes).unwrap().world(map).unwrap();
            let Some(sky) = world.background_camera.as_ref() else {
                continue;
            };
            // Camera positions: origin + eye / scale for every eye in the playable area.
            let playable = world.surfaces.iter().filter(|s| !s.background);
            let (lo, hi) = playable.flat_map(|s| &s.vertices).fold(
                (glam::Vec3::splat(f32::MAX), glam::Vec3::splat(f32::MIN)),
                |(lo, hi), v| (lo.min(v.position), hi.max(v.position)),
            );
            let (near, far) = (sky.origin + lo / sky.scale, sky.origin + hi / sky.scale);
            let reach = |p: glam::Vec3| (p - near).abs().max((p - far).abs()).max_element();
            let mut farthest = 0f32;
            for surface in world.surfaces.iter().filter(|s| s.background) {
                assert!(
                    !surface.material.eq_ignore_ascii_case("tools/toolsskybox"),
                    "{map}: a drawn toolsskybox surface would hide the 2D sky"
                );
                for v in &surface.vertices {
                    farthest = farthest.max(reach(v.position));
                }
            }
            for instance in world.model_instances.iter().filter(|i| {
                i.entity
                    .is_some_and(|id| world.background_entities.contains(&id))
            }) {
                let radius = world
                    .model_assets
                    .get(&instance.asset_key())
                    .map_or(0., |s| {
                        s.iter()
                            .flat_map(|s| &s.vertices)
                            .map(|v| v.position.length())
                            .fold(0f32, f32::max)
                    });
                farthest = farthest.max(reach(instance.origin) + radius * instance.scale);
            }
            eprintln!("{map}: miniature sky reaches {farthest:.0} of {FACE_DISTANCE} units");
            assert!(
                farthest < FACE_DISTANCE,
                "{map}: miniature sky reaches {farthest} units"
            );
            checked += 1;
        }
        assert!(checked >= 5, "only {checked} maps with a 3D skybox checked");
    }
}
