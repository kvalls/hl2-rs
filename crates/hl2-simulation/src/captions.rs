//! Close caption requests from choreographed speech (SDK CSceneEntity::DispatchStartSpeak,
//! CChoreoChannel::ReconcileCloseCaption, AttenuateCaption). The host shows them through
//! the HUD caption panel; ordinary EmitSound captions are emitted by the host's sound queue.
use glam::Vec3;
use source_assets::scenes::{ChoreoScene, EventType};

/// `scene_maxcaptionradius`: captions only reach a player within this many units of one of
/// the sound's origins (0 disables the check).
pub const MAX_CAPTION_RADIUS: f32 = 1200.;
/// CChoreoEvent::CLOSECAPTION.
const CC_DISABLED: i8 = 2;
/// Speech flag bit 2: the event never attenuates its caption (breencasts).
const SUPPRESS_ATTENUATION: u8 = 1 << 2;

/// A caption to show (the CloseCaption user message).
#[derive(Clone, Debug, PartialEq)]
pub struct CaptionRequest {
    /// Lower-cased caption token.
    pub token: String,
    /// Seconds, before the message's tenth-of-a-second quantization.
    pub duration: f32,
    pub from_player: bool,
    /// The speaking actor, whose model gender selects `<token>_male` / `_female`.
    pub actor: Option<crate::sounds::SoundActor>,
}

/// The playback caption of SPEAK event `index` when it is its group's master (the earliest
/// event of its actor's channel sharing its caption token; an event without a token is its own
/// master): the token (the caption token, else the sound name) and the longer of the
/// event's own length and the time to the end of its group's last event.
pub fn scene_caption(scene: &ChoreoScene, index: usize) -> Option<(String, f32)> {
    let event = scene.events.get(index)?;
    let speech = event.speech.as_ref()?;
    if event.kind != EventType::Speak || speech.caption_type == CC_DISABLED {
        return None;
    }
    let length = event.end.map_or(0., |end| end - event.start);
    let mut last_end = 0.;
    if !speech.caption_token.is_empty() {
        let group: Vec<&source_assets::scenes::Event> = scene
            .events
            .iter()
            .filter(|e| {
                e.kind == EventType::Speak && e.actor == event.actor && e.channel == event.channel
            })
            .filter(|e| {
                e.speech.as_ref().is_some_and(|s| {
                    s.caption_type != CC_DISABLED
                        && s.caption_token.eq_ignore_ascii_case(&speech.caption_token)
                })
            })
            .collect();
        if group.len() > 1 {
            // Ordered by start time; ties keep their order.
            let master = group
                .iter()
                .min_by(|a, b| a.start.total_cmp(&b.start))
                .copied()?;
            if !std::ptr::eq(master, event) {
                return None;
            }
            last_end = group
                .iter()
                .max_by(|a, b| a.start.total_cmp(&b.start))
                .and_then(|e| e.end)
                .unwrap_or(0.);
        }
    }
    let token = if speech.caption_token.is_empty() {
        &event.parameters[0]
    } else {
        &speech.caption_token
    };
    if token.is_empty() {
        return None;
    }
    Some((
        token.to_ascii_lowercase(),
        length.max(last_end - event.start),
    ))
}

/// Whether the event's caption skips the distance check.
pub fn suppresses_attenuation(scene: &ChoreoScene, index: usize) -> bool {
    scene
        .events
        .get(index)
        .and_then(|e| e.speech.as_ref())
        .is_some_and(|s| s.flags & SUPPRESS_ATTENUATION != 0)
}

/// AttenuateCaption: true when every sound origin is beyond the caption radius of
/// `listener` (no origins never attenuate).
pub fn attenuated(listener: Vec3, origins: &[Vec3]) -> bool {
    MAX_CAPTION_RADIUS > 0.
        && !origins.is_empty()
        && origins
            .iter()
            .all(|origin| origin.distance_squared(listener) > MAX_CAPTION_RADIUS.powi(2))
}

/// CSoundEmitterSystem::EmitCloseCaption: a sound-script caption is dropped when the sound's
/// origin is beyond the caption radius of the player (no view-entity retry, unlike scenes).
/// Unpositioned sounds, and ambient_generic (EmitAmbientSound passes no origins), never are.
pub fn sound_caption_attenuated(player: Vec3, origin: Option<Vec3>) -> bool {
    origin.is_some_and(|origin| attenuated(player, &[origin]))
}

/// An env_microphone as the caption routing needs it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Microphone {
    /// Where it measures from (its `target`, else itself).
    pub measure: Vec3,
    /// `MaxRange`; 0 hears at any distance.
    pub max_range: f32,
    /// Its speaker's origin.
    pub speaker: Vec3,
}

/// The sound origins of a sound played at `source` (CEnvMicrophone::SoundPlayed): the source
/// itself, then the speaker of every enabled microphone in range, which replays it there.
pub fn sound_origins(source: Vec3, microphones: &[Microphone]) -> Vec<Vec3> {
    std::iter::once(source)
        .chain(
            microphones
                .iter()
                .filter(|m| m.max_range <= 0. || m.measure.distance(source) <= m.max_range)
                .map(|m| m.speaker),
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use source_assets::scenes::{Event, Speech};

    fn speak(start: f32, end: f32, sound: &str, token: &str, channel: usize) -> Event {
        Event {
            kind: EventType::Speak,
            name: sound.into(),
            start,
            end: Some(end),
            parameters: [sound.into(), String::new(), String::new()],
            ramp: Vec::new(),
            flags: 8,
            distance: 0.,
            relative_tags: Vec::new(),
            timing_tags: Vec::new(),
            absolute_tags: [Vec::new(), Vec::new()],
            gesture_duration: None,
            relative_reference: None,
            flex_tracks: Vec::new(),
            loop_count: None,
            speech: Some(Speech {
                caption_type: 0,
                caption_token: token.into(),
                flags: 0,
            }),
            actor: Some(0),
            channel: Some(channel),
            enabled: true,
        }
    }

    fn scene(events: Vec<Event>) -> ChoreoScene {
        ChoreoScene {
            text_crc32: 0,
            actors: Vec::new(),
            events,
            ramp: Vec::new(),
            ignore_phonemes: false,
        }
    }

    #[test]
    fn single_speech_captions_its_sound_name_for_its_length() {
        let s = scene(vec![speak(0.14, 12.62, "EP_01.KL_FellowCitizens06", "", 0)]);
        let (token, duration) = scene_caption(&s, 0).unwrap();
        assert_eq!(token, "ep_01.kl_fellowcitizens06");
        assert!((duration - 12.48).abs() < 1e-4);
    }

    #[test]
    fn grouped_speech_captions_once_from_the_master_until_the_last_slave() {
        let s = scene(vec![
            speak(4., 6., "line.b", "combined.token", 0),
            speak(1., 3., "line.a", "Combined.Token", 0),
            speak(2., 3., "other.channel", "combined.token", 1),
            speak(7., 9.5, "line.c", "combined.token", 0),
        ]);
        let (token, duration) = scene_caption(&s, 1).unwrap();
        assert_eq!(token, "combined.token");
        assert_eq!(duration, 8.5, "from 1 until the last slave ends at 9.5");
        assert_eq!(scene_caption(&s, 0), None, "slave");
        assert_eq!(scene_caption(&s, 3), None, "slave");
        // Alone on its channel.
        assert_eq!(scene_caption(&s, 2), Some(("combined.token".into(), 1.)));
        let mut disabled = scene(vec![speak(0., 1., "x", "", 0)]);
        disabled.events[0].speech.as_mut().unwrap().caption_type = CC_DISABLED;
        assert_eq!(scene_caption(&disabled, 0), None);
    }

    #[test]
    fn captions_attenuate_beyond_the_radius_unless_a_speaker_is_near() {
        let kleiner = Vec3::new(10987.1, -2147.19, -1038.86);
        let camera = Vec3::new(4900.05, -510.362, 2.1579);
        assert!(attenuated(camera, &[kleiner]));
        assert!(!attenuated(camera, &[]));
        // ep1_background02: mic_breencast_jumbotron2 (MaxRange 100) next to Kleiner replays
        // him at speaker_breencast_jumbotron2 beside the camera.
        let microphone = Microphone {
            measure: Vec3::new(10993., -2180., -988.595),
            max_range: 100.,
            speaker: Vec3::new(4920.42, -494.81, -28.),
        };
        let origins = sound_origins(kleiner, &[microphone]);
        assert_eq!(origins.len(), 2);
        assert!(!attenuated(camera, &origins));
        let far = Microphone {
            measure: kleiner + Vec3::X * 150.,
            ..microphone
        };
        assert_eq!(sound_origins(kleiner, &[far]), [kleiner]);
    }

    #[test]
    fn sound_script_captions_attenuate_by_the_player_origin_only() {
        let player = Vec3::ZERO;
        assert!(!sound_caption_attenuated(player, None), "unpositioned");
        assert!(!sound_caption_attenuated(player, Some(Vec3::X * 1200.)));
        assert!(sound_caption_attenuated(player, Some(Vec3::X * 1201.)));
    }
}
