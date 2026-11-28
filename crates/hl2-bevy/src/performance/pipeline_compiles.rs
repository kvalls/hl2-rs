//! Pipelines compiled during play (`--profile` only). Bevy creates a render pipeline the first
//! time a view draws with it, synchronously on macOS, so every pipeline compiled after the level
//! is revealed is a stall in play. The render world lists, per frame since gameplay started,
//! which pipelines finished compiling (label, shaders, sample count, enabled shader defs), so a
//! hitch can be named without a trace build. Nothing runs without `--profile`.
use bevy::{
    prelude::{Local, Res},
    render::render_resource::{CachedPipelineState, PipelineCache, PipelineDescriptor},
    shader::{CachedPipelineId, ShaderCacheError, ShaderDefVal},
};

const MAX_EVENTS: usize = 64;
const MAX_NAMED: usize = 24;

/// What the report shows: compile events after gameplay started.
#[derive(Default)]
pub(super) struct Compiles {
    active: bool,
    frame: u64,
    total: usize,
    dropped: usize,
    events: Vec<Event>,
}

struct Event {
    frame: u64,
    /// The `frame_total` stage sample index at the time (the render world lags the main world by
    /// about one frame), so an event lines up with `cpu_stages.*.slowest`.
    stage_sample: usize,
    count: usize,
    pipelines: Vec<String>,
}

impl Compiles {
    /// Starts a fresh list at the first gameplay frame (the loading screen's compiles are not play).
    pub(super) fn start(&mut self) {
        *self = Self {
            active: true,
            ..Self::default()
        };
    }

    fn record(&mut self, compiled: Vec<String>, stage_sample: usize) {
        if !self.active {
            return;
        }
        self.frame += 1;
        if compiled.is_empty() {
            return;
        }
        self.total += compiled.len();
        if self.events.len() == MAX_EVENTS {
            self.dropped += 1;
            return;
        }
        let count = compiled.len();
        self.events.push(Event {
            frame: self.frame,
            stage_sample,
            count,
            pipelines: compiled.into_iter().take(MAX_NAMED).collect(),
        });
    }

    pub(super) fn report(&self) -> serde_json::Value {
        serde_json::json!({
            "scope": "render frames since gameplay started, map loads and held reveals left out; pipelines that finished compiling in that frame (a held frame just before gameplay may land in frame 1)",
            "total": self.total,
            "dropped_events": self.dropped,
            "events": self.events.iter().map(|e| serde_json::json!({
                "frame": e.frame,
                "stage_sample": e.stage_sample,
                "count": e.count,
                "pipelines": e.pipelines,
            })).collect::<Vec<_>>(),
        })
    }
}

/// The pipelines the cache still had waiting after the last frame, and how many it held.
#[derive(Default)]
pub(super) struct Seen {
    len: usize,
    waiting: Vec<CachedPipelineId>,
}

/// Runs in the render world's cleanup, after the frame's queue was processed.
pub(super) fn record(
    cache: Option<Res<PipelineCache>>,
    performance: Res<super::Performance>,
    mut seen: Local<Seen>,
) {
    let Some(cache) = cache else { return };
    // A slice iterator underneath: `count` and `nth` are O(1).
    let len = cache.pipelines().count();
    let candidates = std::mem::take(&mut seen.waiting)
        .into_iter()
        .chain(seen.len..len);
    let mut compiled = Vec::new();
    for id in candidates {
        let Some(pipeline) = cache.pipelines().nth(id) else {
            continue;
        };
        match &pipeline.state {
            CachedPipelineState::Ok(_) => compiled.push(describe(&pipeline.descriptor)),
            // The cache queues these again next frame (`PipelineCache::process_queue`).
            CachedPipelineState::Err(
                ShaderCacheError::ShaderNotLoaded(_)
                | ShaderCacheError::ShaderImportNotYetAvailable,
            )
            | CachedPipelineState::Queued
            | CachedPipelineState::Creating(_) => seen.waiting.push(id),
            CachedPipelineState::Err(_) => {
                compiled.push(format!("failed: {}", describe(&pipeline.descriptor)));
            }
        }
    }
    seen.len = len;
    if performance.loading() {
        return;
    }
    let stage_sample = performance.recorded(super::FRAME_TOTAL);
    performance
        .2
        .lock()
        .expect("pipeline compile log")
        .record(compiled, stage_sample);
}

fn describe(descriptor: &PipelineDescriptor) -> String {
    let path = |shader: &bevy::asset::Handle<bevy::shader::Shader>| {
        shader
            .path()
            .map_or_else(|| "<unnamed>".to_owned(), ToString::to_string)
    };
    match descriptor {
        PipelineDescriptor::RenderPipelineDescriptor(d) => {
            let mut text = format!(
                "{} vs={} msaa={}",
                d.label.as_deref().unwrap_or("<unlabelled>"),
                path(&d.vertex.shader),
                d.multisample.count
            );
            if let Some(fragment) = &d.fragment {
                text += &format!(" fs={}", path(&fragment.shader));
            }
            let fragment = d.fragment.iter().flat_map(|f| &f.shader_defs);
            text + &defs(d.vertex.shader_defs.iter().chain(fragment))
        }
        PipelineDescriptor::ComputePipelineDescriptor(d) => format!(
            "{} cs={}{}",
            d.label.as_deref().unwrap_or("<unlabelled>"),
            path(&d.shader),
            defs(d.shader_defs.iter())
        ),
        PipelineDescriptor::MeshPipelineDescriptor(d) => {
            format!("{} (mesh)", d.label.as_deref().unwrap_or("<unlabelled>"))
        }
    }
}

/// The defs that are switched on, each once, in order.
fn defs<'a>(all: impl Iterator<Item = &'a ShaderDefVal>) -> String {
    let mut names: Vec<String> = Vec::new();
    for def in all {
        let name = match def {
            ShaderDefVal::Bool(name, true) => name.to_string(),
            ShaderDefVal::Int(name, value) => format!("{name}={value}"),
            ShaderDefVal::UInt(name, value) => format!("{name}={value}"),
            ShaderDefVal::Bool(_, false) => continue,
        };
        if !names.contains(&name) {
            names.push(name);
        }
    }
    if names.is_empty() {
        String::new()
    } else {
        format!(" [{}]", names.join(","))
    }
}

#[cfg(test)]
mod tests {
    use super::{Compiles, defs};
    use bevy::shader::ShaderDefVal;

    #[test]
    fn only_compiles_after_gameplay_started_are_kept_and_events_are_bounded() {
        let mut compiles = Compiles::default();
        compiles.record(vec!["loading".into()], 0);
        assert_eq!(compiles.report()["total"], 0);
        compiles.start();
        compiles.record(Vec::new(), 0);
        compiles.record(vec!["a".into(), "b".into()], 7);
        let report = compiles.report();
        assert_eq!(report["total"], 2);
        assert_eq!(report["events"][0]["frame"], 2);
        assert_eq!(report["events"][0]["stage_sample"], 7);
        assert_eq!(report["events"][0]["pipelines"][1], "b");
        for _ in 0..super::MAX_EVENTS + 3 {
            compiles.record(vec!["c".into()], 8);
        }
        let report = compiles.report();
        assert_eq!(
            report["events"].as_array().unwrap().len(),
            super::MAX_EVENTS
        );
        assert_eq!(report["dropped_events"], 4);
        assert_eq!(report["total"], 2 + super::MAX_EVENTS + 3);
    }

    #[test]
    fn defs_list_enabled_values_once() {
        let all = [
            ShaderDefVal::Bool("SKINNED".into(), true),
            ShaderDefVal::Bool("MORPH_TARGETS".into(), false),
            ShaderDefVal::UInt("MAX_LIGHTS".into(), 8),
            ShaderDefVal::Bool("SKINNED".into(), true),
        ];
        assert_eq!(defs(all.iter()), " [SKINNED,MAX_LIGHTS=8]");
        assert_eq!(defs([].iter()), "");
    }
}
