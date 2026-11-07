//! The map's cubemap samples, for models (DESIGN 14o session 48). `$envmap env_cubemap` on a
//! VertexLitGeneric material means a sample near the model. Each sample's texture is named after
//! where it was built and lives in the map's pak, HDR first, as the engine's loader reads it.
use anyhow::{Context, Result};
use glam::Vec3;
use source_assets::{bsp::Bsp, vpk::Vfs, vtf};
use std::sync::Arc;

/// Cubemap faces are small; this only bounds a bad file.
const MAX_DIMENSION: usize = 1024;

/// One decoded sample.
pub struct Sample {
    pub origin: Vec3,
    pub path: String,
    pub cube: Arc<vtf::Cube>,
}

/// The texture of a sample, by the name the engine builds from its origin.
fn texture_path(map: &str, origin: [i32; 3], suffix: &str) -> String {
    let [x, y, z] = origin;
    format!("materials/maps/{map}/c{x}_{y}_{z}{suffix}.vtf")
}

fn read_sample(
    vfs: &Vfs,
    map: &str,
    origin: [i32; 3],
    hdr: bool,
) -> Result<Option<(String, Vec<u8>)>> {
    if hdr {
        let path = texture_path(map, origin, ".hdr");
        if let Some(data) = vfs.read(&path)? {
            return Ok(Some((path, data)));
        }
    }
    let path = texture_path(map, origin, "");
    Ok(vfs.read(&path)?.map(|data| (path, data)))
}

fn decode_sample(
    vfs: &Vfs,
    map: &str,
    origin: [i32; 3],
    (picmip, hdr): (i32, bool),
) -> Result<Option<Sample>> {
    let Some((path, data)) = read_sample(vfs, map, origin, hdr)? else {
        return Ok(None);
    };
    let cube = vtf::decode_cube_picmip(&data, MAX_DIMENSION, picmip)
        .with_context(|| format!("decode {path}"))?;
    Ok(Some(Sample {
        origin: Vec3::from_array(origin.map(|v| v as f32)),
        path,
        cube: Arc::new(cube),
    }))
}

/// Every sample the map lists. One that is absent or broken is left out and reported, since a
/// model then only reflects a farther sample.
pub fn load(vfs: &Vfs, map: &str, bsp: &Bsp, quality: (i32, bool)) -> (Vec<Sample>, Vec<String>) {
    let mut samples = Vec::new();
    let mut errors = Vec::new();
    for origin in bsp.cubemap_origins() {
        match decode_sample(vfs, map, origin, quality) {
            Ok(Some(sample)) => samples.push(sample),
            Ok(None) => errors.push(format!("cubemap sample absent: {origin:?}")),
            Err(error) => errors.push(format!("{error:#}")),
        }
    }
    (samples, errors)
}

/// The index of the sample nearest `point`, as the retail light cache chooses when it fills an
/// entry. Equal distances keep the earlier sample, as its strict comparison does.
pub fn nearest(origins: impl Iterator<Item = Vec3>, point: Vec3) -> Option<usize> {
    origins
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            a.distance_squared(point)
                .total_cmp(&b.distance_squared(point))
        })
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sample_is_named_after_its_origin_negative_values_included() {
        assert_eq!(
            texture_path("d1_canals_02", [-12, 340, -5], ".hdr"),
            "materials/maps/d1_canals_02/c-12_340_-5.hdr.vtf"
        );
    }

    #[test]
    fn the_nearest_sample_wins_and_none_means_none() {
        let origins = [Vec3::ZERO, Vec3::X * 100., Vec3::X * 300.];
        assert_eq!(nearest(origins.into_iter(), Vec3::X * 180.), Some(1));
        assert_eq!(nearest(std::iter::empty(), Vec3::ZERO), None);
    }
}
