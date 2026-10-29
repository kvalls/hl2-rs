//! Resolution and loading of SourceScheme fonts for the GameUI and console widgets.

use super::CellMetrics;
use source_assets::keyvalues;
use source_assets::vpk::Vfs;
use source_ui::canvas::Canvas;
use source_ui::font::FontFace;

pub(crate) struct LoadedUiFonts {
    pub developer: Option<FontFace>,
    pub menu: Option<FontFace>,
    pub menu_metrics: CellMetrics,
    pub ui: Option<FontFace>,
    pub bold: Option<FontFace>,
    pub mono: Option<FontFace>,
    pub marlett: Option<FontFace>,
}

pub(crate) fn load_ui_fonts(vfs: &Vfs, canvas: &Canvas) -> LoadedUiFonts {
    let font = |path: &str| {
        vfs.read(path)
            .ok()
            .flatten()
            .and_then(|bytes| FontFace::load(&bytes, canvas.clone()).ok())
    };
    let developer = FontFace::load(source_ui::PROGGY_CLEAN, canvas.clone()).ok();
    // Menu items and dialog controls draw the source scheme's MenuLarge, Default, and UiBold
    // faces (GorDIN 600, 400, and 700 in the installed scheme) from its registered font files.
    let reader = crate::gameui::resources::Reader::new(vfs);
    let scheme = reader
        .read("resource/SourceScheme.res")
        .and_then(|bytes| keyvalues::parse_resource(&bytes).ok())
        .and_then(|roots| roots.into_iter().next());
    let resolve = |name: &str, fallbacks: &[&str]| {
        scheme
            .as_ref()
            .and_then(|root| source_ui::scheme_fonts::resolve(root, name, |path| reader.read(path)))
            .or_else(|| fallbacks.iter().find_map(|path| reader.read(path)))
    };
    let menu_bytes = resolve(
        "MenuLarge",
        &[
            "resource/linux_fonts/liberationsans-regular.ttf",
            "resource/gordin-regular.ttf",
        ],
    );
    let menu = menu_bytes
        .as_deref()
        .and_then(|bytes| FontFace::load(bytes, canvas.clone()).ok())
        .or_else(|| developer.clone());
    let menu_metrics = menu_bytes
        .as_deref()
        .and_then(CellMetrics::from_bytes)
        .unwrap_or_default();
    let ui = resolve(
        "Default",
        &[
            "resource/gordin-regular.ttf",
            "resource/linux_fonts/liberationsans-regular.ttf",
        ],
    )
    .and_then(|bytes| FontFace::load(&bytes, canvas.clone()).ok())
    .or_else(|| developer.clone());
    let mono = font("resource/linux_fonts/liberationmono-regular.ttf")
        .or_else(|| font("resource/linux_fonts/DejaVuSansMono.ttf"))
        .or_else(|| developer.clone());
    let bold = resolve(
        "UiBold",
        &[
            "resource/gordin-bold.ttf",
            "resource/linux_fonts/liberationsans-bold.ttf",
        ],
    )
    .and_then(|bytes| FontFace::load(&bytes, canvas.clone()).ok())
    .or_else(|| ui.clone());
    // Dialog caption buttons (FrameButton) use Marlett's 'r' glyph (Source SDK Frame.cpp).
    let marlett = resolve("Marlett", &["resource/marlett.ttf"])
        .and_then(|bytes| FontFace::load(&bytes, canvas.clone()).ok());
    LoadedUiFonts {
        developer,
        menu,
        menu_metrics,
        ui,
        bold,
        mono,
        marlett,
    }
}

pub(crate) fn load_title_fonts(console: &mut super::Console, vfs: &Vfs, canvas: &Canvas) {
    let title_bytes = vfs
        .read("resource/HL2EP2.ttf")
        .ok()
        .flatten()
        .or_else(|| vfs.read("resource/hl2ep2.ttf").ok().flatten())
        .or_else(|| {
            vfs.game_root()
                .and_then(|root| std::fs::read(root.join("hl2/resource/hl2ep2.ttf")).ok())
        })
        .or_else(|| vfs.read("resource/HALFLIFE2.ttf").ok().flatten())
        .or_else(|| vfs.read("resource/halflife2.ttf").ok().flatten())
        .or_else(|| {
            vfs.game_root()
                .and_then(|root| std::fs::read(root.join("hl2/resource/halflife2.ttf")).ok())
        });
    if let Some(bytes) = title_bytes {
        console.title_metrics = CellMetrics::from_bytes(&bytes).unwrap_or_default();
        console.title_font = FontFace::load(&bytes, canvas.clone()).ok();
        console.title_face = console.title_font.as_ref().map(FontFace::face);
        if console.title_font.is_some() {
            let is_hl2_logo_font = console
                .title_face
                .as_ref()
                .is_some_and(|f| f.lookup_glyph_index('=') == 0);
            if is_hl2_logo_font {
                // Owned 720px retail capture + GDI probe for halflife2.ttf: requested cell48 -> EM41,
                // native title baseline262 relative to ClientScheme panelY217.
                // Other resolutions scale this measured GDI/label relation; font rasterization remains approximate.
                console.title_metrics = CellMetrics {
                    em_per_cell: 41. / 48.,
                    ascent_per_cell: 45. / 48.,
                };
            }
            console.title = vfs
                .read("gameinfo.txt")
                .ok()
                .flatten()
                .and_then(|bytes| keyvalues::parse_resource(&bytes).ok())
                .and_then(|roots| roots.into_iter().next())
                .and_then(|root| {
                    root.get("title")
                        .and_then(|title| title.text())
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "HALF-LIFE'".into());
        } else {
            console.title_face = None;
            console.title_metrics = CellMetrics::default();
            console.category_titles =
                crate::gameui::resources::fallback_category_titles(&console.gameui.localization);
            console.log(
                "Owned HalfLife2 title font could not be loaded; using the HL2-RS fallback title.",
            );
        }
    } else {
        console.category_titles =
            crate::gameui::resources::fallback_category_titles(&console.gameui.localization);
        console.log("Owned HalfLife2 title font not found; using the HL2-RS fallback title.");
    }

    let lc_bytes = vfs
        .read("lostcoast/resource/lostcoast.ttf")
        .ok()
        .flatten()
        .or_else(|| vfs.read("resource/lostcoast.ttf").ok().flatten())
        .or_else(|| {
            vfs.game_root()
                .and_then(|root| std::fs::read(root.join("lostcoast/resource/lostcoast.ttf")).ok())
        });
    if let Some(bytes) = lc_bytes {
        console.lostcoast_title_metrics = CellMetrics::from_bytes(&bytes).unwrap_or_default();
        console.lostcoast_title_font = FontFace::load(&bytes, canvas.clone()).ok();
        console.lostcoast_title_face = console.lostcoast_title_font.as_ref().map(FontFace::face);
    }
}
