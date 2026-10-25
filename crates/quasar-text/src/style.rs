/// Horizontal alignment for one shaped paragraph or line.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
    Justify,
}

/// Wrapping policy used by the layout engine.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TextWrap {
    #[default]
    None,
    Word,
    Glyph,
    WordOrGlyph,
}

/// Font and layout attributes for a run of text.
///
/// Family names are matched through `fontdb`. When a glyph is missing from the
/// requested family, the engine falls back to another registered face that
/// supports it. Registering fewer faces therefore makes fallback deterministic.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TextStyle {
    pub family: Option<String>,
    /// Font size in physical pixels (DPI already applied by the caller).
    pub size: f32,
    /// Baseline-to-baseline distance in physical pixels.
    pub line_height: f32,
    pub weight: u16,
    pub italic: bool,
    pub align: TextAlign,
    pub wrap: TextWrap,
}

impl TextStyle {
    pub fn new(size: f32) -> Self {
        Self {
            family: None,
            size,
            line_height: size * 1.25,
            weight: 400,
            italic: false,
            align: TextAlign::Start,
            wrap: TextWrap::None,
        }
    }

    pub fn with_family(mut self, family: impl Into<String>) -> Self {
        self.family = Some(family.into());
        self
    }

    pub fn with_optional_family(mut self, family: Option<impl Into<String>>) -> Self {
        self.family = family.map(Into::into);
        self
    }

    pub fn with_line_height(mut self, line_height: f32) -> Self {
        self.line_height = line_height;
        self
    }

    pub fn with_weight(mut self, weight: u16) -> Self {
        self.weight = weight;
        self
    }

    pub fn with_italic(mut self, italic: bool) -> Self {
        self.italic = italic;
        self
    }

    pub fn with_align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    pub fn with_wrap(mut self, wrap: TextWrap) -> Self {
        self.wrap = wrap;
        self
    }

    pub(crate) fn validate(&self) -> crate::TextResult<()> {
        if !self.size.is_finite() || self.size < 1.0 || self.size > 1024.0 {
            return Err(crate::TextError::InvalidStyle(format!(
                "font size must be finite and in 1..=1024, got {}",
                self.size
            )));
        }
        if !self.line_height.is_finite() || self.line_height < self.size * 0.5 {
            return Err(crate::TextError::InvalidStyle(format!(
                "line height must be finite and at least half the font size, got {}",
                self.line_height
            )));
        }
        Ok(())
    }

    pub(crate) fn key(&self) -> StyleKey {
        StyleKey {
            family: self.family.clone(),
            size_bits: self.size.to_bits(),
            line_height_bits: self.line_height.to_bits(),
            weight: self.weight,
            italic: self.italic,
            align: self.align,
            wrap: self.wrap,
        }
    }
}

impl Default for TextStyle {
    fn default() -> Self {
        Self::new(16.0)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct StyleKey {
    pub family: Option<String>,
    pub size_bits: u32,
    pub line_height_bits: u32,
    pub weight: u16,
    pub italic: bool,
    pub align: TextAlign,
    pub wrap: TextWrap,
}
