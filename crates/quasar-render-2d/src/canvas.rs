use crate::{Camera2D, Color, Filter, Rect, Result, TextureId, Textures, Transform2D, invalid};
/// One ordered, transformed quad. Texture source rectangles use pixel units.
#[derive(Debug, Clone)]
pub struct Quad {
    pub texture: TextureId,
    pub origin: [f32; 2],
    pub x: [f32; 2],
    pub y: [f32; 2],
    pub source: Option<Rect>,
    pub color: Color,
    pub clip: Rect,
    pub filter: Option<Filter>,
}
#[derive(Debug, Clone, Copy)]
pub struct Sprite {
    pub texture: TextureId,
    pub destination: Rect,
    pub source: Option<Rect>,
    pub tint: Color,
    pub filter: Option<Filter>,
}
impl Sprite {
    pub fn new(texture: TextureId, destination: Rect) -> Self {
        Self {
            texture,
            destination,
            source: None,
            tint: Color::WHITE,
            filter: None,
        }
    }
}
/// Explicit ordered drawing. State is captured at each draw; no global scene graph.
#[derive(Debug, Clone)]
pub struct Canvas {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub clear: Color,
    pub(crate) quads: Vec<Quad>,
    camera: Camera2D,
    transform: Transform2D,
    clip: Option<Rect>,
}
impl Canvas {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 || width > 16384 || height > 16384 {
            return Err(invalid("canvas dimensions must be 1..16384"));
        }
        Ok(Self {
            width,
            height,
            clear: Color::BLACK,
            quads: Vec::new(),
            camera: Camera2D::top_left(width, height),
            transform: Transform2D::IDENTITY,
            clip: None,
        })
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn quads(&self) -> &[Quad] {
        &self.quads
    }
    pub fn set_camera(&mut self, camera: Camera2D) {
        self.camera = camera;
    }
    pub fn set_transform(&mut self, transform: Transform2D) {
        self.transform = transform;
    }
    /// Clip is in logical screen coordinates, after camera/object transforms.
    /// `None` restores full-canvas clipping. Captured per draw.
    pub fn set_clip(&mut self, clip: Option<Rect>) {
        self.clip = clip;
    }
    pub fn rect(&mut self, rect: Rect, color: impl Into<Color>) {
        self.sprite(Sprite {
            texture: TextureId::WHITE,
            destination: rect,
            source: None,
            tint: color.into(),
            filter: None,
        });
    }
    pub fn sprite(&mut self, sprite: Sprite) {
        let t = self
            .transform
            .then(self.camera.transform(self.width, self.height));
        let d = sprite.destination;
        let p = t.point([d.x, d.y]);
        let x = t.point([d.x + d.width, d.y]);
        let y = t.point([d.x, d.y + d.height]);
        let screen = Rect::new(0.0, 0.0, self.width as f32, self.height as f32);
        let clip = self
            .clip
            .map(|clip| {
                if clip.valid() {
                    clip.intersection(screen)
                } else {
                    clip
                }
            })
            .unwrap_or(screen);
        self.quads.push(Quad {
            texture: sprite.texture,
            origin: p,
            x: [x[0] - p[0], x[1] - p[1]],
            y: [y[0] - p[0], y[1] - p[1]],
            source: sprite.source,
            color: sprite.tint,
            clip,
            filter: sprite.filter,
        });
    }
    /// Draws the built-in 5x7 bitmap font as atlas sprites, with a 6-pixel advance.
    pub fn text(&mut self, position: [f32; 2], text: &str, scale: f32, color: impl Into<Color>) {
        let color = color.into();
        for (index, ch) in text.chars().enumerate() {
            let code = if ch.is_ascii() {
                ch as u32
            } else {
                b'?' as u32
            };
            if ch == ' ' {
                continue;
            }
            self.sprite(Sprite {
                texture: TextureId::FONT,
                destination: Rect::new(
                    position[0] + index as f32 * 6.0 * scale,
                    position[1],
                    5.0 * scale,
                    7.0 * scale,
                ),
                source: Some(Rect::new(
                    (code % 16 * 8) as f32,
                    (code / 16 * 8) as f32,
                    5.0,
                    7.0,
                )),
                tint: color,
                filter: None,
            });
        }
    }
    pub fn validate(&self, textures: &Textures) -> Result<()> {
        if self.quads.len() > 65536 {
            return Err(invalid("a canvas supports at most 65536 quads"));
        }
        for q in &self.quads {
            if q.origin
                .iter()
                .chain(&q.x)
                .chain(&q.y)
                .any(|v| !v.is_finite())
                || !q.clip.valid()
            {
                return Err(invalid(
                    "non-finite geometry/camera or invalid clip rectangle",
                ));
            }
            let image = textures
                .get(q.texture)
                .ok_or_else(|| invalid("draw references a removed or unknown texture"))?;
            if let Some(r) = q.source
                && (!r.valid()
                    || r.x < 0.0
                    || r.y < 0.0
                    || r.x + r.width > image.width as f32
                    || r.y + r.height > image.height as f32)
            {
                return Err(invalid("sprite region is outside its texture"));
            }
        }
        Ok(())
    }
}
