//! The console dialog's geometry, history row wrapping and output row selection.
//!
//! Layout follows the SDK `CConsoleDialog` / `CConsolePanel::PerformLayout`: a titled frame whose
//! client area holds the history (`RichText`, `DepressedButtonBorder`), the entry and the Submit
//! button along the bottom. The panel is not proportional, so its insets are plain layout units.
//! The default placement is the retail one: proportional offsets (480-line basis) of 16 and 64
//! put the frame at `(w/2 - 16, 64)`, half the screen wide and `h - 2 * 64` tall. On a screen
//! narrower than the UI design (square, portrait) half the width is too narrow and the full
//! height too tall, so the frame spans the width between the 16 margins and ends where the
//! design area ends ([`Scaling::design_height`]), keeping the proportions of the retail frame on
//! a 5:4 screen.

use crate::canvas::Rect;
use crate::widgets::{close_button_rect, CLIENT_INSET_X, CLIENT_TOP};
use glam::{vec2, Vec2};
use std::ops::Range;

/// `Frame.ClientInsetY`: space under the client area.
const CLIENT_INSET_Y: f32 = 6.;
/// `CConsolePanel::PerformLayout` insets.
const INSET: f32 = 8.;
pub const ENTRY_TALL: f32 = 24.;
const TOP_TALL: f32 = 4.;
const ENTRY_INSET: f32 = 4.;
pub const SUBMIT_WIDE: f32 = 64.;
/// Pulls the Submit button away from the frame's resize grip.
const SUBMIT_INSET: f32 = 7.;
/// Retail proportional values are relative to a 480-line screen.
pub const PROPORTIONAL_BASE: f32 = 480.;
/// Smallest size the corner grips allow: the SDK `CConsolePanel` minimum (100 x 100), raised so the
/// entry keeps at least a Submit button's width and the history three entry rows.
pub const MIN_SIZE: Vec2 = Vec2::new(
    2. * CLIENT_INSET_X + 3. * INSET + ENTRY_INSET + SUBMIT_WIDE + SUBMIT_INSET + SUBMIT_WIDE,
    CLIENT_TOP
        + CLIENT_INSET_Y
        + 2. * ENTRY_INSET
        + 2. * INSET
        + TOP_TALL
        + ENTRY_TALL
        + 3. * ENTRY_TALL,
);

/// How an app sizes its dialogs on a canvas.
#[derive(Clone, Copy, Debug)]
pub struct Scaling {
    /// Canvas pixels per layout unit.
    pub dialog_scale: fn(Vec2) -> f32,
    /// The height the app's 480-line design spans: the canvas height, or less on screens
    /// narrower than the design.
    pub design_height: fn(Vec2) -> f32,
}

/// Where the console dialog and its controls are. Control rectangles are in layout units
/// relative to the frame; [`Self::canvas`] maps them to canvas pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DialogLayout {
    /// Frame top-left in canvas pixels.
    pub origin: Vec2,
    /// Canvas pixels per layout unit ([`Scaling::dialog_scale`]).
    pub scale: f32,
    /// Frame size in layout units.
    pub size: Vec2,
    pub history: Rect,
    pub entry: Rect,
    pub submit: Rect,
    pub close: Rect,
}

impl DialogLayout {
    /// The retail placement on `canvas`.
    pub fn new(canvas: Vec2, scaling: Scaling) -> Self {
        let (origin, size) = Self::default_frame(canvas, scaling);
        Self::at((scaling.dialog_scale)(canvas), origin, size)
    }

    /// The retail frame on `canvas`: top-left in canvas pixels and size in layout units.
    pub fn default_frame(canvas: Vec2, scaling: Scaling) -> (Vec2, Vec2) {
        let scale = (scaling.dialog_scale)(canvas);
        let design = (scaling.design_height)(canvas);
        let proportional = |value: f32| (value * design / PROPORTIONAL_BASE).floor();
        let (origin, extent) = if design < canvas.y {
            (
                vec2(proportional(16.), proportional(64.)),
                vec2(
                    canvas.x - 2. * proportional(16.),
                    design - 2. * proportional(64.),
                ),
            )
        } else {
            (
                vec2(
                    (canvas.x * 0.5).floor() - proportional(16.),
                    proportional(64.),
                ),
                vec2((canvas.x * 0.5).floor(), canvas.y - 2. * proportional(64.)),
            )
        };
        (
            (origin).max(Vec2::ZERO),
            (extent / scale).round().max(MIN_SIZE),
        )
    }

    /// The layout of a frame at `origin` (canvas pixels) of `size` (layout units).
    pub fn at(scale: f32, origin: Vec2, size: Vec2) -> Self {
        let size = size.max(MIN_SIZE);
        // Frame client area (`Frame::GetClientArea`), then the panel's own insets inside it.
        let (cx, cy) = (CLIENT_INSET_X, CLIENT_TOP);
        let (cw, ch) = (
            size.x - 2. * CLIENT_INSET_X,
            size.y - CLIENT_INSET_Y - CLIENT_TOP,
        );
        let history = Rect::new(
            cx + INSET,
            cy + INSET + TOP_TALL,
            cw - 2. * INSET,
            (ch - (ENTRY_INSET * 2. + INSET * 2. + TOP_TALL + ENTRY_TALL)).max(0.),
        );
        let bottom = cy + ch - (ENTRY_INSET * 2. + ENTRY_TALL);
        let submit_x = cw - (INSET + SUBMIT_WIDE + SUBMIT_INSET);
        Self {
            origin,
            scale,
            size,
            history,
            entry: Rect::new(
                cx + INSET,
                bottom,
                (submit_x - ENTRY_INSET - 2. * INSET).max(0.),
                ENTRY_TALL,
            ),
            submit: Rect::new(cx + submit_x, bottom, SUBMIT_WIDE, ENTRY_TALL),
            close: close_button_rect(size),
        }
    }

    /// A frame-relative layout rectangle in canvas pixels.
    pub fn canvas(&self, rect: Rect) -> Rect {
        Rect::new(
            (self.origin.x + rect.x * self.scale).round(),
            (self.origin.y + rect.y * self.scale).round(),
            (rect.w * self.scale).round(),
            (rect.h * self.scale).round(),
        )
    }

    /// A canvas point in frame-relative layout units.
    pub fn local(&self, point: Vec2) -> Vec2 {
        (point - self.origin) / self.scale
    }
}

/// Splits `text` into history rows at most `width` wide, as byte ranges into it, breaking the
/// way SDK `RichText::RecalculateLineBreaks` does: `\n` (`\r\n`, `\r`) ends a row; a row that
/// overflows breaks before the word that overflowed, or at the overflowing character when that
/// word started the row. Only whitespace separates words. Rows continuing a wrapped line skip
/// their leading spaces (retail skips them when drawing); rows after a newline keep them.
/// Every row holds at least one character, so no width stalls. Empty text is one empty row,
/// and one trailing newline adds none.
pub fn wrap(text: &str, width: f32, advance: impl Fn(char) -> f32) -> Vec<Range<usize>> {
    let end = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix(['\n', '\r']))
        .unwrap_or(text)
        .len();
    let mut rows = Vec::new();
    let mut start = 0;
    loop {
        let (stop, next) = match text[start..end].find(['\n', '\r']) {
            Some(i) => {
                let i = start + i;
                let next = if text[i..].starts_with("\r\n") {
                    i + 2
                } else {
                    i + 1
                };
                (i, Some(next))
            }
            None => (end, None),
        };
        wrap_segment(text, start..stop, width, &advance, &mut rows);
        match next {
            Some(next) => start = next,
            None => return rows,
        }
    }
}

/// [`wrap`] for text without newlines.
fn wrap_segment(
    text: &str,
    segment: Range<usize>,
    width: f32,
    advance: &impl Fn(char) -> f32,
    rows: &mut Vec<Range<usize>>,
) {
    let trimmed = |from: usize, to: usize| from + text[from..to].trim_end().len();
    let measure = |from: usize, to: usize| text[from..to].chars().map(advance).sum::<f32>();
    let pushed = rows.len();
    let (mut row, mut word, mut x) = (segment.start, segment.start, 0.);
    let mut in_word = false;
    let mut chars = text[segment.clone()]
        .char_indices()
        .map(|(i, c)| (segment.start + i, c))
        .peekable();
    while let Some((i, c)) = chars.next() {
        let space = c.is_whitespace();
        if !space && !in_word {
            word = i;
        }
        in_word = !space;
        let w = advance(c);
        if i == row || x + w <= width {
            x += w;
        } else if space {
            // The row ends at the space; the next one starts at the following word.
            rows.push(row..trimmed(row, i));
            row = segment.end;
            while let Some(&(j, d)) = chars.peek() {
                if !d.is_whitespace() {
                    row = j;
                    break;
                }
                chars.next();
            }
            x = 0.;
        } else if word > row {
            // The word moves to the next row, and breaks there when it is too long for one.
            rows.push(row..trimmed(row, word));
            let before = measure(word, i);
            if word < i && before + w > width {
                rows.push(word..i);
                (row, x) = (i, w);
            } else {
                (row, x) = (word, before + w);
            }
        } else {
            rows.push(row..i);
            (row, x) = (i, w);
        }
    }
    if row < segment.end || rows.len() == pushed {
        rows.push(row..segment.end);
    }
}

/// Selected output rows, by line number since the console started, so a selection stays on its
/// lines while new output arrives and old lines scroll out of the buffer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    /// Where the drag started, and where it is now.
    pub anchor: u64,
    pub head: u64,
}

impl Selection {
    pub fn at(line: u64) -> Self {
        Self {
            anchor: line,
            head: line,
        }
    }

    /// Whether `line` is selected.
    pub fn contains(&self, line: u64) -> bool {
        (self.anchor.min(self.head)..=self.anchor.max(self.head)).contains(&line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selections_cover_their_span_in_either_direction() {
        let down = Selection { anchor: 3, head: 5 };
        let up = Selection { anchor: 5, head: 3 };
        for selection in [down, up] {
            assert!(!selection.contains(2));
            assert!((3..=5).all(|line| selection.contains(line)));
            assert!(!selection.contains(6));
        }
        assert!(Selection::at(7).contains(7));
    }

    /// `text` wrapped `width` characters wide, one unit per character.
    fn rows(text: &str, width: usize) -> Vec<&str> {
        wrap(text, width as f32, |_| 1.)
            .into_iter()
            .map(|range| &text[range])
            .collect()
    }

    #[test]
    fn rows_break_before_the_word_that_overflows() {
        assert_eq!(rows("one two three", 9), ["one two", "three"]);
        assert_eq!(rows("one two three", 7), ["one two", "three"]);
        // Exactly as wide as the row: no break.
        assert_eq!(rows("one two", 7), ["one two"]);
        // Continuation rows lose their leading spaces, wherever the break fell.
        assert_eq!(rows("one    two", 5), ["one", "two"]);
        assert_eq!(rows("abc def", 3), ["abc", "def"]);
        // Punctuation does not separate words.
        assert_eq!(rows("a entity:2224v1", 10), ["a", "entity:222", "4v1"]);
    }

    #[test]
    fn long_words_break_at_the_character_that_overflows() {
        assert_eq!(rows("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(rows("ab cdefghij", 4), ["ab", "cdef", "ghij"]);
        // A word moved to the next row breaks again once it fills that row.
        assert_eq!(rows("abc defghi", 5), ["abc", "defgh", "i"]);
        // Every row takes at least one character, whatever the width.
        assert_eq!(rows("abc", 0), ["a", "b", "c"]);
        // Multi-byte characters stay whole.
        assert_eq!(rows("ääää", 2), ["ää", "ää"]);
    }

    #[test]
    fn newlines_start_rows_and_keep_their_indentation() {
        assert_eq!(
            rows("one\ntwo\r\nthree\rfour", 20),
            ["one", "two", "three", "four"]
        );
        assert_eq!(rows("one\n  two", 20), ["one", "  two"]);
        assert_eq!(rows("one\n\ntwo", 20), ["one", "", "two"]);
        assert_eq!(rows("one\n", 20), ["one"], "a trailing newline adds no row");
        assert_eq!(rows("", 20), [""]);
        assert_eq!(rows("\n", 20), [""]);
        assert_eq!(rows("ab cd\nefgh", 3), ["ab", "cd", "efg", "h"]);
        // Byte ranges cover the text in order without the line breaks.
        let text = "Encountered an error\nEntity despawned: invalid";
        let ranges = wrap(text, 12., |_| 1.);
        assert!(ranges.windows(2).all(|pair| pair[0].end <= pair[1].start));
        assert_eq!(ranges.last().unwrap().end, text.len());
    }

    #[test]
    fn rows_measure_with_the_given_advances() {
        // Wide characters count by their advance.
        let advance = |c: char| if c == 'W' { 3. } else { 1. };
        let wrapped = |text: &'static str, width: f32| -> Vec<&'static str> {
            wrap(text, width, advance)
                .into_iter()
                .map(|range| &text[range])
                .collect()
        };
        assert_eq!(wrapped("WW ii WW", 6.), ["WW", "ii", "WW"]);
        // A moved word whose overflowing character does not fit beside it breaks there.
        assert_eq!(wrapped("a bcdW", 5.), ["a", "bcd", "W"]);
    }
}
