//! Projectile and gib models read for effects, posed at their first sequence.
use modkit_core::Surface;
use source_assets::{models, vpk::Vfs};

/// One projectile/gib model posed at sequence 0 frame 0 (setup or worker threads).
#[allow(dead_code)]
pub fn read_effect_model(vfs: &Vfs, path: &str, skin: i32) -> anyhow::Result<Vec<Surface>> {
    read_effect_model_lod(vfs, path, skin, 0)
}

/// One projectile/gib model posed at sequence 0 frame 0 with specified root LOD.
pub fn read_effect_model_lod(
    vfs: &Vfs,
    path: &str,
    skin: i32,
    root_lod: usize,
) -> anyhow::Result<Vec<Surface>> {
    let mut surfaces = models::read_model_lod(vfs, path, skin.max(0) as usize, root_lod)?;
    pose_sequence_zero(vfs, path, &mut surfaces)?;
    Ok(surfaces)
}

/// Studio models draw through their bone pose, not the VVD reference pose: projectile
/// entities never set a sequence, so sequence 0 frame 0 applies (crossbow_bolt.mdl and
/// w_missile*.mdl point along +x only after their root-bone animation). Setup only.
fn pose_sequence_zero(vfs: &Vfs, path: &str, surfaces: &mut [Surface]) -> anyhow::Result<()> {
    use anyhow::Context;
    let Some(first) = source_assets::animation::first_sequence_name(vfs, path)? else {
        return Ok(());
    };
    let rig = source_assets::animation::load(vfs, path, &[first.clone()].into())?;
    let matrices = rig.matrices(&first, 0.);
    for vertex in surfaces.iter_mut().flat_map(|s| s.vertices.iter_mut()) {
        let weights = vertex.skin.as_ref();
        let matrix = match weights {
            Some(w) => {
                let mut blended = glam::Mat4::ZERO;
                for (&bone, &weight) in w.bones.iter().zip(&w.weights) {
                    blended += *matrices
                        .get(bone as usize)
                        .context("vertex bone out of range")?
                        * weight;
                }
                blended
            }
            None => matrices.first().copied().unwrap_or(glam::Mat4::IDENTITY),
        };
        vertex.position = matrix.transform_point3(vertex.position);
        vertex.normal = matrix.transform_vector3(vertex.normal).normalize_or_zero();
    }
    Ok(())
}
