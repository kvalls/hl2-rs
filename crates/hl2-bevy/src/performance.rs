//! Optional bounded CPU stage measurements; disabled for ordinary play.
use bevy::prelude::*;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Instant,
};
#[derive(Resource, Default, Clone)]
pub struct Performance(Arc<Mutex<BTreeMap<&'static str, Samples>>>);
#[derive(Default)]
struct Samples {
    calls: u64,
    values: Vec<f64>,
}
pub struct Scope<'a> {
    performance: Option<&'a Performance>,
    name: &'static str,
    start: Instant,
}
pub fn scope<'a>(performance: Option<&'a Performance>, name: &'static str) -> Scope<'a> {
    Scope {
        performance,
        name,
        start: Instant::now(),
    }
}
impl Drop for Scope<'_> {
    fn drop(&mut self) {
        let Some(performance) = self.performance else {
            return;
        };
        let mut all = performance.0.lock().expect("CPU timing");
        let samples = all.entry(self.name).or_default();
        samples.calls += 1;
        if samples.calls > 30 && samples.values.len() < 4096 {
            samples
                .values
                .push(self.start.elapsed().as_secs_f64() * 1000.);
        }
    }
}
impl Performance {
    pub fn report(&self) -> serde_json::Value {
        let all = self.0.lock().expect("CPU timing");
        serde_json::Value::Object(all.iter().map(|(name,samples)| {
            let mut sorted=samples.values.clone();sorted.sort_by(f64::total_cmp);
            let mean=sorted.iter().sum::<f64>()/sorted.len().max(1) as f64;
            let p95=sorted.get(sorted.len().saturating_sub(1)*95/100).copied().unwrap_or(0.);
            // The slowest samples with their sample index (frame hitches, e.g. a map copy).
            let mut slowest:Vec<(usize,f64)>=samples.values.iter().copied().enumerate().collect();
            slowest.sort_by(|a,b| b.1.total_cmp(&a.1));slowest.truncate(3);
            ((*name).into(),serde_json::json!({"calls":samples.calls,"samples":sorted.len(),"mean_ms":mean,"p95_ms":p95,"max_ms":sorted.last().copied().unwrap_or(0.),"slowest":slowest}))
        }).collect())
    }
}

/// Bevy's render CPU/GPU timings and pipeline counters, with their original units.
pub fn render_report(diagnostics: &bevy::diagnostic::DiagnosticsStore) -> serde_json::Value {
    serde_json::Value::Object(
        diagnostics
            .iter()
            .map(|d| {
                (
                    d.path().to_string(),
                    serde_json::json!({"mean":d.average(),"latest":d.value(),"unit":d.suffix}),
                )
            })
            .collect(),
    )
}

#[derive(Resource, Default)]
struct RenderTimers(BTreeMap<&'static str, Instant>);
/// Optional render-stage CPU wall timings. Ordering fences exist only with --profile.
pub fn install(app: &mut App) {
    use bevy::render::{Render, RenderApp, RenderSystems};
    let performance = Performance::default();
    app.insert_resource(performance.clone())
        .add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin);
    let Some(render) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render
        .insert_resource(performance)
        .init_resource::<RenderTimers>();
    for (name, previous, set, next) in [
        (
            "render_assets",
            RenderSystems::ExtractCommands,
            RenderSystems::PrepareAssets,
            RenderSystems::PrepareMeshes,
        ),
        (
            "render_meshes",
            RenderSystems::PrepareAssets,
            RenderSystems::PrepareMeshes,
            RenderSystems::CreateViews,
        ),
        (
            "render_views",
            RenderSystems::Specialize,
            RenderSystems::PrepareViews,
            RenderSystems::Queue,
        ),
        (
            "render_queue",
            RenderSystems::PrepareViews,
            RenderSystems::Queue,
            RenderSystems::PhaseSort,
        ),
        (
            "render_prepare",
            RenderSystems::PhaseSort,
            RenderSystems::Prepare,
            RenderSystems::Render,
        ),
        (
            "render_graph",
            RenderSystems::Prepare,
            RenderSystems::Render,
            RenderSystems::Cleanup,
        ),
    ] {
        render.add_systems(
            Render,
            (
                (move |mut timers: ResMut<RenderTimers>| {
                    timers.0.insert(name, Instant::now());
                })
                .after(previous)
                .before(set.clone()),
                (move |mut timers: ResMut<RenderTimers>, performance: Res<Performance>| {
                    if let Some(start) = timers.0.remove(name) {
                        drop(Scope {
                            performance: Some(&performance),
                            name,
                            start,
                        });
                    }
                })
                .after(set)
                .before(next),
            ),
        );
    }
    // Subsets of Prepare are fenced inside their parent, so these intervals
    // measure only their own stage rather than accumulating previous work.
    for (name, previous, set, next) in [
        (
            "render_resource_setup",
            None,
            RenderSystems::PrepareResources,
            Some(RenderSystems::PrepareResourcesBatchPhases),
        ),
        (
            "render_batch",
            Some(RenderSystems::PrepareResources),
            RenderSystems::PrepareResourcesBatchPhases,
            Some(RenderSystems::PrepareResourcesWritePhaseBuffers),
        ),
        (
            "render_write",
            Some(RenderSystems::PrepareResourcesBatchPhases),
            RenderSystems::PrepareResourcesWritePhaseBuffers,
            Some(RenderSystems::PrepareResourcesCollectPhaseBuffers),
        ),
        (
            "render_collect",
            Some(RenderSystems::PrepareResourcesWritePhaseBuffers),
            RenderSystems::PrepareResourcesCollectPhaseBuffers,
            Some(RenderSystems::PrepareResourcesFlush),
        ),
        (
            "render_flush",
            Some(RenderSystems::PrepareResourcesCollectPhaseBuffers),
            RenderSystems::PrepareResourcesFlush,
            Some(RenderSystems::PrepareBindGroups),
        ),
        (
            "render_bindgroups",
            Some(RenderSystems::PrepareResourcesFlush),
            RenderSystems::PrepareBindGroups,
            None,
        ),
    ] {
        let mut begin = (move |mut timers: ResMut<RenderTimers>| {
            timers.0.insert(name, Instant::now());
        })
        .in_set(RenderSystems::Prepare)
        .before(set.clone());
        if let Some(previous) = previous {
            begin = begin.after(previous);
        }
        let mut end = (move |mut timers: ResMut<RenderTimers>, performance: Res<Performance>| {
            if let Some(start) = timers.0.remove(name) {
                drop(Scope {
                    performance: Some(&performance),
                    name,
                    start,
                });
            }
        })
        .in_set(RenderSystems::Prepare)
        .after(set);
        if let Some(next) = next {
            end = end.before(next);
        }
        render.add_systems(Render, (begin, end));
    }
}
