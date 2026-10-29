//! Eight reusable per-pixel mask bits. GPU realization uses GAL stencil state;
//! the reference renderer implements the same compare/replace rules in a byte plane.
use crate::{Canvas, Result, Sprite, SpriteTransform, invalid};
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PixelState {
    /// Pass when (stored & read_mask) == (reference & read_mask).
    pub read_mask: u8,
    pub reference: u8,
    /// Replace these bits with reference after coverage and comparison pass.
    pub write_mask: u8,
    pub color_write: bool,
    /// Inclusive threshold on sampled alpha * tint alpha, in bytes.
    pub alpha_min: u8,
    /// Exclude this raw index from coverage, independent of palette color/alpha.
    pub exclude_index: Option<u8>,
}
impl Default for PixelState {
    fn default() -> Self {
        Self {
            read_mask: 0,
            reference: 0,
            write_mask: 0,
            color_write: true,
            alpha_min: 0,
            exclude_index: None,
        }
    }
}
/// Caller-selected, distinct single bits. Other bits remain available for windows,
/// cutouts, HUD masks or additional priority groups. Sprite order is highest first.
#[derive(Debug, Clone, Copy)]
pub struct Priority2D {
    background: u8,
    sprites: u8,
}
impl Default for Priority2D {
    fn default() -> Self {
        Self {
            background: 1,
            sprites: 2,
        }
    }
}
impl Priority2D {
    pub fn background_mask(self) -> u8 {
        self.background
    }
    pub fn sprite_mask(self) -> u8 {
        self.sprites
    }
    pub fn new(background: u8, sprites: u8) -> Result<Self> {
        if background.count_ones() != 1 || sprites.count_ones() != 1 || background == sprites {
            return Err(invalid("priority needs distinct single mask bits"));
        }
        Ok(Self {
            background,
            sprites,
        })
    }
}
impl Canvas {
    /// Color and occupancy are distinct: indexed zero can draw a universal color
    /// while remaining nonopaque for subsequent behind-background sprites.
    pub fn background_sprite(
        &mut self,
        sprite: Sprite,
        pose: SpriteTransform,
        priority: Priority2D,
        zero_index: Option<u8>,
    ) {
        let old = self.pixel_state();
        self.set_pixel_state(PixelState {
            write_mask: priority.background,
            alpha_min: 1,
            ..Default::default()
        });
        self.sprite_transformed(sprite, pose);
        self.set_pixel_state(PixelState {
            write_mask: priority.background,
            reference: priority.background,
            color_write: false,
            alpha_min: 1,
            exclude_index: zero_index,
            ..Default::default()
        });
        self.sprite_transformed(sprite, pose);
        self.set_pixel_state(old);
    }
    /// First covered sprite wins, even when hidden by background. Transparent
    /// texels never claim a pixel. Sort game-owned OAM/priority values before calling.
    pub fn priority_sprite(
        &mut self,
        sprite: Sprite,
        pose: SpriteTransform,
        priority: Priority2D,
        behind_background: bool,
    ) {
        let old = self.pixel_state();
        self.set_pixel_state(PixelState {
            read_mask: priority.sprites
                | if behind_background {
                    priority.background
                } else {
                    0
                },
            alpha_min: 1,
            ..Default::default()
        });
        self.sprite_transformed(sprite, pose);
        self.set_pixel_state(PixelState {
            reference: priority.sprites,
            write_mask: priority.sprites,
            color_write: false,
            alpha_min: 1,
            ..Default::default()
        });
        self.sprite_transformed(sprite, pose);
        self.set_pixel_state(old);
    }
}
