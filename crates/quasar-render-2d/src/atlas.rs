//! CPU-only sprite atlas and caller-clocked animation helpers.
use crate::{Rect, Result, Sprite, TextureId, invalid};
#[derive(Debug, Clone)]
pub struct SpriteAtlas {
    texture: TextureId,
    frame: [u32; 2],
    margin: u32,
    spacing: u32,
    columns: u32,
    rows: u32,
}
impl SpriteAtlas {
    pub fn grid(
        texture: TextureId,
        image: [u32; 2],
        frame: [u32; 2],
        margin: u32,
        spacing: u32,
    ) -> Result<Self> {
        let mut counts = [0; 2];
        for i in 0..2 {
            if frame[i] == 0 || image[i] > 16384 {
                return Err(invalid("invalid atlas dimensions"));
            }
            let usable = image[i]
                .checked_sub(
                    margin
                        .checked_mul(2)
                        .ok_or_else(|| invalid("atlas margin overflow"))?,
                )
                .ok_or_else(|| invalid("atlas margins exceed image"))?;
            let stride = frame[i]
                .checked_add(spacing)
                .ok_or_else(|| invalid("atlas spacing overflow"))?;
            let span = usable
                .checked_add(spacing)
                .ok_or_else(|| invalid("atlas extent overflow"))?;
            if span == 0 || !span.is_multiple_of(stride) {
                return Err(invalid("atlas has incomplete grid cells"));
            }
            counts[i] = span / stride;
            if counts[i] == 0 {
                return Err(invalid("atlas has no frames"));
            }
        }
        Ok(Self {
            texture,
            frame,
            margin,
            spacing,
            columns: counts[0],
            rows: counts[1],
        })
    }
    pub fn len(&self) -> u32 {
        self.columns * self.rows
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn columns(&self) -> u32 {
        self.columns
    }
    pub fn region(&self, index: u32) -> Result<Rect> {
        if index >= self.len() {
            return Err(invalid("atlas frame is out of bounds"));
        }
        Ok(Rect::new(
            (self.margin + index % self.columns * (self.frame[0] + self.spacing)) as f32,
            (self.margin + index / self.columns * (self.frame[1] + self.spacing)) as f32,
            self.frame[0] as f32,
            self.frame[1] as f32,
        ))
    }
    pub fn sprite(&self, index: u32, destination: Rect) -> Result<Sprite> {
        let mut sprite = Sprite::new(self.texture, destination);
        sprite.source = Some(self.region(index)?);
        Ok(sprite)
    }
}
#[derive(Debug, Clone, Copy)]
pub struct AnimationFrame {
    pub frame: u32,
    pub duration_ms: u32,
}
#[derive(Debug, Clone)]
pub struct AnimationClip {
    frames: Vec<AnimationFrame>,
    ends: Vec<u64>,
    duration: u64,
}
impl AnimationClip {
    pub fn new(frames: Vec<AnimationFrame>) -> Result<Self> {
        if frames.is_empty() || frames.len() > 65536 || frames.iter().any(|f| f.duration_ms == 0) {
            return Err(invalid("animation needs bounded nonzero-duration frames"));
        }
        let mut duration = 0u64;
        let mut ends = Vec::with_capacity(frames.len());
        for f in &frames {
            duration = duration
                .checked_add(u64::from(f.duration_ms))
                .ok_or_else(|| invalid("animation duration overflow"))?;
            ends.push(duration);
        }
        Ok(Self {
            frames,
            ends,
            duration,
        })
    }
    pub fn duration_ms(&self) -> u64 {
        self.duration
    }
    pub fn frame_at(&self, elapsed_ms: u64) -> u32 {
        self.frames[self
            .ends
            .partition_point(|end| *end <= elapsed_ms % self.duration)]
        .frame
    }
}
