/// Logical 2D rectangle. Coordinates start at the top left; positive Y points down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    pub(crate) fn valid(self) -> bool {
        [
            self.x,
            self.y,
            self.width,
            self.height,
            self.x + self.width,
            self.y + self.height,
        ]
        .iter()
        .all(|v| v.is_finite())
            && self.width >= 0.0
            && self.height >= 0.0
    }
    pub(crate) fn bounds(self) -> [f32; 4] {
        [self.x, self.y, self.x + self.width, self.y + self.height]
    }
    pub fn intersection(self, other: Self) -> Self {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        Self::new(
            x,
            y,
            (self.x + self.width).min(other.x + other.width).max(x) - x,
            (self.y + self.height).min(other.y + other.height).max(y) - y,
        )
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub [u8; 4]);
impl Color {
    pub const WHITE: Self = Self([255; 4]);
    pub const BLACK: Self = Self([0, 0, 0, 255]);
    pub const TRANSPARENT: Self = Self([0; 4]);
}
impl From<[u8; 4]> for Color {
    fn from(value: [u8; 4]) -> Self {
        Self(value)
    }
}

/// Column-vector affine transform. `a.then(b)` applies a first, then b.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform2D {
    pub x: [f32; 2],
    pub y: [f32; 2],
    pub translation: [f32; 2],
}
impl Default for Transform2D {
    fn default() -> Self {
        Self::IDENTITY
    }
}
impl Transform2D {
    pub const IDENTITY: Self = Self {
        x: [1.0, 0.0],
        y: [0.0, 1.0],
        translation: [0.0, 0.0],
    };
    pub fn translation(x: f32, y: f32) -> Self {
        Self {
            translation: [x, y],
            ..Self::IDENTITY
        }
    }
    pub fn scale(x: f32, y: f32) -> Self {
        Self {
            x: [x, 0.0],
            y: [0.0, y],
            ..Self::IDENTITY
        }
    }
    pub fn rotation(radians: f32) -> Self {
        let (s, c) = radians.sin_cos();
        Self {
            x: [c, s],
            y: [-s, c],
            ..Self::IDENTITY
        }
    }
    pub fn point(self, p: [f32; 2]) -> [f32; 2] {
        [
            p[0] * self.x[0] + p[1] * self.y[0] + self.translation[0],
            p[0] * self.x[1] + p[1] * self.y[1] + self.translation[1],
        ]
    }
    pub fn then(self, next: Self) -> Self {
        let origin = next.point(self.translation);
        let x = next.point([
            self.translation[0] + self.x[0],
            self.translation[1] + self.x[1],
        ]);
        let y = next.point([
            self.translation[0] + self.y[0],
            self.translation[1] + self.y[1],
        ]);
        Self {
            x: [x[0] - origin[0], x[1] - origin[1]],
            y: [y[0] - origin[0], y[1] - origin[1]],
            translation: origin,
        }
    }
}
/// Orthographic camera centered on a world point, with positive zoom and radians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera2D {
    pub center: [f32; 2],
    pub zoom: f32,
    pub rotation: f32,
}
impl Camera2D {
    pub fn top_left(width: u32, height: u32) -> Self {
        Self {
            center: [width as f32 / 2.0, height as f32 / 2.0],
            zoom: 1.0,
            rotation: 0.0,
        }
    }
    pub(crate) fn transform(self, width: u32, height: u32) -> Transform2D {
        Transform2D::translation(-self.center[0], -self.center[1])
            .then(Transform2D::rotation(-self.rotation))
            .then(Transform2D::scale(self.zoom, self.zoom))
            .then(Transform2D::translation(
                width as f32 / 2.0,
                height as f32 / 2.0,
            ))
    }
}
