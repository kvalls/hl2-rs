//! Isolated physics-step benchmark on owned `d1_trainstation_02`.

use super::super::Physics;
use glam::Vec3;
use modkit_core::movement::TICK;
use source_assets::{bsp::Bsp, install, models, vpk::Vfs};
use std::time::Instant;

fn report_stats(label: &str, samples: &[f64]) {
    if samples.is_empty() {
        println!("{label}: empty");
        return;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let min = sorted[0];
    let max = sorted[sorted.len() - 1];
    let median = sorted[sorted.len() / 2];
    let mean = sorted.iter().sum::<f64>() / sorted.len() as f64;
    let p95 = sorted[(sorted.len().saturating_sub(1) * 95) / 100];
    println!(
        "{label}: n={} min={min:.3}ms median={median:.3}ms mean={mean:.3}ms p95={p95:.3}ms max={max:.3}ms",
        sorted.len()
    );
}

fn measure_ticks(physics: &mut Physics, count: usize) -> (Vec<f64>, Vec<f64>, usize, usize) {
    let mut wall_ms = Vec::with_capacity(count);
    let mut step_ms = Vec::with_capacity(count);
    let mut max_awake = 0usize;
    for _ in 0..count {
        let started = Instant::now();
        physics.tick(TICK);
        wall_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        step_ms.push(physics.last_step_ms);
        max_awake = max_awake.max(physics.awake_bodies());
    }
    (wall_ms, step_ms, max_awake, physics.awake_bodies())
}

#[ignore = "isolated pure-Rust physics benchmark on owned d1_trainstation_02; run release with --ignored --nocapture"]
#[test]
fn benchmark_station02_physics_step() {
    let root = install::discover().expect("owned Half-Life 2 installation required");
    let mut vfs = Vfs::mount(&root).expect("mount HL2 VFS");
    let bytes = vfs
        .read("maps/d1_trainstation_02.bsp")
        .expect("read maps/d1_trainstation_02.bsp")
        .expect("maps/d1_trainstation_02.bsp missing from VFS");
    let bsp = Bsp::parse(&bytes).expect("parse maps/d1_trainstation_02.bsp");
    vfs.mount_pak(bsp.lump(40)).expect("mount BSP pakfile");
    let mut world = bsp
        .world("d1_trainstation_02")
        .expect("build d1_trainstation_02 World");
    models::append_models(&mut world, &vfs);

    let mut physics = Physics::new(&world);
    let dynamic_ids: Vec<usize> = physics.dynamic.keys().copied().collect();

    for _ in 0..60 {
        physics.tick(TICK);
    }

    let (rest_wall, rest_step, rest_max_awake, rest_end_awake) = measure_ticks(&mut physics, 300);
    println!(
        "resting: dynamic_bodies={} awake_max={rest_max_awake} awake_end={rest_end_awake}",
        dynamic_ids.len()
    );
    report_stats("resting wall", &rest_wall);
    report_stats("resting last_step_ms", &rest_step);

    let dir = Vec3::new(1.0, 0.5, 2.0).normalize();
    for &id in &dynamic_ids {
        physics.impulse(id, dir, 12.0);
    }

    let (active_wall, active_step, active_max_awake, active_end_awake) =
        measure_ticks(&mut physics, 300);
    println!(
        "active: dynamic_bodies={} awake_max={active_max_awake} awake_end={active_end_awake}",
        dynamic_ids.len()
    );
    report_stats("active wall", &active_wall);
    report_stats("active last_step_ms", &active_step);
}
