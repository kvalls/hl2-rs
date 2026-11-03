//! Independent visual resources prepared alongside map material decoding.
use super::{faces, load_sky, work::Cancellation};
use anyhow::Result;
use modkit_core::World;
use source_assets::{sky::Skybox, vpk::Vfs, vtf::Image};
use std::{collections::BTreeMap, sync::Arc, time::Instant};

pub(super) struct Auxiliary {
    pub hud: hl2_ui::hud::WeaponHud,
    pub faces: faces::Faces,
    pub sky: Option<Skybox>,
    pub cookie: Option<Arc<Image>>,
    pub errors: Vec<String>,
    pub timings: BTreeMap<String, f64>,
}
pub(super) fn load(
    vfs: &Vfs,
    world: &World,
    canvas: source_ui::canvas::Canvas,
    hdr: bool,
    cancel: &Cancellation,
) -> Result<Auxiliary> {
    let started = Instant::now();
    cancel.check()?;
    let hud = hl2_ui::hud::WeaponHud::load(vfs, canvas)?;
    let hud_ms = started.elapsed().as_secs_f64() * 1000.;
    cancel.check()?;
    let started = Instant::now();
    let mut errors = Vec::new();
    let faces = faces::load(vfs, world, &mut errors);
    let faces_ms = started.elapsed().as_secs_f64() * 1000.;
    cancel.check()?;
    let started = Instant::now();
    let sky = load_sky(vfs, world, hdr, &mut errors);
    let sky_ms = started.elapsed().as_secs_f64() * 1000.;
    cancel.check()?;
    let cookie = crate::flashlight::load_cookie(vfs).map(Arc::new);
    Ok(Auxiliary {
        hud,
        faces,
        sky,
        cookie,
        errors,
        timings: [
            ("aux_hud_ms".into(), hud_ms),
            ("aux_faces_ms".into(), faces_ms),
            ("aux_sky_ms".into(), sky_ms),
        ]
        .into(),
    })
}
