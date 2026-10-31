//! Profiling-only inventory of live blur textures, deduplicated by GPU identity.
use super::Textures;
use crate::bevy::diagnostics::Diagnostics;
use bevy::{
    core_pipeline::prepass::ViewPrepassTextures,
    prelude::{IntoScheduleConfigs, Local, Query, Res},
    render::{
        Render, RenderSystems,
        render_resource::{Texture, TextureId},
    },
};

#[derive(Default, Clone, serde::Serialize)]
pub struct Report {
    pub references: u32,
    pub unique_textures: u32,
    pub nominal_bytes: u64,
    pub motion_references: u32,
    pub unique_motion_textures: u32,
    pub motion_bytes: u64,
    pub motion_sample_count: u32,
    pub overflow: bool,
}

#[derive(Default)]
struct Inventory {
    ids: [Option<TextureId>; 32],
    report: Report,
}
impl Inventory {
    fn add(&mut self, texture: &Texture, motion: bool) {
        self.report.references += 1;
        self.report.motion_references += u32::from(motion);
        if self.ids.contains(&Some(texture.id())) {
            return;
        }
        let Some(slot) = self.ids.iter_mut().find(|id| id.is_none()) else {
            self.report.overflow = true;
            return;
        };
        *slot = Some(texture.id());
        let bytes = payload(
            texture.width(),
            texture.height(),
            texture.sample_count(),
            texture
                .format()
                .block_copy_size(None)
                .expect("uncompressed blur format"),
        );
        self.report.unique_textures += 1;
        self.report.nominal_bytes += bytes;
        if motion {
            self.report.unique_motion_textures += 1;
            self.report.motion_bytes += bytes;
            self.report.motion_sample_count = texture.sample_count();
        }
    }
}
fn payload(width: u32, height: u32, samples: u32, bytes: u32) -> u64 {
    u64::from(width) * u64::from(height) * u64::from(samples) * u64::from(bytes)
}

pub fn install(render: &mut bevy::app::SubApp) {
    render.add_systems(Render, capture.in_set(RenderSystems::Cleanup));
}
fn capture(
    mut frames: Local<u32>,
    views: Query<(&Textures, Option<&ViewPrepassTextures>)>,
    performance: Res<Diagnostics>,
) {
    *frames += 1;
    if !frames.is_multiple_of(120) {
        return;
    }
    let mut inventory = Inventory::default();
    for (textures, prepass) in &views {
        inventory.add(&textures.background.texture, false);
        for reduced in &textures.reduced {
            inventory.add(&reduced.texture, false);
        }
        if let Some(output) = &textures.output {
            inventory.add(&output.texture, false);
        }
        if let Some(gathered) = &textures.object_gathered {
            inventory.add(&gathered.texture, false);
        }
        if let Some(environment) = &textures.environment {
            inventory.add(&environment.texture, false);
        }
        if let Some(motion) = prepass.and_then(|p| p.motion_vectors.as_ref()) {
            inventory.add(&motion.texture.texture, true);
            if let Some(resolve) = &motion.resolve_target {
                inventory.add(&resolve.texture, true);
            }
        }
    }
    performance.record_blur_memory(inventory.report);
}

#[cfg(test)]
mod tests {
    use super::payload;
    #[test]
    fn native_motion_payload_counts_samples_without_overflow() {
        assert_eq!(payload(3456, 2168, 4, 4), 119881728);
        assert_eq!(payload(65536, 65536, 8, 8), 274877906944);
    }
}
