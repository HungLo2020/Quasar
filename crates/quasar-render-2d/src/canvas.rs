use crate::{Camera2D, Color, Filter, Rect, Result, TextureId, Textures, Transform2D, invalid};
/// One ordered, transformed quad. Texture source rectangles use pixel units.
#[derive(Debug, Clone)]
pub struct Quad {
    pub texture: TextureId,
    pub palette: Option<crate::PaletteBinding>,
    pub pixels: crate::PixelState,
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
/// Normalized sprite transforms: transpose first, then horizontal/vertical
/// flips. This also represents all eight orthogonal Tiled tile orientations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpriteTransform {
    pub transpose: bool,
    pub flip_x: bool,
    pub flip_y: bool,
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
    palette: Option<crate::PaletteBinding>,
    pixels: crate::PixelState,
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
            palette: None,
            pixels: crate::PixelState::default(),
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
    pub fn camera(&self) -> Camera2D {
        self.camera
    }
    pub fn set_transform(&mut self, transform: Transform2D) {
        self.transform = transform;
    }
    /// Clip is in logical screen coordinates, after camera/object transforms.
    /// `None` restores full-canvas clipping. Captured per draw.
    pub fn set_clip(&mut self, clip: Option<Rect>) {
        self.clip = clip;
    }
    pub fn set_palette(&mut self, palette: Option<crate::PaletteBinding>) {
        self.palette = palette;
    }
    pub fn palette(&self) -> Option<crate::PaletteBinding> {
        self.palette
    }
    pub fn set_pixel_state(&mut self, pixels: crate::PixelState) {
        self.pixels = pixels;
    }
    pub fn pixel_state(&self) -> crate::PixelState {
        self.pixels
    }
    pub fn uses_pixel_masks(&self) -> bool {
        self.quads
            .iter()
            .any(|q| q.pixels.read_mask != 0 || q.pixels.write_mask != 0)
    }
    /// Append a recorded canvas in screen coordinates. Scale must be finite and
    /// nonnegative. Source clear color is not drawn; camera and current drawing
    /// state are unchanged. Pixel mask bits share the destination mask buffer.
    /// Both image and palette IDs are passed to `resolve`.
    pub fn append_canvas(
        &mut self,
        source: &Canvas,
        origin: [f32; 2],
        scale: [f32; 2],
        clip: Option<Rect>,
        mut resolve: impl FnMut(TextureId) -> TextureId,
    ) -> Result<()> {
        if origin.iter().chain(&scale).any(|v| !v.is_finite())
            || scale.iter().any(|v| *v < 0.0)
            || clip.is_some_and(|r| !r.valid())
        {
            return Err(invalid("invalid composition transform or clip"));
        }
        if self.quads.len().saturating_add(source.quads.len()) > 65536 {
            return Err(invalid("a canvas supports at most 65536 quads"));
        }
        let bounds = Rect::new(0.0, 0.0, self.width as f32, self.height as f32);
        let bounds = clip.map_or(bounds, |c| c.intersection(bounds));
        let mut appended = Vec::with_capacity(source.quads.len());
        for q in &source.quads {
            let mut q = q.clone();
            q.origin = [
                origin[0] + q.origin[0] * scale[0],
                origin[1] + q.origin[1] * scale[1],
            ];
            q.x = [q.x[0] * scale[0], q.x[1] * scale[1]];
            q.y = [q.y[0] * scale[0], q.y[1] * scale[1]];
            q.clip = Rect::new(
                origin[0] + q.clip.x * scale[0],
                origin[1] + q.clip.y * scale[1],
                q.clip.width * scale[0],
                q.clip.height * scale[1],
            );
            if q.origin
                .iter()
                .chain(&q.x)
                .chain(&q.y)
                .any(|v| !v.is_finite())
                || !q.clip.valid()
            {
                return Err(invalid("composition produces invalid geometry"));
            }
            q.clip = q.clip.intersection(bounds);
            q.texture = resolve(q.texture);
            if let Some(ref mut palette) = q.palette {
                palette.texture = resolve(palette.texture);
            }
            appended.push(q);
        }
        self.quads.extend(appended);
        Ok(())
    }

    /// Draw a centered, butt-ended line. Invalid or degenerate lines are ignored.
    pub fn line(&mut self, a: [f32; 2], b: [f32; 2], width: f32, color: impl Into<Color>) {
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let length = dx.hypot(dy);
        if !length.is_finite() || length <= 0.0 || !width.is_finite() || width <= 0.0 {
            return;
        }
        let previous = self.transform;
        self.transform = Transform2D::rotation(dy.atan2(dx))
            .then(Transform2D::translation(
                a[0] * 0.5 + b[0] * 0.5,
                a[1] * 0.5 + b[1] * 0.5,
            ))
            .then(previous);
        self.rect(Rect::new(-length * 0.5, -width * 0.5, length, width), color);
        self.transform = previous;
    }
    /// Draw connected butt-ended segments. Joins may overlap.
    pub fn polyline(&mut self, points: &[[f32; 2]], width: f32, color: impl Into<Color>) {
        let color = color.into();
        for pair in points.windows(2) {
            self.line(pair[0], pair[1], width, color);
        }
    }
    /// Draw an inside rectangle outline; corners do not overlap.
    pub fn outline_rect(&mut self, rect: Rect, width: f32, color: impl Into<Color>) {
        if !rect.valid() || !width.is_finite() || width <= 0.0 {
            return;
        }
        let color = color.into();
        let w = width.min(rect.width * 0.5).min(rect.height * 0.5);
        self.rect(Rect::new(rect.x, rect.y, rect.width, w), color);
        self.rect(
            Rect::new(rect.x, rect.y + rect.height - w, rect.width, w),
            color,
        );
        self.rect(
            Rect::new(rect.x, rect.y + w, w, rect.height - 2.0 * w),
            color,
        );
        self.rect(
            Rect::new(
                rect.x + rect.width - w,
                rect.y + w,
                w,
                rect.height - 2.0 * w,
            ),
            color,
        );
    }
    /// Draw an approximated circle outline with an explicit segment count.
    pub fn circle_outline(
        &mut self,
        center: [f32; 2],
        radius: f32,
        segments: usize,
        width: f32,
        color: impl Into<Color>,
    ) {
        if center.iter().any(|v| !v.is_finite())
            || !radius.is_finite()
            || radius <= 0.0
            || !(3..=4096).contains(&segments)
        {
            return;
        }
        let color = color.into();
        let point = |i: usize| {
            let angle = i as f32 * std::f32::consts::TAU / segments as f32;
            [
                center[0] + radius * angle.cos(),
                center[1] + radius * angle.sin(),
            ]
        };
        for i in 0..segments {
            self.line(point(i), point((i + 1) % segments), width, color);
        }
    }
    /// Width of the built-in 5x7 font's visible cells, including spaces.
    pub fn bitmap_text_width(text: &str, scale: f32) -> f32 {
        let count = text.chars().count();
        if count == 0 {
            0.0
        } else {
            (count as f32 * 6.0 - 1.0) * scale
        }
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
        self.sprite_transformed(sprite, SpriteTransform::default());
    }
    pub fn sprite_flipped(&mut self, sprite: Sprite, flip_x: bool, flip_y: bool) {
        self.sprite_transformed(
            sprite,
            SpriteTransform {
                flip_x,
                flip_y,
                transpose: false,
            },
        );
    }
    pub fn sprite_transformed(&mut self, sprite: Sprite, pose: SpriteTransform) {
        let t = self
            .transform
            .then(self.camera.transform(self.width, self.height));
        let d = sprite.destination;
        let corner = |mut u: f32, mut v: f32| {
            if pose.transpose {
                std::mem::swap(&mut u, &mut v);
            }
            if pose.flip_x {
                u = 1.0 - u;
            }
            if pose.flip_y {
                v = 1.0 - v;
            }
            t.point([d.x + u * d.width, d.y + v * d.height])
        };
        let p = corner(0.0, 0.0);
        let x = corner(1.0, 0.0);
        let y = corner(0.0, 1.0);
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
            palette: self.palette,
            pixels: self.pixels,
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
            if let Some(p) = q.palette
                && textures.dimensions(p.texture) != Some((256, 1))
            {
                return Err(invalid("palette must reference a live 256x1 texture"));
            }
            if q.pixels.exclude_index.is_some() && q.palette.is_none() {
                return Err(invalid("index coverage requires an indexed draw"));
            }
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
                .dimensions(q.texture)
                .ok_or_else(|| invalid("draw references a removed or unknown texture"))?;
            if let Some(r) = q.source
                && (!r.valid()
                    || r.x < 0.0
                    || r.y < 0.0
                    || r.x + r.width > image.0 as f32
                    || r.y + r.height > image.1 as f32)
            {
                return Err(invalid("sprite region is outside its texture"));
            }
        }
        Ok(())
    }
}
