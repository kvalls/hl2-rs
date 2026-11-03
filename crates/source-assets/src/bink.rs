//! Bink Video (`.bik`) container header parser.
//!
//! Half-Life 2 and its episodes use Bink 1 (`BIKi` / `BIKb`) videos for the valve logo,
//! recap videos, and teaser movies in `media/`.

use anyhow::{ensure, Result};

/// Whether a software or hardware video frame decoder is present.
/// The container header reader parses metadata, geometry and duration for all
/// Bink movies; frame packet decompression into RGBA presentation textures is
/// not yet implemented.
pub const HAS_VIDEO_DECODER: bool = false;

/// Header metadata parsed from a Bink 1 container file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinkHeader {
    /// FourCC magic, e.g. `b"BIKi"`.
    pub magic: [u8; 4],
    /// File size in bytes (as declared in the header).
    pub file_size: u32,
    /// Total number of frames in the movie.
    pub frames: u32,
    /// Largest individual frame packet size in bytes.
    pub max_frame_size: u32,
    /// Native video width in pixels.
    pub width: u32,
    /// Native video height in pixels.
    pub height: u32,
    /// Frame rate numerator.
    pub fps_num: u32,
    /// Frame rate denominator.
    pub fps_den: u32,
    /// Container flags.
    pub flags: u32,
    /// Number of multiplexed audio tracks.
    pub audio_tracks: u32,
    /// Primary audio track sample rate in Hz (e.g. 44100 or 48000), if present.
    pub audio_sample_rate: u32,
    /// Primary audio track channel count (1 = mono, 2 = stereo), if present.
    pub audio_channels: u32,
}

impl BinkHeader {
    /// Parses a Bink 1 video container header from file bytes.
    pub fn read(data: &[u8]) -> Result<Self> {
        ensure!(
            data.len() >= 44,
            "truncated Bink header: need at least 44 bytes, got {}",
            data.len()
        );

        let magic: [u8; 4] = data[0..4].try_into()?;
        ensure!(
            &magic[0..3] == b"BIK" || &magic[0..3] == b"KB2",
            "invalid Bink magic: {:?}, expected BIK/KB2",
            magic
        );

        let file_size_minus_8 = u32::from_le_bytes(data[4..8].try_into()?);
        let file_size = file_size_minus_8.saturating_add(8);
        let frames = u32::from_le_bytes(data[8..12].try_into()?);
        let max_frame_size = u32::from_le_bytes(data[12..16].try_into()?);
        let width = u32::from_le_bytes(data[20..24].try_into()?);
        let height = u32::from_le_bytes(data[24..28].try_into()?);
        let fps_num = u32::from_le_bytes(data[28..32].try_into()?);
        let fps_den = u32::from_le_bytes(data[32..36].try_into()?);
        let flags = u32::from_le_bytes(data[36..40].try_into()?);
        let audio_tracks = u32::from_le_bytes(data[40..44].try_into()?);

        ensure!(width > 0 && width <= 8192, "invalid video width: {width}");
        ensure!(
            height > 0 && height <= 8192,
            "invalid video height: {height}"
        );
        ensure!(frames > 0, "video has zero frames");

        let mut audio_sample_rate = 0;
        let mut audio_channels = 0;

        if audio_tracks > 0 {
            let track_info_offset = 44 + (audio_tracks as usize) * 4;
            if data.len() >= track_info_offset + 4 {
                let rate =
                    u16::from_le_bytes(data[track_info_offset..track_info_offset + 2].try_into()?)
                        as u32;
                let track_flags = data[track_info_offset + 3];
                let is_stereo = (track_flags & 0x10) != 0;
                audio_sample_rate = rate;
                audio_channels = if is_stereo { 2 } else { 1 };
            }
        }

        Ok(Self {
            magic,
            file_size,
            frames,
            max_frame_size,
            width,
            height,
            fps_num,
            fps_den,
            flags,
            audio_tracks,
            audio_sample_rate,
            audio_channels,
        })
    }

    /// Calculated frame rate in frames per second (defaults to 30.0 if denominator is 0).
    #[inline]
    pub fn fps(&self) -> f32 {
        if self.fps_den > 0 && self.fps_num > 0 {
            self.fps_num as f32 / self.fps_den as f32
        } else {
            30.0
        }
    }

    /// Calculated playback duration in seconds.
    #[inline]
    pub fn duration(&self) -> f32 {
        let fps = self.fps();
        if fps > 0.0 {
            self.frames as f32 / fps
        } else {
            0.0
        }
    }

    /// Aspect ratio (`width / height`).
    #[inline]
    pub fn aspect_ratio(&self) -> f32 {
        if self.height > 0 {
            self.width as f32 / self.height as f32
        } else {
            4.0 / 3.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_header_parses_geometry_and_fps() {
        let mut data = vec![0u8; 56];
        data[0..4].copy_from_slice(b"BIKi");
        data[4..8].copy_from_slice(&1000u32.to_le_bytes());
        data[8..12].copy_from_slice(&240u32.to_le_bytes()); // frames
        data[12..16].copy_from_slice(&5000u32.to_le_bytes()); // max frame
        data[20..24].copy_from_slice(&1280u32.to_le_bytes()); // width
        data[24..28].copy_from_slice(&720u32.to_le_bytes()); // height
        data[28..32].copy_from_slice(&30000u32.to_le_bytes()); // fps num
        data[32..36].copy_from_slice(&1001u32.to_le_bytes()); // fps den
        data[36..40].copy_from_slice(&0u32.to_le_bytes()); // flags
        data[40..44].copy_from_slice(&1u32.to_le_bytes()); // audio tracks
        data[44..48].copy_from_slice(&1024u32.to_le_bytes()); // track 0 max size
        data[48..50].copy_from_slice(&44100u16.to_le_bytes()); // track 0 sample rate
        data[50] = 0;
        data[51] = 0x10; // stereo

        let header = BinkHeader::read(&data).expect("valid synthetic bink header");
        assert_eq!(&header.magic, b"BIKi");
        assert_eq!(header.file_size, 1008);
        assert_eq!(header.frames, 240);
        assert_eq!(header.width, 1280);
        assert_eq!(header.height, 720);
        assert_eq!(header.audio_tracks, 1);
        assert_eq!(header.audio_sample_rate, 44100);
        assert_eq!(header.audio_channels, 2);
        assert!((header.fps() - 29.97).abs() < 0.01);
        assert!((header.duration() - 8.008).abs() < 0.01);
        assert!((header.aspect_ratio() - 16.0 / 9.0).abs() < 0.001);
    }

    #[test]
    fn rejects_invalid_magic_or_truncated() {
        assert!(BinkHeader::read(&[0u8; 20]).is_err());
        assert!(BinkHeader::read(&[b'R', b'I', b'F', b'F', 0, 0, 0, 0]).is_err());
    }

    #[test]
    #[ignore = "requires owned installed Half-Life 2 copy"]
    fn parses_owned_bink_videos() {
        let Ok(root) = crate::install::discover() else {
            return;
        };
        for (rel, exp_w, exp_h) in [
            ("hl2/media/valve.bik", 1024, 768),
            ("hl2_complete/media/valve.bik", 1920, 1080),
            ("hl2_complete/media/ep1_recap.bik", 1280, 720),
            ("hl2_complete/media/ep2_teaser.bik", 1280, 960),
        ] {
            let path = root.join(rel);
            if !path.exists() {
                continue;
            }
            let bytes = std::fs::read(&path).expect("read bik");
            let h = BinkHeader::read(&bytes).expect("parse bink header");
            assert_eq!(h.width, exp_w);
            assert_eq!(h.height, exp_h);
            assert!(h.frames > 0);
            assert!(h.fps() > 20.0);
            assert!(h.duration() > 5.0);
        }
    }
}
