pub mod animation;
pub mod bsp;
pub mod details;
pub mod install;
pub mod keyvalues;
pub mod lighting;
pub mod model_lighting;
pub mod models;
pub mod monitor_material;
pub mod navigation;
pub mod overlays;
pub mod phy;
pub mod proximity_material;
pub mod scenes;
pub mod sky;
pub mod sounds;
pub mod visibility;
pub mod vpk;
pub mod vtf;

use anyhow::{bail, Result};
pub(crate) fn bytes(data: &[u8], offset: usize, len: usize) -> Result<&[u8]> {
    data.get(
        offset
            ..offset
                .checked_add(len)
                .ok_or_else(|| anyhow::anyhow!("offset overflow"))?,
    )
    .ok_or_else(|| anyhow::anyhow!("truncated data at {offset} for {len} bytes"))
}
pub(crate) fn u16le(d: &[u8], o: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(bytes(d, o, 2)?.try_into()?))
}
pub(crate) fn i16le(d: &[u8], o: usize) -> Result<i16> {
    Ok(i16::from_le_bytes(bytes(d, o, 2)?.try_into()?))
}
pub(crate) fn u32le(d: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(bytes(d, o, 4)?.try_into()?))
}
pub(crate) fn i32le(d: &[u8], o: usize) -> Result<i32> {
    Ok(i32::from_le_bytes(bytes(d, o, 4)?.try_into()?))
}
pub(crate) fn f32le(d: &[u8], o: usize) -> Result<f32> {
    let x = f32::from_le_bytes(bytes(d, o, 4)?.try_into()?);
    if !x.is_finite() {
        bail!("non-finite float at {o}")
    }
    Ok(x)
}
pub(crate) fn vec3(d: &[u8], o: usize) -> Result<glam::Vec3> {
    Ok(glam::Vec3::new(
        f32le(d, o)?,
        f32le(d, o + 4)?,
        f32le(d, o + 8)?,
    ))
}
pub(crate) fn index(value: i32, len: usize) -> Result<usize> {
    let n = usize::try_from(value)?;
    if n >= len {
        bail!("index {n} exceeds {len}")
    }
    Ok(n)
}
pub(crate) fn records(d: &[u8], size: usize) -> Result<impl Iterator<Item = &[u8]>> {
    if !d.len().is_multiple_of(size) {
        bail!("invalid record size {size}: {} bytes", d.len())
    }
    Ok(d.chunks_exact(size))
}

pub mod eyes;
pub mod flexes;
pub mod sentence;
