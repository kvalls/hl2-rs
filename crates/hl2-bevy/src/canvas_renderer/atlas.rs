//! Shared pages for small canvas textures (glyphs, small icons) and a white block, so runs of
//! text and solid rectangles share one material and draw as one batch instead of one mesh and
//! draw per glyph. Entries keep edge-replicated padding, so sampling inside a page matches the
//! clamp-to-edge sampling of a texture of its own; pages have no mipmaps.
use super::HudMaterial;
use bevy::{
    asset::RenderAssetUsages,
    image::{ImageSampler, ImageSamplerDescriptor},
    math::UVec2,
    prelude::{Assets, Handle, Image},
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use source_ui::canvas::{FilterMode, Quad, Texture2D};
use std::collections::HashMap;

/// Page side in pixels: a new entry re-uploads its whole page, so pages stay small (1 MiB).
const PAGE: u32 = 512;
/// Edge-replicated border around every entry, wide enough for bilinear taps.
const PAD: u32 = 2;
/// Largest texture side placed in a page; larger textures keep their own material.
const MAX_ENTRY: u16 = 128;
/// Side of the white block every page starts with (for untextured quads).
const WHITE: u32 = 4;

/// How a quad's texture coordinates map into the texture it is drawn from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum UvMap {
    /// Its own texture: unchanged.
    Own,
    /// A page entry: `offset + uv * scale`.
    Page { offset: [f32; 2], scale: [f32; 2] },
    /// The page's white block: one point inside it.
    White([f32; 2]),
}
impl UvMap {
    pub(super) fn apply(self, uvs: &mut [[f32; 2]; 4]) {
        match self {
            Self::Own => {}
            Self::Page { offset, scale } => {
                for uv in uvs {
                    *uv = [offset[0] + uv[0] * scale[0], offset[1] + uv[1] * scale[1]];
                }
            }
            Self::White(point) => *uvs = [point; 4],
        }
    }
}

struct Page {
    filter: FilterMode,
    image: Handle<Image>,
    normal: Handle<HudMaterial>,
    additive: Handle<HudMaterial>,
    /// Shelf packer: next free x, top of the current row, height of the current row.
    cursor: (u32, u32, u32),
}
impl Page {
    fn new(
        filter: FilterMode,
        images: &mut Assets<Image>,
        materials: &mut Assets<HudMaterial>,
    ) -> Self {
        let mut image = Image::new(
            Extent3d {
                width: PAGE,
                height: PAGE,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            vec![0; (PAGE * PAGE * 4) as usize],
            TextureFormat::Rgba8Unorm,
            // Kept in the main world so entries are written in place.
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        image.sampler = ImageSampler::Descriptor(match filter {
            FilterMode::Nearest => ImageSamplerDescriptor::nearest(),
            FilterMode::Linear => ImageSamplerDescriptor::linear(),
        });
        let side = WHITE + 2 * PAD;
        if let Some(data) = image.data.as_mut() {
            for y in 0..side {
                let row = (y * PAGE * 4) as usize;
                data[row..row + (side * 4) as usize].fill(255);
            }
        }
        let image = images.add(image);
        let material = |additive| HudMaterial {
            texture: image.clone(),
            additive,
            modulate: false,
        };
        Self {
            filter,
            normal: materials.add(material(false)),
            additive: materials.add(material(true)),
            image,
            cursor: (side, 0, side),
        }
    }
    /// Where `size` plus padding would go and the packer state after it; `None` when full.
    fn slot(&self, size: UVec2) -> Option<(UVec2, (u32, u32, u32))> {
        let (w, h) = (size.x + 2 * PAD, size.y + 2 * PAD);
        let (mut x, mut y, mut row) = self.cursor;
        if x + w > PAGE {
            (x, y, row) = (0, y + row, 0);
        }
        if y + h > PAGE || w > PAGE {
            return None;
        }
        Some((UVec2::new(x, y), (x + w, y, row.max(h))))
    }
    /// Reserves `size` plus padding; the top-left of the padded slot, or `None` when full.
    fn reserve(&mut self, size: UVec2) -> Option<UVec2> {
        let (slot, cursor) = self.slot(size)?;
        self.cursor = cursor;
        Some(slot)
    }
    fn material(&self, additive: bool) -> Handle<HudMaterial> {
        if additive {
            self.additive.clone()
        } else {
            self.normal.clone()
        }
    }
    fn white(&self) -> [f32; 2] {
        let centre = (PAD + WHITE / 2) as f32 / PAGE as f32;
        [centre, centre]
    }
}

/// Copies `texture` into `data` (a page) at `slot`, its border replicated `PAD` pixels out.
fn blit(data: &mut [u8], texture: &Texture2D, slot: UVec2) {
    let (w, h) = (u32::from(texture.0.width), u32::from(texture.0.height));
    let rgba = &texture.0.rgba;
    for y in 0..h + 2 * PAD {
        let sy = y.saturating_sub(PAD).min(h - 1);
        for x in 0..w + 2 * PAD {
            let sx = x.saturating_sub(PAD).min(w - 1);
            let from = ((sy * w + sx) * 4) as usize;
            let to = (((slot.y + y) * PAGE + slot.x + x) * 4) as usize;
            data[to..to + 4].copy_from_slice(&rgba[from..from + 4]);
        }
    }
}

struct Entry {
    // Retain CPU ownership so pointer identities cannot be reused for different textures.
    _cpu: Texture2D,
    page: usize,
    map: UvMap,
}

#[derive(Default)]
pub(super) struct Atlas {
    pages: Vec<Page>,
    entries: HashMap<usize, Entry>,
    /// The page the previous quad drew from: untextured quads join it to keep one batch.
    last: Option<usize>,
}
impl Atlas {
    pub(super) fn pages(&self) -> usize {
        self.pages.len()
    }
    /// The page material and coordinate map for `quad`, or `None` when it needs its own texture.
    pub(super) fn material(
        &mut self,
        quad: &Quad,
        images: &mut Assets<Image>,
        materials: &mut Assets<HudMaterial>,
    ) -> Option<(Handle<HudMaterial>, UvMap)> {
        let Some(texture) = &quad.texture else {
            let page = match self.last {
                Some(page) => page,
                None => self.page_with_room(FilterMode::Linear, UVec2::ZERO, images, materials)?,
            };
            self.last = Some(page);
            let page = &self.pages[page];
            return Some((page.material(quad.additive), UvMap::White(page.white())));
        };
        if !fits(quad, texture) {
            return None;
        }
        let (page, map) = match self.entries.get(&texture.id()) {
            Some(entry) => (entry.page, entry.map),
            None => self.insert(texture, images, materials)?,
        };
        self.last = Some(page);
        Some((self.pages[page].material(quad.additive), map))
    }
    fn insert(
        &mut self,
        texture: &Texture2D,
        images: &mut Assets<Image>,
        materials: &mut Assets<HudMaterial>,
    ) -> Option<(usize, UvMap)> {
        let size = UVec2::new(u32::from(texture.0.width), u32::from(texture.0.height));
        let page = self.page_with_room(texture.filter(), size, images, materials)?;
        let slot = self.pages[page].reserve(size)?;
        let mut image = images.get_mut(&self.pages[page].image)?;
        blit(image.data.as_mut()?, texture, slot);
        let map = UvMap::Page {
            offset: ((slot + PAD).as_vec2() / PAGE as f32).to_array(),
            scale: (size.as_vec2() / PAGE as f32).to_array(),
        };
        self.entries.insert(
            texture.id(),
            Entry {
                _cpu: texture.clone(),
                page,
                map,
            },
        );
        Some((page, map))
    }
    /// The newest page of `filter` with room for `size`, opening a new one when needed.
    fn page_with_room(
        &mut self,
        filter: FilterMode,
        size: UVec2,
        images: &mut Assets<Image>,
        materials: &mut Assets<HudMaterial>,
    ) -> Option<usize> {
        if let Some(i) = self.pages.iter().rposition(|p| p.filter == filter)
            && self.pages[i].slot(size).is_some()
        {
            return Some(i);
        }
        self.pages.push(Page::new(filter, images, materials));
        Some(self.pages.len() - 1)
    }
}

/// Whether `quad` samples only inside its small `texture`, so a page entry can stand in for it.
fn fits(quad: &Quad, texture: &Texture2D) -> bool {
    let s = quad.source;
    texture.0.width <= MAX_ENTRY
        && texture.0.height <= MAX_ENTRY
        && texture.0.width > 0
        && texture.0.height > 0
        && s.x >= 0.
        && s.y >= 0.
        && s.x + s.w <= texture.width()
        && s.y + s.h <= texture.height()
}

#[cfg(test)]
mod tests {
    use super::{Atlas, FilterMode, HudMaterial, PAGE, Quad, Texture2D, UvMap};
    use bevy::prelude::{Assets, Image};
    use source_ui::canvas::{Color, Rect};

    fn quad(texture: Option<Texture2D>, source: Rect) -> Quad {
        Quad {
            texture,
            source,
            destination: Rect::new(0., 0., 4., 4.),
            color: Color::new(1., 1., 1., 1.),
            additive: false,
            modulate: false,
            vertices: None,
        }
    }

    #[test]
    fn glyphs_and_rectangles_share_a_page_and_sample_the_same_texels() {
        let mut images = Assets::<Image>::default();
        let mut materials = Assets::<HudMaterial>::default();
        let mut atlas = Atlas::default();
        // 2x1 texture: red, green.
        let glyph = Texture2D::from_rgba8(2, 1, &[255, 0, 0, 255, 0, 255, 0, 255]);
        let other = Texture2D::from_rgba8(1, 1, &[0, 0, 255, 255]);
        let full = Rect::new(0., 0., 2., 1.);
        let (a, map) = atlas
            .material(
                &quad(Some(glyph.clone()), full),
                &mut images,
                &mut materials,
            )
            .unwrap();
        let (b, _) = atlas
            .material(
                &quad(Some(other), Rect::new(0., 0., 1., 1.)),
                &mut images,
                &mut materials,
            )
            .unwrap();
        let (white, white_map) = atlas
            .material(
                &quad(None, Rect::new(0., 0., 1., 1.)),
                &mut images,
                &mut materials,
            )
            .unwrap();
        assert_eq!(a, b);
        assert_eq!(a, white);
        // The same texture again reuses its entry.
        let (again, again_map) = atlas
            .material(&quad(Some(glyph), full), &mut images, &mut materials)
            .unwrap();
        assert_eq!((again, again_map), (a.clone(), map));
        assert_eq!(atlas.pages(), 1);

        let page = images
            .get(&atlas.pages[0].image)
            .unwrap()
            .data
            .clone()
            .unwrap();
        let texel = |uv: [f32; 2]| {
            let (x, y) = (
                (uv[0] * PAGE as f32) as usize,
                (uv[1] * PAGE as f32) as usize,
            );
            let i = (y * PAGE as usize + x) * 4;
            [page[i], page[i + 1], page[i + 2], page[i + 3]]
        };
        let mut uvs = [[0.25, 0.5], [0.75, 0.5], [0.25, 0.5], [0.75, 0.5]];
        map.apply(&mut uvs);
        assert_eq!(texel(uvs[0]), [255, 0, 0, 255]);
        assert_eq!(texel(uvs[1]), [0, 255, 0, 255]);
        // Padding replicates the edge texels (clamp-to-edge).
        let UvMap::Page { offset, .. } = map else {
            panic!("page entry expected")
        };
        assert_eq!(
            texel([offset[0] - 1.5 / PAGE as f32, offset[1]]),
            [255, 0, 0, 255]
        );
        let mut white_uvs = [[0.; 2]; 4];
        white_map.apply(&mut white_uvs);
        assert_eq!(texel(white_uvs[0]), [255; 4]);
    }

    #[test]
    fn large_or_wrapping_textures_keep_their_own_material_and_full_pages_open_new_ones() {
        let mut images = Assets::<Image>::default();
        let mut materials = Assets::<HudMaterial>::default();
        let mut atlas = Atlas::default();
        let big = Texture2D::from_rgba8(129, 1, &[0; 129 * 4]);
        assert!(
            atlas
                .material(
                    &quad(Some(big), Rect::new(0., 0., 129., 1.)),
                    &mut images,
                    &mut materials
                )
                .is_none()
        );
        let small = Texture2D::from_rgba8(2, 2, &[0; 16]);
        assert!(
            atlas
                .material(
                    &quad(Some(small), Rect::new(0., 0., 4., 2.)),
                    &mut images,
                    &mut materials
                )
                .is_none()
        );
        let mut keep = vec![];
        for _ in 0..40 {
            let t = Texture2D::from_rgba8(128, 128, &[0; 128 * 128 * 4]);
            atlas
                .material(
                    &quad(Some(t.clone()), Rect::new(0., 0., 128., 128.)),
                    &mut images,
                    &mut materials,
                )
                .unwrap();
            keep.push(t);
        }
        // 512 / (128 + 4) = 3 per row and 3 rows per page (after the white block's row).
        assert!(atlas.pages() >= 4, "{}", atlas.pages());
        let nearest = Texture2D::from_rgba8(1, 1, &[0; 4]);
        nearest.set_filter(FilterMode::Nearest);
        let before = atlas.pages();
        atlas
            .material(
                &quad(Some(nearest), Rect::new(0., 0., 1., 1.)),
                &mut images,
                &mut materials,
            )
            .unwrap();
        assert_eq!(atlas.pages(), before + 1, "a filter mode gets its own page");
    }
}
