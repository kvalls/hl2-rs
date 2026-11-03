//! Setup-only Windows font rasterization for owned monochrome crosshair icons.
//! No native game code, persistent GDI handles or frame-system file reads.
use crate::canvas::{FilterMode, Texture2D};
use anyhow::{bail, ensure, Context, Result};
use glam::{vec2, Vec2};
use std::{
    collections::{BTreeMap, BTreeSet},
    ptr::null_mut,
};
use windows_sys::Win32::{Foundation::HANDLE, Graphics::Gdi::*};

pub struct Glyph {
    pub texture: Option<Texture2D>,
    pub origin: Vec2,
    pub advance: i32,
    pub height: i32,
}
struct Registration(HANDLE);
impl Drop for Registration {
    fn drop(&mut self) {
        // SAFETY: owned process-private font registration; all DCs have been released.
        unsafe {
            RemoveFontMemResourceEx(self.0);
        }
    }
}
struct ContextDc {
    dc: HDC,
    font: HFONT,
    previous: HGDIOBJ,
}
impl Drop for ContextDc {
    fn drop(&mut self) {
        // SAFETY: restore the original object before deleting the owned font and DC.
        unsafe {
            if !self.previous.is_null() && self.previous as isize != -1 {
                SelectObject(self.dc, self.previous);
            }
            if !self.font.is_null() {
                DeleteObject(self.font);
            }
            DeleteDC(self.dc);
        }
    }
}
fn decode_bitmap(width: usize, height: usize, bitmap: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        width <= 2048 && height <= 2048,
        "crosshair glyph bitmap exceeds budget"
    );
    let stride = width.div_ceil(32) * 4;
    ensure!(
        bitmap.len() >= stride * height,
        "truncated GDI crosshair bitmap"
    );
    Ok((0..height)
        .flat_map(|y| {
            (0..width).flat_map(move |x| {
                let alpha = if bitmap[y * stride + x / 8] & (0x80 >> (x % 8)) != 0 {
                    255
                } else {
                    0
                };
                [255, 255, 255, alpha]
            })
        })
        .collect())
}

pub fn load(
    bytes: &[u8],
    requests: &BTreeSet<(char, u16)>,
) -> Result<BTreeMap<(char, u16), Glyph>> {
    ensure!(
        bytes.len() <= 16 * 1024 * 1024 && requests.len() <= 64,
        "crosshair font setup exceeds budget"
    );
    if requests.is_empty() {
        return Ok(BTreeMap::new());
    }
    let face = ttf_parser::Face::parse(bytes, 0).context("owned crosshair font metadata")?;
    let family = face
        .names()
        .into_iter()
        .filter(|name| name.name_id == ttf_parser::name_id::FAMILY)
        .find_map(|name| name.to_string())
        .context("owned crosshair font family")?;
    let family_w: Vec<_> = family.encode_utf16().chain(Some(0)).collect();
    ensure!(family_w.len() <= 32, "crosshair font family too long");
    // SAFETY: GDI copies the valid bounded in-memory font. Registration is private
    // to this process and removed by RAII after every requested bitmap is copied.
    let registered = unsafe {
        let mut count = 0;
        // The binding types this output pointer as const; GDI writes the count.
        let count_pointer = &raw mut count;
        AddFontMemResourceEx(
            bytes.as_ptr().cast_mut().cast(),
            bytes.len() as u32,
            null_mut(),
            count_pointer,
        )
    };
    ensure!(
        !registered.is_null(),
        "cannot register owned crosshair font with GDI"
    );
    let _registered = Registration(registered);
    let mut output = BTreeMap::new();
    for &tall in requests
        .iter()
        .map(|(_, tall)| tall)
        .collect::<BTreeSet<_>>()
    {
        ensure!(
            (1..=1024).contains(&tall),
            "crosshair font height outside budget"
        );
        // SAFETY: locally owned memory DC/font, integer bounded height and terminated family.
        let dc = unsafe { CreateCompatibleDC(null_mut()) };
        ensure!(!dc.is_null(), "cannot create crosshair font DC");
        let mut context = ContextDc {
            dc,
            font: null_mut(),
            previous: null_mut(),
        };
        context.font = unsafe {
            CreateFontW(
                i32::from(tall),
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                u32::from(ANSI_CHARSET),
                u32::from(OUT_DEFAULT_PRECIS),
                u32::from(CLIP_DEFAULT_PRECIS),
                u32::from(NONANTIALIASED_QUALITY),
                u32::from(DEFAULT_PITCH),
                family_w.as_ptr(),
            )
        };
        ensure!(
            !context.font.is_null(),
            "cannot create owned crosshair font"
        );
        context.previous = unsafe { SelectObject(dc, context.font) };
        ensure!(
            !context.previous.is_null() && context.previous as isize != -1,
            "cannot select crosshair font"
        );
        let mut selected = [0u16; 64];
        let length = unsafe { GetTextFaceW(dc, selected.len() as i32, selected.as_mut_ptr()) };
        ensure!(length > 0, "cannot check selected crosshair font");
        let selected = String::from_utf16_lossy(
            &selected[..selected
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(selected.len())],
        );
        ensure!(
            selected.eq_ignore_ascii_case(&family),
            "GDI substituted the owned crosshair font"
        );
        let mut metrics = TEXTMETRICW::default();
        ensure!(
            unsafe { GetTextMetricsW(dc, &mut metrics) } != 0,
            "crosshair text metrics failed"
        );
        let matrix = MAT2 {
            eM11: FIXED { fract: 0, value: 1 },
            eM12: FIXED { fract: 0, value: 0 },
            eM21: FIXED { fract: 0, value: 0 },
            eM22: FIXED { fract: 0, value: 1 },
        };
        for &(character, height) in requests.iter().filter(|(_, height)| *height == tall) {
            let mut glyph = GLYPHMETRICS::default();
            // SAFETY: valid selected font, output structs and identity transform.
            let required = unsafe {
                GetGlyphOutlineW(
                    dc,
                    character as u32,
                    GGO_BITMAP,
                    &mut glyph,
                    0,
                    null_mut(),
                    &matrix,
                )
            };
            if required == GDI_ERROR as u32 {
                bail!("cannot rasterize owned crosshair glyph {character:?}");
            }
            ensure!(
                required <= 4 * 1024 * 1024,
                "crosshair glyph bytes exceed budget"
            );
            let texture = if required == 0 || glyph.gmBlackBoxX == 0 || glyph.gmBlackBoxY == 0 {
                None
            } else {
                let mut bitmap = vec![0; required as usize];
                // SAFETY: destination has exactly the required bounded length.
                ensure!(
                    unsafe {
                        GetGlyphOutlineW(
                            dc,
                            character as u32,
                            GGO_BITMAP,
                            &mut glyph,
                            required,
                            bitmap.as_mut_ptr().cast(),
                            &matrix,
                        )
                    } != GDI_ERROR as u32,
                    "crosshair bitmap read failed"
                );
                let rgba = decode_bitmap(
                    glyph.gmBlackBoxX as usize,
                    glyph.gmBlackBoxY as usize,
                    &bitmap,
                )?;
                let texture = Texture2D::from_rgba8(
                    glyph.gmBlackBoxX as u16,
                    glyph.gmBlackBoxY as u16,
                    &rgba,
                );
                texture.set_filter(FilterMode::Nearest);
                Some(texture)
            };
            output.insert(
                (character, height),
                Glyph {
                    texture,
                    origin: vec2(
                        glyph.gmptGlyphOrigin.x as f32,
                        (metrics.tmAscent - glyph.gmptGlyphOrigin.y) as f32,
                    ),
                    advance: i32::from(glyph.gmCellIncX),
                    height: metrics.tmHeight,
                },
            );
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn monochrome_rows_use_most_significant_bits_and_dword_padding() {
        let bitmap = [0x81, 0x80, 0xff, 0xff, 0x40, 0x00, 0xff, 0xff];
        let rgba = decode_bitmap(9, 2, &bitmap).unwrap();
        let alpha: Vec<_> = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| pixel[3])
            .collect();
        assert_eq!(
            alpha,
            [255, 0, 0, 0, 0, 0, 0, 255, 255, 0, 255, 0, 0, 0, 0, 0, 0, 0]
        );
        assert!(decode_bitmap(9, 2, &bitmap[..7]).is_err());
        assert!(decode_bitmap(2049, 1, &[]).is_err());
    }
}
