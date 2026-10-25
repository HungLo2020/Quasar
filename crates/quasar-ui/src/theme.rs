/// Straight RGBA color. Alpha is used by the optional 2D adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const WHITE: Self = Self::rgba(255, 255, 255, 255);
    pub const BLACK: Self = Self::rgba(0, 0, 0, 255);
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::rgba(r, g, b, 255)
    }
    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let blend = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t) as u8;
        Self::rgba(
            blend(self.r, other.r),
            blend(self.g, other.g),
            blend(self.b, other.b),
            blend(self.a, other.a),
        )
    }
    pub fn with_alpha(self, alpha: u8) -> Self {
        Self { a: alpha, ..self }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TextStyle {
    Body,
    Heading,
    Small,
    Dim,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Theme {
    pub name: String,
    pub background: Color,
    pub panel: Color,
    pub panel_alt: Color,
    pub text: Color,
    pub dim_text: Color,
    pub accent: Color,
    pub focus: Color,
    pub disabled: Color,
    pub button: Color,
    pub button_hover: Color,
    pub button_active: Color,
    pub selection: Color,
    pub track: Color,
    pub fill: Color,
    pub danger: Color,
    pub overlay: Color,
    pub font_size: f32,
    pub heading_size: f32,
    pub small_size: f32,
    pub row_height: f32,
    pub padding: f32,
    pub gap: f32,
    pub border_width: f32,
    pub focus_width: f32,
    pub transition_rate: f32,
}

impl Theme {
    pub fn retro() -> Self {
        Self {
            name: "retro".into(),
            background: Color::rgba(10, 14, 24, 235),
            panel: Color::rgba(24, 35, 52, 250),
            panel_alt: Color::rgba(34, 49, 72, 250),
            text: Color::rgb(232, 226, 190),
            dim_text: Color::rgb(154, 174, 196),
            accent: Color::rgb(244, 210, 120),
            focus: Color::rgb(255, 255, 255),
            disabled: Color::rgb(105, 116, 130),
            button: Color::rgb(40, 61, 88),
            button_hover: Color::rgb(57, 86, 122),
            button_active: Color::rgb(82, 122, 160),
            selection: Color::rgb(61, 91, 128),
            track: Color::rgb(12, 20, 31),
            fill: Color::rgb(110, 206, 174),
            danger: Color::rgb(220, 104, 92),
            overlay: Color::rgba(0, 0, 0, 170),
            font_size: 14.0,
            heading_size: 22.0,
            small_size: 10.0,
            row_height: 24.0,
            padding: 6.0,
            gap: 4.0,
            border_width: 1.0,
            focus_width: 2.0,
            transition_rate: 18.0,
        }
    }

    pub fn modern() -> Self {
        Self {
            name: "modern".into(),
            background: Color::rgba(18, 21, 28, 230),
            panel: Color::rgba(36, 41, 51, 245),
            panel_alt: Color::rgba(46, 53, 66, 245),
            text: Color::rgb(240, 244, 250),
            dim_text: Color::rgb(174, 184, 198),
            accent: Color::rgb(86, 166, 255),
            focus: Color::rgb(255, 255, 255),
            disabled: Color::rgb(112, 120, 132),
            button: Color::rgb(48, 56, 70),
            button_hover: Color::rgb(61, 76, 98),
            button_active: Color::rgb(76, 112, 160),
            selection: Color::rgb(62, 91, 132),
            track: Color::rgb(24, 29, 38),
            fill: Color::rgb(86, 166, 255),
            danger: Color::rgb(236, 103, 112),
            overlay: Color::rgba(0, 0, 0, 150),
            font_size: 16.0,
            heading_size: 26.0,
            small_size: 11.0,
            row_height: 30.0,
            padding: 8.0,
            gap: 6.0,
            border_width: 1.0,
            focus_width: 2.0,
            transition_rate: 14.0,
        }
    }

    pub fn color(&self, style: TextStyle) -> Color {
        match style {
            TextStyle::Body | TextStyle::Heading => self.text,
            TextStyle::Small => self.dim_text,
            TextStyle::Dim => self.dim_text.with_alpha(190),
        }
    }

    pub fn size(&self, style: TextStyle) -> f32 {
        match style {
            TextStyle::Body => self.font_size,
            TextStyle::Heading => self.heading_size,
            TextStyle::Small => self.small_size,
            TextStyle::Dim => self.small_size,
        }
    }

    /// WCAG 2.x relative-luminance contrast ratio. Alpha is ignored.
    pub fn contrast_ratio(foreground: Color, background: Color) -> f32 {
        fn channel(value: u8) -> f32 {
            let value = f32::from(value) / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        }
        let luminance = |color: Color| {
            0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
        };
        let first = luminance(foreground);
        let second = luminance(background);
        (first.max(second) + 0.05) / (first.min(second) + 0.05)
    }
}
