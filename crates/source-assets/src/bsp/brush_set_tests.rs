//! Installed brushes exercise the portable index without adding an asset dependency to core.
use super::Bsp;
use glam::Vec3;
use modkit_core::{trace_brushes, BrushSet, Trace};

struct Sequence(u64);
impl Sequence {
    fn point(&mut self) -> Vec3 {
        Vec3::from_array(std::array::from_fn(|_| {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
            (self.0 >> 40) as f32 / (1u64 << 24) as f32
        }))
    }
}

fn assert_same(expected: Trace, actual: Trace, map: &str, case: usize) {
    assert_eq!(
        expected.fraction.to_bits(),
        actual.fraction.to_bits(),
        "{map} {case}"
    );
    assert_eq!(
        expected.normal.to_array().map(f32::to_bits),
        actual.normal.to_array().map(f32::to_bits),
        "{map} {case}"
    );
    assert_eq!(expected.start_solid, actual.start_solid, "{map} {case}");
    assert_eq!(expected.all_solid, actual.all_solid, "{map} {case}");
}

#[test]
#[ignore = "requires owned HL2 installation"]
fn owned_campaign_brush_set_matches_ordered_full_scan() {
    let vfs = crate::vpk::Vfs::mount(&crate::install::discover().expect("owned install")).unwrap();
    for map in ["d1_trainstation_02", "d1_canals_01", "d2_coast_01"] {
        let bsp = Bsp::parse(&vfs.read(&format!("maps/{map}.bsp")).unwrap().unwrap()).unwrap();
        let world = bsp.world(map).unwrap();
        let spawn = world.spawn().0;
        let set = BrushSet::new(world.brushes);
        let mut values = Sequence(512);
        let (mut candidates, mut fallback, mut short_candidates, mut short_fallback) = (0, 0, 0, 0);
        let (mut hits, mut solid) = (0, 0);
        for case in 0..6000 {
            // Local movement around spawn, map-wide probes, and deliberately long sweeps.
            let center = if case % 2 == 0 { spawn } else { Vec3::ZERO };
            let extent = if case % 2 == 0 { 1500. } else { 16000. };
            let start = center + (values.point() - 0.5) * extent;
            let reach = [0., 0.03125, 2., 30., 400., 32000.][case % 6];
            let end = start + (values.point() - 0.5) * reach;
            let (mins, maxs) = match case % 3 {
                0 => (Vec3::ZERO, Vec3::ZERO),
                1 => (Vec3::new(-16., -16., 0.), Vec3::new(16., 16., 72.)),
                _ => (-Vec3::splat(8.), Vec3::splat(8.)),
            };
            let mask = [1, 0x1400b, 0x200400b, u32::MAX][case % 4];
            let expected = trace_brushes(set.brushes(), start, end, mins, maxs, mask);
            assert_same(expected, set.trace(start, end, mins, maxs, mask), map, case);
            hits += usize::from(expected.fraction < 1.);
            solid += usize::from(expected.start_solid);
            let count = set.candidate_count(start, end, mins, maxs);
            candidates += count.unwrap_or(set.brushes().len());
            fallback += usize::from(count.is_none());
            if reach <= 30. {
                short_candidates += count.unwrap_or(set.brushes().len());
                short_fallback += usize::from(count.is_none());
            }
        }
        assert!(hits > 0 && solid > 0, "{map}: no hit/solid cases exercised");
        eprintln!(
            "{map}: brushes={} unbounded={} sweeps=6000 mean_candidates={:.2} \
            full_scan_fallbacks={fallback} short_sweeps=4000 short_mean_candidates={:.2} \
            short_fallbacks={short_fallback} hits={hits} start_solid={solid}",
            set.brushes().len(),
            set.unbounded_count(),
            candidates as f64 / 6000.,
            short_candidates as f64 / 4000.
        );
    }
}
