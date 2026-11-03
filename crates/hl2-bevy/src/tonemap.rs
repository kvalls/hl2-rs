//! Source HDR tone mapping (mat_hdr_level 2): every Source shader multiplies its output by
//! the tonemap scale (FinalOutput TONEMAP_SCALE_LINEAR), which Bevy carries as camera
//! exposure. Auto exposure samples a luminance histogram of the presented frame.
use crate::gameplay::Gameplay;
use bevy::{
    asset::RenderAssetUsages,
    camera::Exposure,
    prelude::*,
    render::{
        gpu_readback::{Readback, ReadbackComplete},
        render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
    },
};
use hl2_simulation::tonemap::{self, AutoExposure, BINS};
use std::sync::{Arc, Mutex};

/// Cameras whose Source materials take the tonemap scale.
#[derive(Component)]
pub struct ToneMapped;

/// sRGB byte to linear.
static LINEAR: std::sync::LazyLock<[f32; 256]> = std::sync::LazyLock::new(|| {
    std::array::from_fn(|i| {
        let c = i as f32 / 255.;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    })
});
/// Readback interval in frames; the SDK refreshes one of 17 histogram queries per frame.
const INTERVAL: u64 = 8;

#[derive(Resource)]
pub struct Tonemap {
    pub exposure: AutoExposure,
    pub forced: Option<f32>,
    histogram: Arc<Mutex<Option<[u32; BINS]>>>,
    pending: Arc<Mutex<bool>>,
    frames: u64,
    map: String,
    pub readbacks: u64,
    /// SDK GetBloomAmount state (starts at 1 each level).
    pub bloom: f32,
    /// `mat_hdr_level` of the applied settings: below Full the frame takes no automatic exposure.
    pub level: hl2_ui::config::HdrMode,
    /// Pre-bloom frame written by the bloom pass (the SDK histogram runs before bloom).
    presample: Option<Handle<Image>>,
}
impl Tonemap {
    pub fn new(forced: Option<f32>) -> Self {
        Self {
            exposure: AutoExposure::default(),
            forced,
            histogram: Default::default(),
            pending: Default::default(),
            frames: 0,
            map: String::new(),
            readbacks: 0,
            bloom: 1.,
            level: hl2_ui::config::HdrMode::Full,
            presample: None,
        }
    }
    pub fn scale(&self) -> f32 {
        if !self.level.is_hdr() {
            return 1.;
        }
        self.forced.unwrap_or(self.exposure.current)
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"scale": self.scale(), "forced": self.forced, "auto": self.exposure,
            "histogram": *self.histogram.lock().expect("tonemap histogram"), "readbacks": self.readbacks, "bloom": self.bloom})
    }
}

/// Bevy view exposure = 2^-ev100 / 1.2.
pub fn ev100(scale: f32) -> f32 {
    -(1.2 * scale.max(1e-6)).log2()
}

pub fn update(
    mut commands: Commands,
    mut state: ResMut<Tonemap>,
    (game, sim, ui): (
        Res<Gameplay>,
        Res<crate::simulation::Simulation>,
        Option<Res<crate::ui_host::Console>>,
    ),
    time: Res<Time<Real>>,
    mut images: ResMut<Assets<Image>>,
    mut cameras: Query<&mut Exposure, With<ToneMapped>>,
    mut blooms: Query<&mut crate::bloom::SourceBloom>,
) {
    if let Some(ui) = ui.as_deref() {
        state.level = ui.source.config.video.hdr;
    }
    state.frames += 1;
    let presample = state
        .presample
        .get_or_insert_with(|| {
            let mut image = Image::new_fill(
                Extent3d {
                    width: PRESAMPLE.0,
                    height: PRESAMPLE.1,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                &[0, 0, 0, 255],
                TextureFormat::Rgba8Unorm,
                RenderAssetUsages::RENDER_WORLD,
            );
            image.texture_descriptor.usage |=
                TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC;
            images.add(image)
        })
        .clone();
    // ResetToneMapping(1.0) when a level starts.
    if state.map != game.world.name {
        state.map = game.world.name.clone();
        state.exposure = AutoExposure::default();
        state.bloom = 1.;
        *state.histogram.lock().expect("tonemap histogram") = None;
    }
    // A paused game freezes everything, exposure and bloom adaptation included (owner
    // observation of native HL2, 2026-10-08); the last exposure stays on the cameras. A menu
    // background level keeps running.
    let paused = !sim.world_running();
    if state.forced.is_none() && state.level.is_hdr() && !paused {
        let request = state.frames.is_multiple_of(INTERVAL) && {
            let mut pending = state.pending.lock().expect("tonemap pending");
            !std::mem::replace(&mut *pending, true)
        };
        if request {
            state.readbacks += 1;
            let (histogram, pending) = (state.histogram.clone(), state.pending.clone());
            commands
                .spawn(Readback::texture(presample.clone()))
                .observe(move |event: On<ReadbackComplete>, mut commands: Commands| {
                    if let Some(h) = histogram_of(&event.data) {
                        *histogram.lock().expect("tonemap histogram") = Some(h);
                    }
                    *pending.lock().expect("tonemap pending") = false;
                    commands.entity(event.entity).despawn();
                });
        }
        let control = game.scene.tonemap;
        let latest = *state.histogram.lock().expect("tonemap histogram");
        if let Some(h) = latest {
            let current = state.exposure.current;
            state
                .exposure
                .set_target(tonemap::target_scale(&h, current), control);
        }
        state.exposure.advance(time.delta_secs(), control);
    }
    if !paused {
        state.bloom = tonemap::ease_bloom(state.bloom, game.scene.tonemap.bloom);
    }
    for mut bloom in &mut blooms {
        bloom.amount = state.bloom * state.level.bloom_factor();
        if bloom.presample.as_ref() != Some(&presample) {
            bloom.presample = Some(presample.clone());
        }
    }
    let ev = ev100(state.scale());
    for mut exposure in &mut cameras {
        if exposure.ev100 != ev {
            exposure.ev100 = ev;
        }
    }
}

/// Presample size (gamma-encoded RGBA8; 1280-byte rows need no readback padding).
const PRESAMPLE: (u32, u32) = (320, 180);

/// SDK luminance histogram over the central 90% x 85% of the pre-bloom frame. Float HDR
/// (retail client 101d8490 scales the comparison by the tonemap scale only for HDR type 2)
/// measures linear colour, so the gamma-encoded presample is decoded first.
fn histogram_of(data: &[u8]) -> Option<[u32; BINS]> {
    let (width, height) = (PRESAMPLE.0 as usize, PRESAMPLE.1 as usize);
    if data.len() < width * height * 4 {
        return None;
    }
    let (x0, x1) = (
        (width as f32 * 0.05) as usize,
        (width as f32 * 0.95) as usize,
    );
    let (y0, y1) = (
        (height as f32 * 0.075) as usize,
        (height as f32 * 0.925) as usize,
    );
    let mut bins = [0u32; BINS];
    for y in y0..y1 {
        for x in x0..x1 {
            let p = &data[(y * width + x) * 4..][..4];
            let rgb = [p[0], p[1], p[2]].map(|c| LINEAR[c as usize]);
            if let Some(bin) = tonemap::bin_of(tonemap::luminance(rgb)) {
                bins[bin] += 1;
            }
        }
    }
    Some(bins)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_ldr_frame_has_no_exposure_and_scaled_bloom() {
        use hl2_ui::config::HdrMode;
        let mut state = Tonemap::new(Some(2.));
        state.exposure.current = 0.5;
        assert_eq!(state.scale(), 2., "HDR follows the forced scale");
        for level in [HdrMode::None, HdrMode::Bloom] {
            state.level = level;
            assert_eq!(state.scale(), 1., "{level:?}");
        }
    }

    #[test]
    fn ev100_round_trips_bevy_exposure() {
        for scale in [0.5f32, 1., 1.35, 2.] {
            let exposure = Exposure {
                ev100: ev100(scale),
            };
            assert!((exposure.exposure() - scale).abs() < 1e-5, "{scale}");
        }
    }
}
