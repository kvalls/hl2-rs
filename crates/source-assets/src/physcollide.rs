//! The map's PHYSCOLLIDE lump (29): for each brush model, the compiled VPhysics solids
//! and their KeyValues text (dphysmodel_t: modelIndex, dataSize, keydataSize,
//! solidCount; the solids' bytes, then the text; a modelIndex of -1 ends the lump).

use crate::keyvalues;
use anyhow::{Context, Result};
use std::collections::BTreeMap;

/// The first "solid" block's parameters per brush model index (the solid a brush
/// entity's VPhysics object is made from: CPhysBox::CreateVPhysics reads them through
/// PhysModelParseSolid). The text's mass is the compiled solid's volume times its
/// surfaceprop's density (d1_trainstation_01's turnstile, model 61: mass 35.188103,
/// surfaceprop metal, volume 1816). A model whose text has no solid block is absent.
pub fn brush_model_solids(lump: &[u8]) -> Result<BTreeMap<usize, modkit_core::PhysicsSolid>> {
    let mut solids = BTreeMap::new();
    let mut at = 0usize;
    while at + 16 <= lump.len() {
        let model = crate::i32le(lump, at)?;
        if model < 0 {
            break;
        }
        let size = usize::try_from(crate::i32le(lump, at + 4)?).context("negative data size")?;
        let keys = usize::try_from(crate::i32le(lump, at + 8)?).context("negative text size")?;
        let text_at = at + 16 + size;
        let end = text_at
            .checked_add(keys)
            .filter(|end| *end <= lump.len())
            .context("truncated PHYSCOLLIDE model")?;
        let text = &lump[text_at..end];
        let nul = text.iter().position(|b| *b == 0).unwrap_or(text.len());
        let text = std::str::from_utf8(&text[..nul]).context("non-UTF-8 PHYSCOLLIDE text")?;
        if let Some(mut solid) = crate::phy::first_solid(&keyvalues::parse(text)?) {
            solid.drag_fractions = drag_fractions(&lump[at + 16..text_at])?;
            solids.insert(model as usize, solid);
        }
        at = end;
    }
    Ok(solids)
}

/// The first solid's VPHY header drag fractions (the same layout as a .phy solid: its
/// size, then "VPHY", version 0x100, and the stored orthographic area fractions at 12),
/// which retail vphysics copies for the drag bases. vbsp stores zeros (the turnstile's
/// solid: 0 0 0), so brush solids have no air drag. None for an older solid format.
fn drag_fractions(solids: &[u8]) -> Result<Option<glam::Vec3>> {
    if solids.len() < 4 + 24 || &solids[4..8] != b"VPHY" || crate::u16le(solids, 8)? != 0x100 {
        return Ok(None);
    }
    Ok(Some(crate::vec3(solids, 16)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(model: i32, solid_bytes: usize, text: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for field in [model, solid_bytes as i32, text.len() as i32, 1] {
            out.extend_from_slice(&field.to_le_bytes());
        }
        let mut solid = vec![0xAB; solid_bytes];
        if solid_bytes >= 32 {
            // size, "VPHY", version 0x100, model type, surface size, drag fractions.
            solid[4..8].copy_from_slice(b"VPHY");
            solid[8..10].copy_from_slice(&0x100u16.to_le_bytes());
            for (i, v) in [0.25f32, 0.5, 0.].iter().enumerate() {
                solid[16 + 4 * i..20 + 4 * i].copy_from_slice(&v.to_le_bytes());
            }
        }
        out.extend(solid);
        out.extend_from_slice(text.as_bytes());
        out
    }

    #[test]
    fn reads_each_models_first_solid_until_the_end_marker() {
        let mut lump = record(
            61,
            32,
            "solid {\n\"index\" \"0\"\n\"mass\" \"35.188103\"\n\"surfaceprop\" \"metal\"\n\"volume\" \"1815.999634\"\n}\n\0",
        );
        lump.extend(record(3, 4, "solid {\n\"mass\" \"2\"\n}\n"));
        lump.extend(record(-1, 0, ""));
        lump.extend(record(9, 0, "solid { \"mass\" \"5\" }"));
        let solids = brush_model_solids(&lump).unwrap();
        assert_eq!(solids.keys().copied().collect::<Vec<_>>(), [3, 61]);
        assert!((solids[&61].mass - 35.188103).abs() < 1e-4);
        assert_eq!(solids[&61].surfaceprop, "metal");
        assert!((solids[&61].volume - 1816.).abs() < 1e-2);
        assert_eq!(solids[&3].mass, 2.);
        assert_eq!(solids[&3].surfaceprop, "default");
        assert_eq!(
            solids[&61].drag_fractions,
            Some(glam::Vec3::new(0.25, 0.5, 0.))
        );
        assert_eq!(solids[&3].drag_fractions, None);
    }

    #[test]
    fn a_truncated_record_is_an_error() {
        let mut lump = record(1, 8, "solid { }");
        lump.truncate(lump.len() - 3);
        assert!(brush_model_solids(&lump).is_err());
    }
}
