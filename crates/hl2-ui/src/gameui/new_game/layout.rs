//! Interpret HL2's installed dialog/control names and localization for the toolkit.
use super::{Catalog, Localization};
use glam::vec2;
use source_ui::{
    canvas::{Rect, Texture2D},
    card_browser::{BrowserLayout, BrowserSpec, BrowserToggleLayout, Card, ScaledImageLayout},
    layout::Layout,
};

fn rect(layout: &Layout, name: &str, parent: (f32, f32), fallback: Rect) -> Rect {
    layout.get(name).map_or(fallback, |c| {
        let (x, y, w, h) = c.rect(parent);
        Rect::new(x, y, w, h)
    })
}

impl Catalog {
    fn browser_layout(&self) -> BrowserLayout {
        let mut layout = BrowserLayout::default();
        if let Some(c) = self
            .dialog
            .get("NewGameDialog")
            .filter(|c| c.wide > 0. && c.tall > 0.)
        {
            layout.size = vec2(c.wide, c.tall);
        }
        let parent = (layout.size.x, layout.size.y);
        layout.previous = rect(&self.dialog, "Prev", parent, layout.previous);
        layout.next = rect(&self.dialog, "Next", parent, layout.next);
        layout.accept = rect(&self.dialog, "Play", parent, layout.accept);
        layout.cancel = rect(&self.dialog, "Cancel", parent, layout.cancel);
        layout.dividers = ["divider", "Div"]
            .into_iter()
            .filter_map(|name| self.dialog.get(name))
            .map(|c| {
                let (x, y, w, h) = c.rect(parent);
                Rect::new(x, y, w, h)
            })
            .collect();
        if let Some(check) = self.dialog.get("Commentary").filter(|c| c.visible) {
            let (x, y, w, h) = check.rect(parent);
            let backing = self
                .dialog
                .get("ImagePanel1")
                .filter(|c| c.visible)
                .map(|c| {
                    let (bx, by, bw, bh) = c.rect(parent);
                    Rect::new(bx, by, bw, bh)
                });
            let icon = self
                .dialog
                .get("CommentaryIcon")
                .filter(|c| c.visible)
                .map(|c| {
                    let (ix, iy, iw, ih) = c.rect(parent);
                    ScaledImageLayout {
                        rect: Rect::new(ix, iy, iw, ih),
                        scale: c.scale_amount,
                    }
                });
            layout.toggle = Some(BrowserToggleLayout {
                backing,
                icon,
                check_box: Rect::new(x, y, w, h),
            });
        }
        let panel = &mut layout.card;
        let parent = (panel.size.x, panel.size.y);
        panel.heading = rect(&self.panel, "ChapterLabel", parent, panel.heading);
        panel.title = rect(&self.panel, "ChapterNameLabel", parent, panel.title);
        panel.image = rect(&self.panel, "LevelPic", parent, panel.image);
        panel.border = rect(&self.panel, "LevelPicBorder", parent, panel.border);
        panel.badge = self.panel.get("CommentaryIcon").map(|c| {
            let (bx, by, bw, bh) = c.rect(parent);
            ScaledImageLayout {
                rect: Rect::new(bx, by, bw, bh),
                scale: c.scale_amount,
            }
        });
        if panel.border.w > panel.image.w && panel.image.w > 0. && panel.image.h > 0. {
            let pad = ((panel.border.w - panel.image.w) * 0.5).round().max(1.);
            let bottom = panel.border.y + panel.border.h;
            panel.border.h = panel.image.h + 2. * pad;
            panel.border.y = (bottom - panel.border.h).max(panel.title.y + panel.title.h);
            panel.image.x = panel.border.x + pad;
            panel.image.y = panel.border.y + pad;
            panel.size = vec2(panel.border.w, panel.border.y + panel.border.h);
            panel.heading.w = panel.heading.w.min(panel.border.w);
            panel.title.w = panel.title.w.min(panel.border.w);
        }
        if let Some(tall) = self.font_tall {
            panel.font_tall = tall;
        }
        layout
    }

    pub(super) fn browser_spec(&self, loc: &Localization) -> BrowserSpec {
        let label = |name, token| {
            self.dialog
                .get(name)
                .filter(|c| !c.label.is_empty())
                .map_or_else(|| loc.text(token), |c| loc.text(&c.label))
        };
        let cards = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| Card {
                heading: e.heading.clone(),
                title: e.title.clone(),
                image: self
                    .thumbnails
                    .get(i)
                    .and_then(Option::as_ref)
                    .map(|image| Texture2D::from_rgba8(image.width, image.height, &image.rgba)),
            })
            .collect();
        BrowserSpec {
            cards,
            layout: self.browser_layout(),
            title: loc.text(if self.title.is_empty() {
                "#GameUI_NewGame"
            } else {
                &self.title
            }),
            labels: [
                label("Prev", "#GameUI_Prev"),
                label("Next", "#GameUI_Next"),
                label("Play", "#GameUI_StartNewGame"),
                label("Cancel", "#GameUI_Cancel"),
            ],
            toggle_label: self
                .dialog
                .get("Commentary")
                .filter(|c| c.visible)
                .map(|c| {
                    loc.text(if c.label.is_empty() {
                        "#HL2_Enable_Commentary"
                    } else {
                        &c.label
                    })
                }),
            toggle_icon: self
                .commentary_icon
                .as_ref()
                .map(|image| Texture2D::from_rgba8(image.width, image.height, &image.rgba)),
            selection: self.selection,
            fill: self.fill,
            text: self.text,
            release_sound: Some("ui/buttonclickrelease.wav".into()),
        }
    }
}
