//! The shadow texture page and its fragments (Source SDK `clientshadowmgr.cpp`
//! `CTextureAllocator`): a 1024-texel page of sixteen 256-texel blocks, each cut into fragments of
//! one size (one block of 16-texel fragments, one of 32, two of 64, six of 128, six of 256), with a
//! least-recently-used list per size. Each frame a shadow in view asks for a fragment as big as its
//! screen area wants (up to its own texture size); it keeps the one it has when that is near enough,
//! and takes a smaller size when every fragment of the wanted one is already used this frame.

use glam::{UVec2, Vec2};
use std::collections::VecDeque;

/// The page edge, texels.
pub const PAGE: u32 = 1024;
const BLOCK: u32 = 256;
const MIN_POWER: u32 = 4;
const MAX_POWER: u32 = 8;
/// Each block's fragment size, as a power of two, in page order.
const BLOCK_POWERS: [u32; 16] = [4, 5, 6, 6, 7, 7, 7, 7, 7, 7, 8, 8, 8, 8, 8, 8];

#[derive(Clone, Debug)]
struct Fragment {
    block: usize,
    index: u32,
    texture: Option<usize>,
    frame_used: u32,
}

#[derive(Clone, Debug)]
struct Texture {
    power: u32,
    fragment: Option<usize>,
}

/// A shadow's texture in the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureId(usize);

#[derive(Clone, Debug)]
pub struct Allocator {
    fragments: Vec<Fragment>,
    /// Fragments per power, least recently used first.
    lru: Vec<VecDeque<usize>>,
    textures: Vec<Texture>,
    frame: u32,
}

impl Default for Allocator {
    fn default() -> Self {
        Self::new()
    }
}

impl Allocator {
    /// The page with every fragment unused (`CTextureAllocator::Reset`).
    pub fn new() -> Self {
        let mut out = Self {
            fragments: Vec::new(),
            lru: vec![VecDeque::new(); MAX_POWER as usize + 1],
            textures: Vec::new(),
            frame: 0,
        };
        for (block, &power) in BLOCK_POWERS.iter().enumerate() {
            let count = (BLOCK >> power).pow(2);
            for index in (0..count).rev() {
                out.lru[power as usize].push_front(out.fragments.len());
                out.fragments.push(Fragment {
                    block,
                    index,
                    texture: None,
                    frame_used: u32::MAX,
                });
            }
        }
        out
    }

    /// A shadow texture of up to `size` texels (clamped to 16..256), with no fragment yet.
    pub fn allocate(&mut self, size: u32) -> TextureId {
        let size = size.clamp(1 << MIN_POWER, 1 << MAX_POWER);
        self.textures.push(Texture {
            power: size.ilog2(),
            fragment: None,
        });
        TextureId(self.textures.len() - 1)
    }

    pub fn advance_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    fn mark_used(&mut self, f: usize) {
        let power = BLOCK_POWERS[self.fragments[f].block] as usize;
        self.lru[power].retain(|&g| g != f);
        self.lru[power].push_back(f);
        self.fragments[f].frame_used = self.frame;
    }

    fn disconnect(&mut self, f: usize) {
        if let Some(t) = self.fragments[f].texture.take() {
            self.textures[t].fragment = None;
        }
    }

    /// `UseTexture`: gives the texture a fragment for this frame's screen `area` (pixels); true
    /// when it must be redrawn (a new fragment). `redraw` is whether it will be redrawn anyway.
    pub fn use_texture(&mut self, id: TextureId, redraw: bool, area: f32) -> bool {
        let texture = self.textures[id.0].clone();
        let mut desired = MIN_POWER;
        while ((1u32 << desired) as f32).powi(2) < area {
            if desired >= texture.power {
                desired = texture.power;
                break;
            }
            desired += 1;
        }
        let current = texture
            .fragment
            .map(|f| (f, BLOCK_POWERS[self.fragments[f].block]));
        if let Some((f, power)) = current {
            let keep = !redraw && desired < MAX_POWER && desired.saturating_sub(power) <= 1;
            if power == desired || keep {
                self.mark_used(f);
                return false;
            }
        }
        // The least recently used fragment of the wanted size not used this frame, else smaller.
        let mut found = None;
        let mut power = desired as i32;
        while power >= 0 {
            let head = self.lru[power as usize].front().copied();
            if let Some(f) = head.filter(|&f| self.fragments[f].frame_used != self.frame) {
                found = Some(f);
                break;
            }
            power -= 1;
        }
        if let Some((f, current_power)) = current {
            if power <= current_power as i32 {
                self.mark_used(f);
                return false;
            }
            self.disconnect(f);
        }
        let Some(f) = found else {
            return false;
        };
        self.disconnect(f);
        self.textures[id.0].fragment = Some(f);
        self.fragments[f].texture = Some(id.0);
        self.mark_used(f);
        true
    }

    /// The texture's fragment in the page (corner, size), when it has one (`GetTextureRect`).
    pub fn rect(&self, id: TextureId) -> Option<(UVec2, u32)> {
        let fragment = &self.fragments[self.textures[id.0].fragment?];
        let size = 1 << BLOCK_POWERS[fragment.block];
        let per_row = BLOCK / size;
        let block = UVec2::new(fragment.block as u32 % 4, fragment.block as u32 / 4) * BLOCK;
        let at = UVec2::new(fragment.index % per_row, fragment.index / per_row) * size;
        Some((block + at, size))
    }
}

/// A fragment's rectangle in 0..1 page coordinates, inset half a texel against its neighbours
/// (`SetRenderToTextureShadowTexCoords`).
pub fn uv_rect((at, size): (UVec2, u32)) -> (Vec2, Vec2) {
    let page = PAGE as f32;
    (
        (at.as_vec2() + 0.5) / page,
        Vec2::splat((size as f32 - 1.) / page),
    )
}
