//! Moving and resizing dialogs, following the SDK `vgui::Frame` grips.
//!
//! A dialog is dragged by its caption (`frame_caption`: the top `CAPTION_TALL` units, the frame
//! width minus 10, not over the title-bar close button) and, when sizeable, resized by its
//! corner grips (`CORNER_SIZE` square, the bottom-right one `BOTTOM_RIGHT_SIZE`). Like a frame
//! that clips to its parent, the dialog is snapped back fully onto the canvas; one larger than
//! the canvas is pinned at the top-left so its title bar stays reachable. The placement is an
//! offset from the dialog's default position and an optional size, both in layout units, and
//! it is re-clamped to the current canvas and scale every frame, so window resizes and display
//! mode changes never leave a dialog out of reach.
//!
//! UX safety beyond retail: a gesture ends whenever the button is up (also after a release the
//! window missed), [`Placement::cancel`] restores the placement from before the gesture
//! (Escape), and a double click on the caption puts the dialog back to its default position and
//! size.

use crate::canvas::Rect;
use glam::{vec2, Vec2};
use serde::{Deserialize, Serialize};

/// Where a dialog was dragged (and, if sizeable, resized): an offset from its default position
/// and its size, in layout units.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DialogPlacement {
    pub offset: [f32; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[f32; 2]>,
}

/// `Frame::GetCaptionHeight`.
pub const CAPTION_TALL: f32 = 23.;
/// The caption grip is the frame width minus this (`Frame::PerformLayout`).
const CAPTION_INSET: f32 = 10.;
/// `Frame::GetCornerSize`.
pub const CORNER_SIZE: f32 = 8.;
/// `Frame::GetBottomRightSize`.
pub const BOTTOM_RIGHT_SIZE: f32 = 18.;
/// Longest gap between two caption clicks that resets the placement (seconds).
const DOUBLE_CLICK: f64 = 0.5;

/// A resize grip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    /// Which edges move: (-1 left/top moves, +1 right/bottom moves) per axis.
    fn signs(self) -> Vec2 {
        match self {
            Self::TopLeft => vec2(-1., -1.),
            Self::TopRight => vec2(1., -1.),
            Self::BottomLeft => vec2(-1., 1.),
            Self::BottomRight => vec2(1., 1.),
        }
    }
}

/// The pointer shape a placement asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorHint {
    /// The ordinary arrow (retail keeps it over the caption too).
    #[default]
    Arrow,
    /// Over or dragging the top-left or bottom-right grip.
    ResizeNwSe,
    /// Over or dragging the top-right or bottom-left grip.
    ResizeNeSw,
}

impl From<Corner> for CursorHint {
    fn from(corner: Corner) -> Self {
        match corner {
            Corner::TopLeft | Corner::BottomRight => Self::ResizeNwSe,
            Corner::TopRight | Corner::BottomLeft => Self::ResizeNeSw,
        }
    }
}

/// How a dialog may change: its default frame, size limits and grips.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// Default top-left in canvas pixels.
    pub origin: Vec2,
    /// Default size in layout units.
    pub size: Vec2,
    /// Canvas pixels per layout unit.
    pub scale: f32,
    pub canvas: Vec2,
    /// Smallest size in layout units; `None`: not sizeable.
    pub min_size: Option<Vec2>,
    /// The title-bar close button for a frame size, in frame layout units (never a drag handle).
    pub close: fn(Vec2) -> Rect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Gesture {
    Move,
    Resize(Corner),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Active {
    gesture: Gesture,
    /// Pointer at the press, canvas pixels.
    pointer: Vec2,
    /// Placement at the press.
    offset: Vec2,
    size: Vec2,
    /// Clamped frame at the press: top-left in canvas pixels and size in layout units.
    origin: Vec2,
    frame_size: Vec2,
}

/// Where a dialog is: an offset from its default top-left and an optional size, layout units.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placement {
    offset: Vec2,
    size: Option<Vec2>,
    active: Option<Active>,
    /// Time of the last caption press, for double clicks.
    last_caption_press: Option<f64>,
    /// A gesture or reset changed the placement since [`Self::take_changed`].
    changed: bool,
}

/// A frame's clamped top-left (canvas pixels, whole) and size (layout units).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub origin: Vec2,
    pub size: Vec2,
}

impl Placement {
    /// Back to the default position and size.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// A placement saved in the settings.
    pub fn from_saved(saved: Option<DialogPlacement>) -> Self {
        let Some(saved) = saved else {
            return Self::default();
        };
        Self {
            offset: Vec2::from_array(saved.offset),
            size: saved.size.map(Vec2::from_array),
            ..Self::default()
        }
    }

    /// The placement to save; `None` at the default place.
    pub fn saved(&self) -> Option<DialogPlacement> {
        (self.offset != Vec2::ZERO || self.size.is_some()).then(|| DialogPlacement {
            offset: (self.offset * 100.).round().to_array().map(|v| v / 100.),
            size: self.size.map(|s| s.round().to_array()),
        })
    }

    /// Whether a gesture or reset changed the placement since the last call (its owner saves
    /// it then).
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// When a dialog opens: a placement whose frame, as stored, no longer lies fully on the
    /// canvas (the resolution or display mode changed since it was saved) goes back to the
    /// default instead of being pushed to an edge. Returns whether it was reset.
    pub fn reset_if_off_screen(&mut self, frame: &Frame) -> bool {
        let size = match frame.min_size {
            Some(min) => self.size.unwrap_or(frame.size).max(min),
            None => frame.size,
        };
        let origin = frame.origin + self.offset * frame.scale;
        let end = origin + size * frame.scale;
        // Half a pixel for the whole-pixel rounding of placed frames.
        let fits = origin.cmpge(Vec2::splat(-0.5)).all() && end.cmple(frame.canvas + 0.5).all();
        if fits || self.saved().is_none() {
            return false;
        }
        self.reset();
        self.changed = true;
        true
    }

    /// Whether a drag or resize owns the pointer.
    pub fn active(&self) -> bool {
        self.active.is_some()
    }

    /// Ends a gesture, restoring the placement from before it (Escape). Returns whether one was
    /// in progress.
    pub fn cancel(&mut self) -> bool {
        match self.active.take() {
            Some(active) => {
                self.offset = active.offset;
                self.size = (active.size != Vec2::ZERO).then_some(active.size);
                true
            }
            None => false,
        }
    }

    /// Ends a gesture where it is (the dialog closed or lost focus).
    pub fn end(&mut self) {
        self.active = None;
    }

    /// The frame as placed: the stored size within the limits (never larger than the canvas),
    /// then the position snapped fully onto the canvas, or to its top-left when it is larger.
    pub fn place(&self, frame: &Frame) -> Placed {
        let max = (frame.canvas / frame.scale).floor();
        let size = match (frame.min_size, self.size) {
            (Some(min), Some(size)) => size.clamp(min, max.max(min)),
            (Some(min), None) => frame.size.max(min),
            (None, _) => frame.size,
        };
        let pixels = size * frame.scale;
        let origin = frame.origin + self.offset * frame.scale;
        let room = (frame.canvas - pixels).max(Vec2::ZERO);
        Placed {
            origin: origin.clamp(Vec2::ZERO, room).round(),
            size,
        }
    }

    /// The grip or caption under `point` (frame layout units), if any.
    fn handle(frame: &Frame, size: Vec2, point: Vec2) -> Option<Gesture> {
        let inside = |rect: Rect| rect.contains(point);
        if frame.min_size.is_some() {
            let (w, h) = (size.x, size.y);
            let corners = [
                (Corner::TopLeft, Rect::new(0., 0., CORNER_SIZE, CORNER_SIZE)),
                (
                    Corner::TopRight,
                    Rect::new(w - CORNER_SIZE, 0., CORNER_SIZE, CORNER_SIZE),
                ),
                (
                    Corner::BottomLeft,
                    Rect::new(0., h - CORNER_SIZE, CORNER_SIZE, CORNER_SIZE),
                ),
                (
                    Corner::BottomRight,
                    Rect::new(
                        w - BOTTOM_RIGHT_SIZE,
                        h - BOTTOM_RIGHT_SIZE,
                        BOTTOM_RIGHT_SIZE,
                        BOTTOM_RIGHT_SIZE,
                    ),
                ),
            ];
            if let Some((corner, _)) = corners.into_iter().find(|(_, r)| inside(*r)) {
                return Some(Gesture::Resize(corner));
            }
        }
        let caption = Rect::new(0., 0., size.x - CAPTION_INSET, CAPTION_TALL);
        (inside(caption) && !inside((frame.close)(size))).then_some(Gesture::Move)
    }

    /// The pointer shape for `pointer` (canvas pixels).
    pub fn hint(&self, frame: &Frame, pointer: Option<Vec2>) -> CursorHint {
        let gesture = self.active.map(|a| a.gesture).or_else(|| {
            let placed = self.place(frame);
            let point = (pointer? - placed.origin) / frame.scale;
            Self::handle(frame, placed.size, point)
        });
        match gesture {
            Some(Gesture::Resize(corner)) => corner.into(),
            _ => CursorHint::Arrow,
        }
    }

    /// One input frame. A press on the caption starts a move, a press on a grip a resize; while
    /// a gesture lasts the frame follows the pointer. Returns whether the placement owns the
    /// pointer this frame: the dialog then reads nothing else from it (the press that started
    /// the gesture and the release that ends it included).
    pub fn input(
        &mut self,
        frame: &Frame,
        pointer: Option<Vec2>,
        click: bool,
        held: bool,
        time: Option<f64>,
    ) -> bool {
        if let Some(active) = self.active {
            if !held && !click {
                // The release (or a release the window missed) ends the gesture; this frame's
                // pointer is still the gesture's.
                self.active = None;
                self.changed |=
                    self.offset != active.offset || self.size.unwrap_or(Vec2::ZERO) != active.size;
                return true;
            }
            if let Some(pointer) = pointer {
                self.follow(frame, active, pointer);
            }
            return true;
        }
        if !click {
            return false;
        }
        let Some(pointer) = pointer else {
            return false;
        };
        let placed = self.place(frame);
        let point = (pointer - placed.origin) / frame.scale;
        let Some(gesture) = Self::handle(frame, placed.size, point) else {
            return false;
        };
        if gesture == Gesture::Move {
            let double = matches!(
                (time, self.last_caption_press),
                (Some(now), Some(last)) if now - last <= DOUBLE_CLICK
            );
            self.last_caption_press = time;
            if double {
                self.reset();
                self.changed = true;
                return true;
            }
        }
        self.active = Some(Active {
            gesture,
            pointer,
            offset: self.offset,
            size: self.size.unwrap_or(Vec2::ZERO),
            origin: placed.origin,
            frame_size: placed.size,
        });
        true
    }

    fn follow(&mut self, frame: &Frame, active: Active, pointer: Vec2) {
        let delta = (pointer - active.pointer) / frame.scale;
        match active.gesture {
            Gesture::Move => {
                let origin = active.origin + delta * frame.scale;
                // Store the clamped position, so moving back starts at once.
                let placed = Self {
                    offset: (origin - frame.origin) / frame.scale,
                    size: self.size,
                    ..Self::default()
                }
                .place(frame);
                self.offset = (placed.origin - frame.origin) / frame.scale;
            }
            Gesture::Resize(corner) => {
                let Some(min) = frame.min_size else {
                    return;
                };
                let signs = corner.signs();
                // The far edges stay put; the grabbed corner follows the pointer, within the
                // size limits and the canvas.
                let start = active.origin;
                let start_end = start + active.frame_size * frame.scale;
                let max = (frame.canvas / frame.scale).floor().max(min);
                let mut size = (active.frame_size + delta * signs).clamp(min, max);
                let mut origin = vec2(
                    if signs.x < 0. {
                        start_end.x - size.x * frame.scale
                    } else {
                        start.x
                    },
                    if signs.y < 0. {
                        start_end.y - size.y * frame.scale
                    } else {
                        start.y
                    },
                );
                // Growing past a canvas edge: stop at the edge instead of shifting the frame.
                if origin.x < 0. {
                    size.x += origin.x / frame.scale;
                    origin.x = 0.;
                }
                if origin.y < 0. {
                    size.y += origin.y / frame.scale;
                    origin.y = 0.;
                }
                size.x = size
                    .x
                    .min((frame.canvas.x - origin.x) / frame.scale)
                    .max(min.x);
                size.y = size
                    .y
                    .min((frame.canvas.y - origin.y) / frame.scale)
                    .max(min.y);
                self.size = Some(size.round());
                self.offset = (origin - frame.origin) / frame.scale;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 200 x 100 dialog centered on a 1000 x 500 canvas at scale 2.
    fn frame(sizeable: bool) -> Frame {
        Frame {
            origin: vec2(300., 150.),
            size: vec2(200., 100.),
            scale: 2.,
            canvas: vec2(1000., 500.),
            min_size: sizeable.then_some(vec2(120., 60.)),
            close: |size| Rect::new(size.x - 20., 6., 14., 14.),
        }
    }

    fn drag(p: &mut Placement, f: &Frame, from: Vec2, to: Vec2) {
        assert!(p.input(f, Some(from), true, true, None), "press grabs");
        assert!(p.input(f, Some(to), false, true, None), "held follows");
        assert!(
            p.input(f, Some(to), false, false, None),
            "release ends, still owned"
        );
        assert!(!p.active());
    }

    #[test]
    fn the_caption_moves_the_dialog_but_not_its_close_button_or_body() {
        let f = frame(false);
        let mut p = Placement::default();
        assert_eq!(p.place(&f).origin, vec2(300., 150.));
        drag(&mut p, &f, vec2(320., 160.), vec2(220., 200.));
        assert_eq!(p.place(&f).origin, vec2(200., 190.));
        // The close button (layout 180..194, 6..20) is not a handle; the body is not either.
        let close = p.place(&f).origin + vec2(185., 10.) * 2.;
        assert!(!p.input(&f, Some(close), true, true, None));
        let body = p.place(&f).origin + vec2(50., 60.) * 2.;
        assert!(!p.input(&f, Some(body), true, true, None));
        // The caption ends 10 units before the right edge.
        let right = p.place(&f).origin + vec2(195., 10.) * 2.;
        assert!(!p.input(&f, Some(right), true, true, None));
    }

    #[test]
    fn dialogs_stay_on_the_canvas_and_follow_its_size() {
        let f = frame(false);
        let mut p = Placement::default();
        drag(&mut p, &f, vec2(320., 160.), vec2(2000., -500.));
        assert_eq!(
            p.place(&f).origin,
            vec2(600., 0.),
            "snapped fully onto the canvas"
        );
        // Moving back starts at once: the stored offset is the clamped one.
        assert!(p.input(&f, Some(vec2(620., 10.)), true, true, None));
        assert!(p.input(&f, Some(vec2(610., 10.)), false, true, None));
        assert_eq!(p.place(&f).origin, vec2(590., 0.));
        p.end();
        // A smaller window: still fully on it. Larger than the canvas: pinned at the top-left.
        let small = Frame {
            canvas: vec2(500., 300.),
            ..f
        };
        assert_eq!(p.place(&small).origin, vec2(100., 0.));
        let tiny = Frame {
            canvas: vec2(300., 150.),
            ..f
        };
        assert_eq!(p.place(&tiny).origin, Vec2::ZERO);
    }

    #[test]
    fn escape_cancels_a_gesture_and_a_missed_release_ends_it() {
        let f = frame(true);
        let mut p = Placement::default();
        p.input(&f, Some(vec2(320., 160.)), true, true, None);
        p.input(&f, Some(vec2(100., 100.)), false, true, None);
        assert!(p.cancel());
        assert_eq!(p.place(&f).origin, vec2(300., 150.), "restored");
        assert!(!p.cancel());
        // Button up without a release event (pointer left the window): the gesture ends.
        p.input(&f, Some(vec2(320., 160.)), true, true, None);
        assert!(p.input(&f, None, false, false, None));
        assert!(!p.active());
        assert!(
            !p.input(&f, None, false, false, None),
            "then nothing is owned"
        );
    }

    #[test]
    fn a_double_click_on_the_caption_restores_the_default_placement() {
        let f = frame(true);
        let mut p = Placement::default();
        p.input(&f, Some(vec2(320., 160.)), true, true, Some(1.0));
        p.input(&f, Some(vec2(420., 260.)), false, true, Some(1.1));
        p.input(&f, Some(vec2(420., 260.)), false, false, Some(1.2));
        assert_ne!(p.place(&f).origin, vec2(300., 150.));
        p.input(&f, Some(vec2(420., 260.)), true, true, Some(1.4));
        assert_eq!(p.place(&f).origin, vec2(300., 150.));
        assert!(!p.active(), "a reset starts no drag");
    }

    #[test]
    fn corners_resize_within_the_limits_and_keep_the_far_edges() {
        let f = frame(true);
        let mut p = Placement::default();
        // Bottom-right grip (18 units): grow by 50 x 20 units.
        let br = vec2(300., 150.) + vec2(195., 95.) * 2.;
        assert_eq!(p.hint(&f, Some(br)), CursorHint::ResizeNwSe);
        drag(&mut p, &f, br, br + vec2(100., 40.));
        assert_eq!(
            p.place(&f),
            Placed {
                origin: vec2(300., 150.),
                size: vec2(250., 120.)
            }
        );
        // Top-left grip: shrink past the minimum; the bottom-right corner stays.
        let tl = vec2(301., 151.);
        assert_eq!(p.hint(&f, Some(tl)), CursorHint::ResizeNwSe);
        drag(&mut p, &f, tl, tl + vec2(1000., 1000.));
        let placed = p.place(&f);
        assert_eq!(placed.size, vec2(120., 60.));
        assert_eq!(placed.origin + placed.size * 2., vec2(800., 390.));
        // Top-right grip pulled far past the canvas: stops at its edges.
        let tr = placed.origin + vec2(placed.size.x - 2., 2.) * 2.;
        assert_eq!(p.hint(&f, Some(tr)), CursorHint::ResizeNeSw);
        drag(&mut p, &f, tr, tr + vec2(5000., -5000.));
        let placed = p.place(&f);
        assert_eq!(placed.origin.y, 0.);
        assert_eq!(placed.origin.x + placed.size.x * 2., 1000.);
        // Not sizeable: corners are just the caption or the body.
        let fixed = frame(false);
        assert_eq!(
            Placement::default().hint(&fixed, Some(br)),
            CursorHint::Arrow
        );
    }

    #[test]
    fn placements_save_restore_and_reset_when_off_screen() {
        let f = frame(true);
        let mut p = Placement::default();
        assert_eq!(p.saved(), None, "the default place is not saved");
        p.input(&f, Some(vec2(320., 160.)), true, true, None);
        p.input(&f, Some(vec2(100., 100.)), false, true, None);
        assert!(p.cancel());
        assert!(!p.take_changed(), "an undone drag changes nothing");
        drag(&mut p, &f, vec2(320., 160.), vec2(220., 160.));
        assert!(p.take_changed());
        assert!(!p.take_changed(), "reported once");
        let saved = p.saved().unwrap();
        assert_eq!(saved.offset, [-50., 0.]);
        let mut restored = Placement::from_saved(Some(saved));
        assert_eq!(restored.place(&f), p.place(&f));
        assert!(
            !restored.reset_if_off_screen(&f),
            "still fully on the canvas"
        );
        // A smaller resolution would put the frame partly off screen: back to the default.
        let small = Frame {
            canvas: vec2(450., 300.),
            origin: vec2(25., 50.),
            ..f
        };
        assert!(restored.reset_if_off_screen(&small));
        assert_eq!(restored.saved(), None);
        assert!(restored.take_changed(), "the reset is saved too");
        // A resized frame larger than the new canvas resets as well.
        let mut big = Placement::from_saved(Some(DialogPlacement {
            offset: [0., 0.],
            size: Some([450., 240.]),
        }));
        assert!(big.reset_if_off_screen(&f));
    }

    #[test]
    fn a_stored_size_never_exceeds_the_canvas() {
        let f = frame(true);
        let mut p = Placement::default();
        let br = vec2(300., 150.) + vec2(195., 95.) * 2.;
        drag(&mut p, &f, br, br + vec2(400., 100.));
        let smaller = Frame {
            canvas: vec2(400., 200.),
            ..f
        };
        let placed = p.place(&smaller);
        assert!(placed.size.x * 2. <= 400. && placed.size.y * 2. <= 200.);
        assert_eq!(placed.origin, Vec2::ZERO);
    }
}
