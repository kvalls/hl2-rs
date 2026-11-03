//! A late audio result belongs to exactly one installed world and is never joined on a frame.
use super::{PreparedAudio, install};
use crate::{assets::work::Work, gameplay::Gameplay, simulation::Simulation};
use bevy::{
    audio::AudioSource,
    prelude::{Assets, Commands, Res, ResMut, Resource},
};
use std::sync::Arc;

#[derive(Resource)]
pub(crate) struct AudioOwner(pub(super) Arc<()>);

pub(crate) fn stage(commands: &mut Commands, work: Option<Work<PreparedAudio>>) {
    let owner = Arc::new(());
    commands.insert_resource(AudioOwner(owner.clone()));
    if let Some(work) = work {
        commands.insert_resource(PendingAudio::new(work, owner));
    } else {
        commands.remove_resource::<PendingAudio>();
    }
}

#[derive(Resource)]
pub(crate) struct PendingAudio {
    work: Work<PreparedAudio>,
    owner: Arc<()>,
    requests: Vec<hl2_simulation::sounds::SoundRequest>,
    captions: Vec<hl2_simulation::captions::CaptionRequest>,
}
impl PendingAudio {
    pub(crate) fn new(work: Work<PreparedAudio>, owner: Arc<()>) -> Self {
        Self {
            work,
            owner,
            requests: Vec::new(),
            captions: Vec::new(),
        }
    }
    pub(crate) fn defer(&mut self, game: &mut Gameplay) {
        self.requests.append(&mut game.sound_requests);
        self.captions.append(&mut game.captions);
        // Background-only backlog: bounded even if decoding repeatedly fails or stalls.
        if self.requests.len() > 1024 {
            self.requests.drain(..self.requests.len() - 1024);
        }
        if self.captions.len() > 1024 {
            self.captions.drain(..self.captions.len() - 1024);
        }
    }
    pub(crate) fn cancel(&self) {
        self.work.cancel.cancel();
    }
    pub(crate) fn waiting(&self) -> bool {
        !self.work.cancel.cancelled()
    }
}

pub(crate) fn poll(
    mut commands: Commands,
    pending: Option<ResMut<PendingAudio>>,
    mut game: ResMut<Gameplay>,
    (mut sim, owner): (ResMut<Simulation>, Option<Res<AudioOwner>>),
    mut sources: ResMut<Assets<AudioSource>>,
    options: Res<crate::Options>,
    mut ui: ResMut<crate::ui_host::Console>,
) {
    let Some(mut pending) = pending else {
        return;
    };
    if !owner.is_some_and(|owner| Arc::ptr_eq(&pending.owner, &owner.0)) || !pending.waiting() {
        pending.cancel();
        sim.background_audio_pending = false;
        commands.remove_resource::<PendingAudio>();
        return;
    }
    // Freeze installation while a replacement is being decoded, even if its first poll races.
    if sim.loading() {
        return;
    }
    let Some(result) = pending.work.poll() else {
        return;
    };
    match result {
        Ok(mut prepared) => {
            for (wave, sentence, seconds) in prepared.sentences.drain(..) {
                game.scene.lipsync.add_sentence(&wave, sentence, seconds);
            }
            pending.requests.append(&mut game.sound_requests);
            game.sound_requests.append(&mut pending.requests);
            pending.captions.append(&mut game.captions);
            game.captions.append(&mut pending.captions);
            install(&mut commands, &mut sources, prepared, options.mute);
        }
        Err(error) => ui
            .source
            .log(format!("Background audio preparation failed: {error}")),
    }
    sim.background_audio_pending = false;
    commands.remove_resource::<PendingAudio>();
}
