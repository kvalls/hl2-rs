//! Bounded parallel decoding; accept results in authored sorted order to preserve budgets/errors.
use crate::{
    assets::work::Cancellation,
    audio::{
        MAX_PRELOADED_BYTES, Vox,
        reuse::{AudioReuse, Decoded},
    },
};
use anyhow::{Context, Result};
use hl2_simulation::sounds::Library;
use source_assets::vpk::Vfs;
use std::sync::{Arc, mpsc};

type Wave = (String, Arc<[u8]>);
type Sentence = (String, source_assets::sentence::Sentence, f32);
#[derive(Default)]
pub(super) struct DecodedAudio {
    pub waves: Vec<Wave>,
    pub sentences: Vec<Sentence>,
    pub bytes: usize,
    pub vox: Vox,
}
impl DecodedAudio {
    fn accept(&mut self, path: &str, result: Result<Arc<Decoded>, String>, library: &mut Library) {
        match result {
            Ok(decoded) => {
                let size = decoded.waves.iter().map(|(_, d)| d.len()).sum::<usize>();
                if size > MAX_PRELOADED_BYTES.saturating_sub(self.bytes) {
                    library.errors.insert(
                        path.into(),
                        "owned audio preload exceeds 512 MiB budget".into(),
                    );
                    return;
                }
                if let Some(sentence) = &decoded.sentence {
                    self.sentences.push((
                        path.into(),
                        sentence.clone(),
                        decoded.summary.seconds() as f32,
                    ));
                }
                library.decoded.insert(path.into(), decoded.summary.clone());
                for (key, data) in &decoded.waves {
                    self.bytes += data.len();
                    self.waves.push((key.clone(), data.clone()));
                }
            }
            Err(error) => {
                library.errors.insert(path.into(), error);
            }
        }
    }
}

pub(super) fn load(
    vfs: &Vfs,
    specs: &[(String, bool, bool)],
    library: &mut Library,
    reuse: &AudioReuse,
    cancel: &Cancellation,
) -> Result<DecodedAudio> {
    let cores = std::thread::available_parallelism().map_or(1, usize::from);
    if cores < 4 {
        let mut out = DecodedAudio::default();
        for (path, looped, distance) in specs {
            cancel.check()?;
            out.accept(
                path,
                reuse
                    .wave(vfs, path, *looped, *distance, cancel)
                    .map_err(|e| format!("{e:#}")),
                library,
            );
        }
        out.vox = Vox::load(vfs, reuse, cancel)?;
        return Ok(out);
    }
    // At most two wave decoders plus one VOX decoder, irrespective of map size.
    let workers = if cores >= 6 { 2 } else { 1 };
    std::thread::scope(|scope| {
        let vox = scope.spawn(|| Vox::load(vfs, reuse, cancel));
        let mut receivers = Vec::with_capacity(workers);
        for id in 0..workers {
            let (sender, receiver) = mpsc::sync_channel(1);
            receivers.push(receiver);
            scope.spawn(move || {
                for (path, looped, distance) in specs.iter().skip(id).step_by(workers) {
                    if cancel.cancelled() {
                        break;
                    }
                    let result = reuse
                        .wave(vfs, path, *looped, *distance, cancel)
                        .map_err(|e| format!("{e:#}"));
                    if sender.send(result).is_err() {
                        break;
                    }
                }
            });
        }
        let mut out = DecodedAudio::default();
        for (index, (path, _, _)) in specs.iter().enumerate() {
            cancel.check()?;
            let result = receivers[index % workers]
                .recv()
                .context("owned audio decoder disconnected")?;
            out.accept(path, result, library);
        }
        out.vox = vox
            .join()
            .map_err(|_| anyhow::anyhow!("owned VOX decoder panicked"))??;
        Ok(out)
    })
}
