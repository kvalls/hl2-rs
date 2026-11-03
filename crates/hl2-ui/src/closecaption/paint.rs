//! Caption viewport layout and animation.
use super::{
    compute_work, CloseCaption, Item, Layout, Measure, Paint, Run, INSET, MIN_VISIBLE_ITEMS,
    PAN_FADE_TIME, PAN_SLIDE_TIME,
};

impl CloseCaption {
    /// Paint for a panel of `size` pixels at proportional `scale` (screen height / 480) on
    /// the game clock `time`.
    pub fn paint(
        &mut self,
        size: (f32, f32),
        scale: f32,
        layout: &Layout,
        measure: &dyn Measure,
        background: [f32; 4],
        time: f64,
    ) -> Paint {
        let (w, h) = size;
        let window_top = (layout.top_offset * scale).floor();
        let window_bottom = h;
        let inset = (INSET * scale).floor();
        let available_width = (w - 2. * inset - 3.).max(1.);
        let available_height = (h - window_top - 2. * inset).max(measure.tall());
        let metrics = (available_width, measure.tall());
        if self.work_metrics != Some(metrics) {
            for item in &mut self.items {
                item.work = None;
            }
            self.work_metrics = Some(metrics);
        }
        let line_height = measure.tall().max(6.);
        let (visible, total, growing_down) =
            self.prepare(available_width, available_height, layout, measure, time);
        if visible.is_empty() && self.current_alpha == 0. {
            return Paint::default();
        }
        let mut paint = Paint::default();
        let bottom = h;
        let top = bottom - self.current_height - 2. * inset;
        let box_top = top.max(window_top);
        let mut color = background;
        color[3] = layout.bg_alpha / 255. * self.current_alpha;
        paint.background = Some(([0., box_top, w, bottom - box_top], color));
        if visible.is_empty() {
            return paint;
        }
        let left = inset;
        let text_height = if growing_down {
            total
        } else {
            self.current_height
        };
        let mut text_top = bottom - text_height - inset;
        for (position, &index) in visible.iter().enumerate() {
            let item = &mut self.items[index];
            // The oldest item drops the extra time it was created with.
            if position == 0 && item.added_time > 0. {
                item.time_to_live = (item.time_to_live - item.added_time).max(0.);
                item.added_time = 0.;
            }
            let height = item.height;
            let mut fade_line = None;
            let mut fade_line_alpha = 1.;
            if height > available_height {
                let age = item.initial_life_span - item.time_to_live;
                let (offset, fading, opacity) = pan(item, available_height, line_height, age);
                fade_line = fading;
                fade_line_alpha = opacity;
                text_top = window_top + inset - offset;
            }
            let alpha = item.alpha(
                layout.item_hidden_time,
                layout.item_fade_in_time,
                layout.item_fade_out_time,
            );
            for unit in item.work.as_ref().unwrap() {
                let x = left + unit.x + 3.;
                let y = text_top + unit.y;
                let mut line_alpha = alpha;
                if fade_line == Some((unit.y / line_height).round() as usize) {
                    line_alpha *= fade_line_alpha;
                }
                if y < box_top + inset || y + unit.height > window_bottom - inset {
                    continue;
                }
                let (color, highlight) = run_colors(unit.color, item.valid, line_alpha);
                paint.runs.push(Run {
                    text: unit.text.clone(),
                    x,
                    y,
                    width: unit.width + if item.valid { 0. } else { 2. },
                    height: unit.height,
                    style: unit.style,
                    color,
                    highlight,
                });
            }
            text_top += height;
            if text_top >= window_bottom {
                break;
            }
        }
        paint
    }
    fn prepare(
        &mut self,
        available_width: f32,
        available_height: f32,
        layout: &Layout,
        measure: &dyn Measure,
        time: f64,
    ) -> (Vec<usize>, f32, bool) {
        let line_height = measure.tall().max(6.);
        let mut total = 0.;
        let mut visible: Vec<usize> = Vec::new();
        for index in 0..self.items.len() {
            if self.items[index].predisplay > 0. {
                continue;
            }
            if self.items[index].work.is_none() {
                compute_work(&mut self.items[index], available_width, measure);
            }
            let height = self.items[index].height;
            total += height;
            visible.push(index);
            // Start popping really old items off the stack when out of space.
            while total > available_height && visible.len() > MIN_VISIBLE_ITEMS {
                let popped = visible.remove(0);
                total -= self.items[popped].height;
                self.items[popped].time_to_live = 0.;
            }
        }
        let desired_alpha = if visible.is_empty() { 0. } else { 1. };
        let total = f32::max(total.min(available_height), line_height);
        if total != self.goal_height {
            self.goal_height = total;
            self.goal_start = time;
            self.goal_finish = time + f64::from(layout.grow_time);
        }
        if desired_alpha != self.goal_alpha {
            self.goal_alpha = desired_alpha;
            self.goal_start = time;
            self.goal_finish = time + f64::from(layout.grow_time);
        }
        if visible.is_empty()
            && self.goal_height == self.current_height
            && self.goal_alpha == self.current_alpha
        {
            self.goal_start = 0.;
            self.goal_finish = 0.;
            return (visible, total, false);
        }
        let mut growing_down = false;
        if self.goal_finish != 0. && self.goal_start != 0. && self.goal_finish > self.goal_start {
            let to_go = self.goal_height - self.current_height;
            let alpha_to_go = self.goal_alpha - self.current_alpha;
            growing_down = to_go < 0.;
            let frac = ((time - self.goal_start) / (self.goal_finish - self.goal_start))
                .clamp(0., 1.) as f32;
            self.current_height += (frac * to_go).trunc();
            self.current_alpha = (self.current_alpha + frac * alpha_to_go).clamp(0., 1.);
        } else {
            self.current_height = self.goal_height;
            self.current_alpha = self.goal_alpha;
        }
        (visible, total, growing_down)
    }
}

/// Pan by visual lines, so color and style runs on one line move together.
pub(super) fn pan(item: &Item, available: f32, tall: f32, age: f32) -> (f32, Option<usize>, f32) {
    let steps = ((item.height - available) / tall).ceil().max(1.) as usize;
    let interval = item.initial_life_span / (steps + 1) as f32;
    let step = (age / interval.max(0.001)).floor().clamp(0., steps as f32) as usize;
    if step == 0 {
        return (0., None, 1.);
    }
    let elapsed = age - interval * step as f32;
    let base = (step - 1) as f32 * tall;
    let fade = (1. - elapsed / PAN_FADE_TIME).clamp(0., 1.);
    let slide = ((elapsed - PAN_FADE_TIME) / PAN_SLIDE_TIME).clamp(0., 1.);
    (base + tall * slide, Some(step - 1), fade)
}

fn rgba(c: [u8; 4], a: f32) -> [f32; 4] {
    [
        f32::from(c[0]) / 255.,
        f32::from(c[1]) / 255.,
        f32::from(c[2]) / 255.,
        f32::from(c[3]) / 255. * a,
    ]
}

fn run_colors(color: [u8; 4], valid: bool, alpha: f32) -> ([f32; 4], Option<[f32; 4]>) {
    if valid {
        (rgba(color, alpha), None)
    } else {
        (
            rgba([255; 4], alpha),
            Some(rgba([100, 100, 40, 255], alpha)),
        )
    }
}
