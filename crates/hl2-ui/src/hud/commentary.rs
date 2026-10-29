//! Developer commentary HUD progress panel (`HudCommentary` in `scripts/HudLayout.res`).

use super::{color, PanelLayout, WeaponHud};
use crate::gameui::localize::Localization;
use anyhow::Result;
use glam::vec2;
use hl2_simulation::commentary::CommentaryOverlay;
use source_assets::{
    keyvalues::{self, Entry},
    vpk::Vfs,
};
use source_ui::canvas::{draw_texture_ex, texture, Color, DrawTextureParams, Texture2D, WHITE};
use std::collections::BTreeMap;

pub(super) type WeaponIconMaps = (
    BTreeMap<String, String>,
    BTreeMap<String, String>,
    BTreeMap<String, String>,
);

pub(super) fn load_weapon_icons(vfs: &Vfs) -> Result<WeaponIconMaps> {
    let mut ammo_icons = BTreeMap::new();
    let mut secondary_ammo_icons = BTreeMap::new();
    let mut weapon_crosshairs = BTreeMap::<String, String>::new();
    for class in [
        "weapon_crowbar",
        "weapon_pistol",
        "weapon_357",
        "weapon_smg1",
        "weapon_ar2",
        "weapon_shotgun",
        "weapon_crossbow",
        "weapon_rpg",
        "weapon_frag",
    ] {
        if let Some(bytes) = vfs.read(&format!("scripts/{class}.txt"))? {
            if let Some(weapon) = keyvalues::parse_resource(&bytes)?.first() {
                if let Some(textures) = weapon.get("TextureData") {
                    for (key, icons) in [
                        ("ammo", &mut ammo_icons),
                        ("ammo2", &mut secondary_ammo_icons),
                        ("crosshair", &mut weapon_crosshairs),
                    ] {
                        if let Some(icon) = textures
                            .get(key)
                            .and_then(|e| e.get("character"))
                            .and_then(Entry::text)
                        {
                            icons.insert(class.into(), icon.into());
                        }
                    }
                }
            }
        }
    }
    Ok((ammo_icons, secondary_ammo_icons, weapon_crosshairs))
}

pub(super) fn load_localization(vfs: &Vfs) -> Localization {
    let mut loc = Localization::default();
    for loc_path in [
        "resource/valve_english.txt",
        "resource/gameui_english.txt",
        "resource/hl2_english.txt",
    ] {
        if let Ok(Some(bytes)) = vfs.read(loc_path) {
            loc.add_resource(loc_path, &bytes);
        }
    }
    loc
}

pub(super) struct CommentaryHud {
    layout: PanelLayout,
    bg_color: Color,
    icon: Option<Texture2D>,
}

impl CommentaryHud {
    pub(super) fn load(vfs: &Vfs, layout: &Entry) -> Self {
        let entry = layout.get("HudCommentary");
        let panel = entry.map(PanelLayout::read).unwrap_or(PanelLayout {
            x: "c-190".into(),
            y: 350.,
            width: 380.,
            height: 40.,
            text: vec2(8., 20.),
            digit: vec2(50., 2.),
            secondary: vec2(98., 16.),
        });
        let bg_color = entry.map_or(Color::from_rgba(0, 0, 0, 128), |e| {
            color(e, "BackgroundOverrideColor", Color::from_rgba(0, 0, 0, 128))
        });
        let icon = texture(vfs, "vgui/hud/icon_commentary").ok();
        Self {
            layout: panel,
            bg_color,
            icon,
        }
    }

    pub(super) fn draw(
        &self,
        hud: &WeaponHud,
        overlay: Option<&CommentaryOverlay>,
        closecaption: bool,
        time: f64,
    ) {
        if closecaption {
            return;
        }
        let Some(overlay) = overlay else {
            return;
        };
        let _frame = hud.framed();
        let viewport = hud.canvas_size();
        let scale = crate::proportional::scale(viewport);
        let rect = self.layout.rect(scale, viewport);
        hud.rounded_box(rect, self.bg_color);
        if let Some(icon) = &self.icon {
            let size = 32. * scale;
            draw_texture_ex(
                &hud.canvas,
                icon,
                rect.x + 6. * scale,
                rect.y + (rect.h - size) * 0.5,
                WHITE,
                DrawTextureParams {
                    dest_size: Some(vec2(size, size)),
                    ..Default::default()
                },
            );
        }
        let bar_x = rect.x + 44. * scale;
        let bar_y = rect.y + 8. * scale;
        let bar_w = (rect.w - 130. * scale).max(60. * scale);
        let bar_h = 8. * scale;
        let duration = (overlay.end_time - overlay.start_time).max(0.1);
        let frac = (((time as f32) - overlay.start_time) / duration).clamp(0.0, 1.0);
        let border = Color::from_rgba(255, 255, 255, 192);
        hud.canvas.rectangle(bar_x, bar_y, bar_w, 1., border);
        hud.canvas
            .rectangle(bar_x, bar_y + bar_h - 1., bar_w, 1., border);
        hud.canvas.rectangle(bar_x, bar_y, 1., bar_h, border);
        hud.canvas
            .rectangle(bar_x + bar_w - 1., bar_y, 1., bar_h, border);
        if frac > 0.0 {
            hud.canvas.rectangle(
                bar_x + 2.,
                bar_y + 2.,
                ((bar_w - 4.) * frac).max(0.0),
                (bar_h - 4.).max(1.0),
                Color::from_rgba(255, 170, 0, 255),
            );
        }
        let count_text = format!("{} \\ {}", overlay.node_number, overlay.node_number_max);
        let count_w = hud.labels.width(&count_text);
        hud.labels.draw(
            &count_text,
            rect.x + rect.w - 10. * scale - count_w,
            rect.y + 5. * scale,
            WHITE,
        );
        if !overlay.speakers.is_empty() {
            hud.labels.draw(
                &overlay.speakers,
                bar_x,
                rect.y + 20. * scale,
                Color::from_rgba(255, 220, 150, 255),
            );
        }
    }
}
