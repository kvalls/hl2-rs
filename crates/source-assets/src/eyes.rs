//! Bounded PC studio eyeball metadata; reads the same first body models as meshes.
use crate::{bytes, f32le, i32le, vec3, vpk::Vfs};
use anyhow::{bail, Context, Result};
use glam::Vec3;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Eyeball {
    pub surface: usize,
    pub bone: usize,
    pub origin: Vec3,
    pub up: Vec3,
    pub forward: Vec3,
    pub radius: f32,
    pub z_offset: f32,
    pub iris_scale: f32,
    /// FACS eyelid controls; `None` for non-FACS eyeballs.
    pub lids: Option<EyeLids>,
}
/// mstudioeyeball_t eyelid fields: flex descriptor indices for the upper/lower lid
/// raiser, neutral and lowerer, their lid targets, and the lid descriptors written.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EyeLids {
    pub upper_flex: [i32; 3],
    pub lower_flex: [i32; 3],
    pub upper_target: [f32; 3],
    pub lower_target: [f32; 3],
    pub upper_lid: i32,
    pub lower_lid: i32,
}
/// Server actor attention uses the animated "eyes" attachment when available.
#[derive(Clone, Debug)]
pub struct Attachment {
    pub bone: usize,
    pub local: glam::Mat4,
    pub world_align: bool,
}
/// Default SDK `studiohdr2_t::MaxEyeDeflection` (`cos(30 deg)` when omitted or zero).
pub const DEFAULT_MAX_EYE_DEFLECTION: f32 = 0.866;

#[derive(Clone, Debug)]
pub struct Gaze {
    pub view_offset: Vec3,
    pub attachment: Option<Attachment>,
    pub max_eye_deflection: f32,
}
pub fn load_gaze(vfs: &Vfs, model: &str) -> Result<Gaze> {
    decode_gaze(&vfs.read(model)?.context("gaze MDL absent")?)
}
fn read_max_eye_deflection(data: &[u8]) -> f32 {
    let Ok(hdr2) = i32le(data, 400) else {
        return DEFAULT_MAX_EYE_DEFLECTION;
    };
    let Ok(base) = usize::try_from(hdr2) else {
        return DEFAULT_MAX_EYE_DEFLECTION;
    };
    // `studiohdr_t` ends at byte 408 (`studiohdr2index` at 400); `flMaxEyeDeflection` is at +12.
    if base < 408 || base.checked_add(16).is_none_or(|end| end > data.len()) {
        return DEFAULT_MAX_EYE_DEFLECTION;
    }
    let Ok(deflection) = f32le(data, base + 12) else {
        return DEFAULT_MAX_EYE_DEFLECTION;
    };
    if deflection.is_finite() && deflection > 0. && deflection <= 1. {
        deflection
    } else {
        DEFAULT_MAX_EYE_DEFLECTION
    }
}
/// Clamp an eye view target into the model's `max_deflection` cone around `forward`
/// (SDK C_BaseFlex::SetViewTarget), keeping a minimum forward depth of 6 units.
pub fn clamp_view_target(origin: Vec3, forward: Vec3, target: Vec3, max_deflection: f32) -> Vec3 {
    let forward = forward.normalize_or(Vec3::X);
    let max_cos = if max_deflection.is_finite() && max_deflection > 0. && max_deflection <= 1. {
        max_deflection
    } else {
        DEFAULT_MAX_EYE_DEFLECTION
    };
    let delta = target - origin;
    if !delta.is_finite() {
        return origin + forward * 6.;
    }
    let x = delta.dot(forward);
    let lateral = delta - forward * x;
    let front = forward * x.max(6.) + lateral;
    let dist = front.length();
    let mut dir = front / dist;
    if dir.dot(forward) < max_cos {
        let lat_sq = lateral.length_squared();
        dir = if lat_sq > 1e-12 {
            forward * max_cos + lateral * ((1. - max_cos * max_cos).max(0.) / lat_sq).sqrt()
        } else {
            forward
        };
    }
    origin + dir * dist
}
fn decode_gaze(data: &[u8]) -> Result<Gaze> {
    validate_header(data)?;
    let view_offset = vec3(data, 80)?;
    let attachment = read_attachments(data)?
        .into_iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("eyes"))
        .map(|(_, a)| a);
    Ok(Gaze {
        view_offset,
        attachment,
        max_eye_deflection: read_max_eye_deflection(data),
    })
}
/// Every mstudioattachment_t: name, bone and bone-local 3x4 transform.
pub fn read_attachments(data: &[u8]) -> Result<Vec<(String, Attachment)>> {
    validate_header(data)?;
    let bones = count(data, 156, 256)?;
    let attachments = count(data, 240, 4096)?;
    let base = usize::try_from(i32le(data, 244)?)?;
    bytes(data, base, attachments * 92)?;
    let mut result = Vec::with_capacity(attachments);
    for id in 0..attachments {
        let at = base + id * 92;
        let start = relative(data, at, at)?;
        let tail = data.get(start..).context("attachment name outside MDL")?;
        let end = tail
            .iter()
            .take(1025)
            .position(|b| *b == 0)
            .context("attachment name unterminated or exceeds 1024 bytes")?;
        let name = std::str::from_utf8(&tail[..end])?;
        let bone = usize::try_from(i32le(data, at + 8)?)?;
        if bone >= bones {
            bail!("attachment {name} bone outside skeleton");
        }
        let flags = crate::u32le(data, at + 4)?;
        if flags & !0x10000 != 0 {
            bail!("unsupported attachment {name} flags {flags:#x}");
        }
        let mut matrix = [0.; 16];
        matrix[15] = 1.;
        for row in 0..3 {
            for col in 0..4 {
                matrix[col * 4 + row] = f32le(data, at + 12 + (row * 4 + col) * 4)?;
            }
        }
        let local = glam::Mat4::from_cols_array(&matrix);
        let determinant = local.determinant();
        if !determinant.is_finite() || determinant.abs() < 1e-8 {
            bail!("singular attachment {name} transform");
        }
        result.push((
            name.to_owned(),
            Attachment {
                bone,
                local,
                world_align: flags & 0x10000 != 0,
            },
        ));
    }
    Ok(result)
}
fn validate_header(data: &[u8]) -> Result<()> {
    if data.len() > 64 * 1024 * 1024
        || bytes(data, 0, 4)? != b"IDST"
        || !(44..=49).contains(&i32le(data, 4)?)
        || usize::try_from(i32le(data, 76)?)? != data.len()
    {
        bail!("invalid studio eye header");
    }
    Ok(())
}
/// Model-space eye forward and up toward `target` (or the authored direction), with the
/// retail z-offset adjustment. `bone` is the current bone-to-model transform.
pub fn basis(eye: &Eyeball, bone: glam::Mat4, target: Option<Vec3>) -> (Vec3, Vec3) {
    let origin = bone.transform_point3(eye.origin);
    let authored_up = bone.transform_vector3(eye.up).normalize();
    let fallback = -bone.transform_vector3(eye.forward).normalize();
    let mut forward = target.map_or(fallback, |p| {
        (p - origin).try_normalize().unwrap_or(fallback)
    });
    let right = forward
        .cross(authored_up)
        .try_normalize()
        .unwrap_or_else(|| fallback.cross(authored_up).normalize());
    forward = (forward + right * (eye.z_offset * 2.)).normalize();
    let right = forward.cross(authored_up).normalize();
    (forward, right.cross(forward).normalize())
}
/// Bone-local eye forward/up (VectorIRotate of the model-space basis) for eyelids.
pub fn local_basis(bone: glam::Mat4, (forward, up): (Vec3, Vec3)) -> (Vec3, Vec3) {
    let inverse = glam::Mat3::from_mat4(bone).transpose();
    (inverse * forward, inverse * up)
}
/// Bone-local basis of an eye looking along its authored direction.
pub fn rest_basis(eye: &Eyeball) -> (Vec3, Vec3) {
    local_basis(glam::Mat4::IDENTITY, basis(eye, glam::Mat4::IDENTITY, None))
}
/// Retail R_StudioEyelidFACS (StudioRender 1001bd80, reviewed privately): lid angles are
/// weighted sums of asin(target / radius) over raiser/neutral/lowerer descriptor weights;
/// the lid descriptors receive the authored-up component of the lid point on the eyeball,
/// using the eye's bone-local forward/up.
pub fn apply_eyelids(eye: &Eyeball, weights: &mut [f32], (forward, up): (Vec3, Vec3)) {
    let Some(lids) = &eye.lids else {
        return;
    };
    let r = eye.radius;
    let angle = |flex: &[i32; 3], target: &[f32; 3], weights: &[f32]| -> f32 {
        (0..3)
            .map(|i| {
                let w = usize::try_from(flex[i])
                    .ok()
                    .and_then(|d| weights.get(d))
                    .copied()
                    .unwrap_or(0.);
                (target[i] / r).clamp(-1., 1.).asin() * w
            })
            .sum()
    };
    let upper = angle(&lids.upper_flex, &lids.upper_target, weights);
    let lower = angle(&lids.lower_flex, &lids.lower_target, weights);
    for (lid, a) in [(lids.upper_lid, upper), (lids.lower_lid, lower)] {
        if let Some(w) = usize::try_from(lid).ok().and_then(|d| weights.get_mut(d)) {
            *w = eye.up.dot(forward * (r * a.cos()) + up * (r * a.sin()));
        }
    }
}
/// Retail StudioRender planar iris basis, at default eyeball-size adjustment.
/// `bone` is the current bone-to-model transform, before entity scale/rotation.
pub fn projection(eye: &Eyeball, bone: glam::Mat4, target: Option<Vec3>) -> [glam::Vec4; 2] {
    let origin = bone.transform_point3(eye.origin);
    let (forward, up) = basis(eye, bone, target);
    let u = -forward.cross(up).normalize() * eye.iris_scale;
    let v = -up * eye.iris_scale;
    [u.extend(0.5 - u.dot(origin)), v.extend(0.5 - v.dot(origin))]
}
pub fn load(vfs: &Vfs, model: &str) -> Result<Vec<Eyeball>> {
    decode(&vfs.read(model)?.context("eyeball MDL absent")?)
}
fn count(data: &[u8], at: usize, limit: usize) -> Result<usize> {
    let n = usize::try_from(i32le(data, at)?)?;
    if n > limit {
        bail!("studio eyeball table exceeds limit at {at}");
    }
    Ok(n)
}
fn relative(data: &[u8], base: usize, at: usize) -> Result<usize> {
    Ok(usize::try_from(base as i64 + i32le(data, at)? as i64)?)
}
fn decode(data: &[u8]) -> Result<Vec<Eyeball>> {
    validate_header(data)?;
    let bones = count(data, 156, 4096)?;
    let bodies = count(data, 232, 256)?;
    let start = usize::try_from(i32le(data, 236)?)?;
    bytes(data, start, bodies * 16)?;
    let mut result = Vec::new();
    let mut surface = 0;
    for body in 0..bodies {
        let b = start + body * 16;
        if count(data, b + 4, 256)? == 0 {
            continue;
        }
        let m = relative(data, b, b + 12)?;
        bytes(data, m, 148)?;
        let meshes = count(data, m + 72, 4096)?;
        let mesh_start = relative(data, m, m + 76)?;
        bytes(data, mesh_start, meshes * 116)?;
        let eyes = count(data, m + 100, 32)?;
        let eye_start = relative(data, m, m + 104)?;
        bytes(data, eye_start, eyes * 172)?;
        for mesh in 0..meshes {
            let at = mesh_start + mesh * 116;
            if i32le(data, at + 24)? != 1 {
                continue;
            }
            let index = usize::try_from(i32le(data, at + 28)?)?;
            if index >= eyes {
                bail!("mesh eyeball index outside table");
            }
            let at = eye_start + index * 172;
            let eye = Eyeball {
                surface: surface + mesh,
                bone: usize::try_from(i32le(data, at + 4)?)?,
                origin: vec3(data, at + 8)?,
                z_offset: f32le(data, at + 20)?,
                radius: f32le(data, at + 24)?,
                up: vec3(data, at + 28)?,
                forward: vec3(data, at + 40)?,
                iris_scale: f32le(data, at + 60)?,
                lids: if bytes(data, at + 140, 1)?[0] != 0 {
                    None
                } else {
                    let ints = |o: usize| -> Result<[i32; 3]> {
                        Ok([i32le(data, o)?, i32le(data, o + 4)?, i32le(data, o + 8)?])
                    };
                    let floats = |o: usize| -> Result<[f32; 3]> {
                        Ok([f32le(data, o)?, f32le(data, o + 4)?, f32le(data, o + 8)?])
                    };
                    Some(EyeLids {
                        upper_flex: ints(at + 68)?,
                        lower_flex: ints(at + 80)?,
                        upper_target: floats(at + 92)?,
                        lower_target: floats(at + 104)?,
                        upper_lid: i32le(data, at + 116)?,
                        lower_lid: i32le(data, at + 120)?,
                    })
                },
            };
            if eye.bone >= bones
                || eye.radius <= 0.
                || eye.iris_scale <= 0.
                || eye.up.length_squared() < 0.5
                || eye.forward.length_squared() < 0.5
                || eye.up.cross(eye.forward).length_squared() < 0.25
            {
                bail!("invalid studio eyeball basis/bone/scalars");
            }
            result.push(eye);
            if result.len() > 512 {
                bail!("studio eyeball budget exceeded");
            }
        }
        surface += meshes;
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<u8> {
        let mut d = vec![0; 800];
        d[..4].copy_from_slice(b"IDST");
        for (at, n) in [
            (4, 48i32),
            (76, 800),
            (156, 1),
            (232, 1),
            (236, 240),
            (244, 1),
            (252, 16),
            (328, 1),
            (332, 148),
            (356, 1),
            (360, 264),
            (428, 1),
        ] {
            d[at..at + 4].copy_from_slice(&n.to_le_bytes());
        }
        // body +240, model +256, mesh +404, eyeball +520.
        for (at, v) in [(544, 0.5f32), (548, 1.), (564, 1.), (580, 1.5)] {
            d[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        d
    }
    fn gaze_fixture() -> Vec<u8> {
        let mut d = vec![0u8; 500];
        d[..4].copy_from_slice(b"IDST");
        for (at, value) in [
            (4, 48i32),
            (76, 500),
            (156, 1),
            (240, 1),
            (244, 300),
            (300, 100),
        ] {
            d[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        d[400..405].copy_from_slice(b"eyes\0");
        for (at, value) in [
            (88, 70f32),
            (312, 1.),
            (332, 1.),
            (352, 1.),
            (324, 4.),
            (340, 5.),
            (356, 6.),
        ] {
            d[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        d
    }
    #[test]
    fn facs_eyelids_follow_weighted_targets_and_gaze() {
        let eye = Eyeball {
            surface: 0,
            bone: 0,
            origin: Vec3::ZERO,
            up: Vec3::Z,
            forward: -Vec3::X,
            radius: 0.5,
            z_offset: 0.,
            iris_scale: 1.,
            lids: Some(EyeLids {
                upper_flex: [0, 1, 2],
                lower_flex: [3, 4, 5],
                upper_target: [0.3, 0.2, -0.25],
                lower_target: [-0.1, -0.25, -0.3],
                upper_lid: 6,
                lower_lid: 7,
            }),
        };
        // Neutral weights, looking straight ahead: lids sit at the neutral targets.
        let mut w = [0., 1., 0., 0., 1., 0., 0., 0.];
        apply_eyelids(&eye, &mut w, rest_basis(&eye));
        assert!(
            (w[6] - 0.2).abs() < 1e-5 && (w[7] + 0.25).abs() < 1e-5,
            "{w:?}"
        );
        // Looking up raises the lid point's authored-up component.
        let (f, u) = (
            Vec3::new(1., 0., 0.3).normalize(),
            Vec3::new(-0.3, 0., 1.).normalize(),
        );
        let mut up = [0., 1., 0., 0., 1., 0., 0., 0.];
        apply_eyelids(&eye, &mut up, (f, u));
        assert!(up[6] > w[6] && up[7] > w[7]);
        // Non-FACS eyeballs leave descriptors alone.
        let mut untouched = w;
        apply_eyelids(&Eyeball { lids: None, ..eye }, &mut untouched, (f, u));
        assert_eq!(untouched, w);
    }
    #[test]
    fn attachment_basis_fallback_and_malformed_records_are_distinguished() {
        let d = gaze_fixture();
        let gaze = decode_gaze(&d).unwrap();
        assert_eq!(gaze.view_offset, Vec3::new(0., 0., 70.));
        let a = gaze.attachment.unwrap();
        assert_eq!(a.bone, 0);
        assert_eq!(a.local.transform_point3(Vec3::ZERO), Vec3::new(4., 5., 6.));
        assert!(!a.world_align);
        for (at, n) in [
            (244, 490u32),
            (308, 1),
            (300, 1000),
            (316, 0x7fc00000),
            (304, 1),
            (312, 0),
        ] {
            let mut bad = d.clone();
            bad[at..at + 4].copy_from_slice(&n.to_le_bytes());
            assert!(decode_gaze(&bad).is_err(), "offset {at}");
        }
        let mut overflow = d.clone();
        for at in [312, 332] {
            overflow[at..at + 4].copy_from_slice(&f32::MAX.to_le_bytes());
        }
        assert!(decode_gaze(&overflow).is_err());
        let mut missing = d.clone();
        missing[400..405].copy_from_slice(b"hand\0");
        assert!(decode_gaze(&missing).unwrap().attachment.is_none());
        missing[240..244].copy_from_slice(&0i32.to_le_bytes());
        assert!(decode_gaze(&missing).unwrap().attachment.is_none());
    }
    #[test]
    fn max_eye_deflection_decodes_studiohdr2_and_clamps_view_target_cone() {
        // Synthetic IDST header >= 408 bytes without studiohdr2 falls back to cos(30 deg).
        let mut d = vec![0u8; 512];
        d[..4].copy_from_slice(b"IDST");
        d[4..8].copy_from_slice(&48i32.to_le_bytes());
        d[76..80].copy_from_slice(&512i32.to_le_bytes());
        assert_eq!(
            decode_gaze(&d).unwrap().max_eye_deflection,
            DEFAULT_MAX_EYE_DEFLECTION
        );
        // Valid studiohdr2index at 408 with zero flMaxEyeDeflection still uses default 0.866.
        d[400..404].copy_from_slice(&408i32.to_le_bytes());
        assert_eq!(
            decode_gaze(&d).unwrap().max_eye_deflection,
            DEFAULT_MAX_EYE_DEFLECTION
        );
        // Custom flMaxEyeDeflection at hdr2 + 12 (byte 420) is honored.
        d[420..424].copy_from_slice(&0.707f32.to_le_bytes());
        assert!((decode_gaze(&d).unwrap().max_eye_deflection - 0.707).abs() < 1e-6);

        let origin = Vec3::new(10., 20., 30.);
        let forward = Vec3::X;
        // Within 30 deg: left unchanged when >= 6 units ahead.
        let within = origin + Vec3::new(100., 20., 10.);
        assert!((clamp_view_target(origin, forward, within, 0.866) - within).length() < 1e-4);
        // Wide 60/75 deg and behind targets are clamped onto the max_deflection cone.
        for wide in [
            origin + Vec3::new(30., 80., 0.),
            origin + Vec3::new(-40., 60., 15.),
        ] {
            let clamped = clamp_view_target(origin, forward, wide, 0.866);
            let dir = (clamped - origin).normalize();
            assert!((dir.dot(forward) - 0.866).abs() < 1e-4, "{dir:?}");
            assert!(clamped.y > origin.y);
        }
        // Collinear target straight behind clamps forward cleanly at depth >= 6.
        let behind = clamp_view_target(origin, forward, origin - forward * 50., 0.866);
        assert!((behind - (origin + forward * 6.)).length() < 1e-4);
    }

    #[test]
    #[ignore = "requires owned installed HL2 actor MDLs"]
    fn owned_actor_gaze_attachments_match_bone_skeletons() {
        let vfs = Vfs::mount(&crate::install::discover().unwrap()).unwrap();
        for model in [
            "models/barney.mdl",
            "models/kleiner.mdl",
            "models/humans/group01/male_07.mdl",
            "models/gman_high.mdl",
            "models/police.mdl",
        ] {
            let gaze = load_gaze(&vfs, model).unwrap();
            assert!(
                gaze.max_eye_deflection.is_finite()
                    && (0.5..=1.0).contains(&gaze.max_eye_deflection),
                "{model}: bad max_eye_deflection {}",
                gaze.max_eye_deflection
            );
            let attachment = gaze.attachment.expect(model);
            let rig = crate::animation::load(&vfs, model, &["idle_subtle".into()].into()).unwrap();
            assert!(attachment.bone < rig.bones.len(), "{model}");
            let matrix = rig.matrices("idle_subtle", 0.)[attachment.bone]
                * rig.bones[attachment.bone].inverse_bind.inverse()
                * attachment.local;
            assert!(matrix.is_finite());
            assert!(
                (matrix.transform_vector3(Vec3::X).length() - 1.).abs() < 1e-3,
                "{model}"
            );
        }
    }
    #[test]
    fn valid_eye_and_malformed_tables_are_distinguished() {
        let d = fixture();
        let eyes = decode(&d).unwrap();
        assert_eq!(eyes.len(), 1);
        assert_eq!(eyes[0].surface, 0);
        assert_eq!(eyes[0].up, Vec3::X);
        assert_eq!(eyes[0].forward, Vec3::Y);
        for (at, n) in [
            (532, 0x7fc00000u32),
            (524, 1),
            (432, 4),
            (356, 33),
            (360, 10000),
        ] {
            let mut bad = d.clone();
            bad[at..at + 4].copy_from_slice(&n.to_le_bytes());
            assert!(decode(&bad).is_err());
        }
        assert!(decode(&d[..799]).is_err());
    }
    #[test]
    fn planar_basis_centers_eye_tracks_target_and_preserves_bone_transform() {
        let eye = decode(&fixture()).unwrap().remove(0);
        let rows = projection(&eye, glam::Mat4::IDENTITY, None);
        assert_eq!(rows[0].dot(eye.origin.extend(1.)), 0.5);
        assert_eq!(rows[1].dot(eye.origin.extend(1.)), 0.5);
        assert!((rows[0].truncate().length() - eye.iris_scale).abs() < 1e-6);
        assert!(rows[0].truncate().dot(rows[1].truncate()).abs() < 1e-6);
        let bone = glam::Mat4::from_rotation_translation(
            glam::Quat::from_rotation_z(0.7),
            Vec3::new(10., 3., -9.),
        );
        let transformed = projection(&eye, bone, None);
        for p in [Vec3::ZERO, Vec3::Z * 0.3, Vec3::X * 0.4] {
            for i in 0..2 {
                assert!(
                    (rows[i].dot(p.extend(1.))
                        - transformed[i].dot(bone.transform_point3(p).extend(1.)))
                    .abs()
                        < 2e-6
                );
            }
        }
        let tracked = projection(&eye, glam::Mat4::IDENTITY, Some(Vec3::new(0., -20., 5.)));
        assert_ne!(tracked[0], rows[0]);
        assert_eq!(
            projection(&eye, glam::Mat4::IDENTITY, Some(eye.origin)),
            rows
        );
    }

    #[test]
    #[ignore = "requires owned installed HL2"]
    fn owned_eyes_match_mesh_materials_and_bone_projection() {
        let vfs = Vfs::mount(&crate::install::discover().unwrap()).unwrap();
        for model in [
            "models/barney.mdl",
            "models/humans/group01/male_07.mdl",
            "models/gman_high.mdl",
            "models/vortigaunt.mdl",
        ] {
            let eyes = load(&vfs, model).unwrap();
            assert!(eyes.len() >= 2, "{model}");
            let meshes = crate::models::read_model(&vfs, model, 0).unwrap();
            let rig =
                crate::animation::load(&vfs, model, &std::collections::BTreeSet::new()).unwrap();
            for eye in eyes {
                assert!(meshes[eye.surface].material.to_lowercase().contains("eye"));
                let bone = rig.bones[eye.bone].inverse_bind.inverse();
                assert!(projection(&eye, bone, None).iter().all(|r| r.is_finite()));
            }
        }
    }
}
