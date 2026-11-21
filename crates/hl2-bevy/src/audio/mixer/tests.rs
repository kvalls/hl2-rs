use super::*;

fn pcm(samples: &[f32], channels: u16) -> Pcm {
    let bytes = super::super::wav::wav_bytes(samples, channels).unwrap();
    Pcm::parse(bytes.into()).unwrap()
}
fn start(gain: f32, speed: f32, looped: bool) -> Start {
    Start {
        looped,
        gain,
        speed,
        paused: false,
    }
}
/// The next `frames` stereo frames of the mixer's output.
fn output(mixer: &Mixer, frames: usize) -> Vec<f32> {
    let mut source = MixerStream(mixer.clone()).decoder();
    (0..frames * CHANNELS)
        .map(|_| source.next().unwrap())
        .collect()
}
fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn wav_parsing_accepts_backend_pcm_only() {
    let mono = pcm(&[0.5, -0.5], 1);
    assert_eq!((mono.frames(), mono.channels), (2, 1));
    assert!(near(mono.sample(1, 0), -0.5));
    let stereo = pcm(&[0.25, -0.25, 0.75, -0.75], 2);
    assert_eq!(stereo.frames(), 2);
    assert!(near(stereo.sample(1, 1), -0.75));
    assert!(Pcm::parse(b"RIFF\0\0\0\0WAVEjunk".to_vec().into()).is_none());
    let mut other_rate = super::super::wav::wav_bytes(&[0.], 1).unwrap();
    other_rate[24..28].copy_from_slice(&44_100u32.to_le_bytes());
    assert!(Pcm::parse(other_rate.into()).is_none());
}

#[test]
fn a_mono_voice_plays_on_both_channels_at_its_gain_then_ends() {
    let mixer = Mixer::default();
    let voice = mixer.play(pcm(&[0.5; 300], 1), start(0.5, 1., false));
    assert!(!voice.finished());
    // Two blocks: 300 frames of sound, then silence once it has ended.
    let out = output(&mixer, 2 * BLOCK);
    assert!(out[..600].iter().all(|s| near(*s, 0.25)));
    assert!(out[600..].iter().all(|s| *s == 0.));
    assert!(voice.finished());
    assert!(near(
        voice.position().as_secs_f32() * BACKEND_RATE as f32,
        300.
    ));
}

#[test]
fn stereo_voices_keep_their_channels_and_voices_add_up() {
    let mixer = Mixer::default();
    let left: Vec<f32> = [0.25, 0.].repeat(BLOCK);
    let _left = mixer.play(pcm(&left, 2), start(1., 1., true));
    let _both = mixer.play(pcm(&[0.25; BLOCK], 1), start(1., 1., true));
    let out = output(&mixer, BLOCK);
    assert!(out.chunks(2).all(|f| near(f[0], 0.5) && near(f[1], 0.25)));
}

#[test]
fn gain_changes_ramp_across_one_block() {
    let mixer = Mixer::default();
    let voice = mixer.play(pcm(&[1.; 4 * BLOCK], 1), start(0., 1., false));
    voice.set_gain(1.);
    let out = output(&mixer, BLOCK);
    assert!(near(out[0], 1. / BLOCK as f32) && near(out[2 * BLOCK - 1], 1.));
    assert!(
        out.chunks(2)
            .zip(out.chunks(2).skip(1))
            .all(|(a, b)| b[0] >= a[0])
    );
    assert!(output(&mixer, BLOCK).iter().all(|s| near(*s, 1.)));
}

#[test]
fn speed_steps_through_the_source_with_linear_interpolation() {
    let mixer = Mixer::default();
    let ramp: Vec<f32> = (0..1000).map(|i| i as f32 / 2000.).collect();
    let voice = mixer.play(pcm(&ramp, 1), start(1., 0.5, false));
    let out = output(&mixer, 4);
    // Half speed: frames 0, 0.5, 1, 1.5 of a ramp rising 1/2000 per frame.
    for (i, frame) in out.chunks(2).enumerate() {
        assert!(near(frame[0], i as f32 * 0.5 / 2000.), "{frame:?}");
    }
    voice.set_speed(2.);
    let _ = output(&mixer, BLOCK);
    assert!(voice.position().as_secs_f64() * f64::from(BACKEND_RATE) > 2. * BLOCK as f64);
}

#[test]
fn loops_start_over_and_paused_or_silent_voices_hold_or_move_on() {
    let mixer = Mixer::default();
    let looped = mixer.play(pcm(&[0.5; 100], 1), start(1., 1., true));
    let out = output(&mixer, 3 * BLOCK);
    assert!(out.iter().all(|s| near(*s, 0.5)) && !looped.finished());
    looped.set_paused(true);
    let held = looped.position();
    assert!(output(&mixer, BLOCK).iter().all(|s| *s == 0.));
    assert_eq!(looped.position(), held);
    // A silent one-shot still reaches its end.
    let silent = mixer.play(pcm(&[0.5; 300], 1), start(0., 1., false));
    let _ = output(&mixer, 2 * BLOCK);
    assert!(silent.finished());
}

#[test]
fn dropping_a_voice_stops_it() {
    let mixer = Mixer::default();
    let voice = mixer.play(pcm(&[0.5; 4 * BLOCK], 1), start(1., 1., false));
    assert_eq!(mixer.voices(), 1);
    drop(voice);
    assert_eq!(mixer.voices(), 0);
    assert!(output(&mixer, BLOCK).iter().all(|s| *s == 0.));
}
