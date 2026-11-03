//! Audio cvars (the Options Audio page): volumes, captions, speakers and sound quality.

use crate::config::{Captions, SoundQuality, Speakers};
use crate::console::Console;
use source_ui::console::command::{flag, parse, Invalid};

pub(super) fn volume_value(console: &Console) -> Option<String> {
    Some(format!("{:.2}", console.config.audio.volume))
}

pub(super) fn set_volume(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.audio.volume = parse::<f32>(arg)?.clamp(0.0, 1.0);
    Ok(())
}

pub(super) fn snd_musicvolume_value(console: &Console) -> Option<String> {
    Some(format!("{:.2}", console.config.audio.music_volume))
}

pub(super) fn set_snd_musicvolume(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.audio.music_volume = parse::<f32>(arg)?.clamp(0.0, 1.0);
    Ok(())
}

pub(super) fn snd_mute_losefocus_value(console: &Console) -> Option<String> {
    Some(u8::from(console.config.audio.mute_losefocus).to_string())
}

pub(super) fn set_snd_mute_losefocus(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.audio.mute_losefocus = flag(arg)?;
    Ok(())
}

pub(super) fn closecaption_value(console: &Console) -> Option<String> {
    Some(u8::from(console.config.audio.captions != Captions::None).to_string())
}

pub(super) fn set_closecaption(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    if !flag(arg)? {
        console.config.audio.captions = Captions::None;
    } else if console.config.audio.captions == Captions::None {
        console.config.audio.captions = Captions::ClosedCaptions;
    }
    Ok(())
}

pub(super) fn cc_subtitles_value(console: &Console) -> Option<String> {
    Some(u8::from(console.config.audio.captions == Captions::Subtitles).to_string())
}

pub(super) fn set_cc_subtitles(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    if flag(arg)? {
        console.config.audio.captions = Captions::Subtitles;
    } else if console.config.audio.captions == Captions::Subtitles {
        console.config.audio.captions = Captions::ClosedCaptions;
    }
    Ok(())
}

pub(super) fn snd_surround_speakers_value(console: &Console) -> Option<String> {
    Some(console.config.audio.speakers.channels().to_string())
}

pub(super) fn set_snd_surround_speakers(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.audio.speakers = Speakers::from_channels(parse(arg)?).ok_or(Invalid::Usage)?;
    Ok(())
}

pub(super) fn snd_pitchquality_value(console: &Console) -> Option<String> {
    Some(u8::from(console.config.audio.quality == SoundQuality::High).to_string())
}

pub(super) fn set_snd_pitchquality(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.audio.quality = if flag(arg)? {
        SoundQuality::High
    } else {
        SoundQuality::Medium
    };
    Ok(())
}

pub(super) fn dsp_slow_cpu_value(console: &Console) -> Option<String> {
    Some(u8::from(console.config.audio.quality == SoundQuality::Low).to_string())
}

pub(super) fn set_dsp_slow_cpu(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.audio.quality = if flag(arg)? {
        SoundQuality::Low
    } else {
        SoundQuality::High
    };
    Ok(())
}
