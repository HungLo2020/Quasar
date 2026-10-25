use crate::TextResult;
use crate::style::{TextAlign, TextStyle, TextWrap};
use cosmic_text::fontdb;
use cosmic_text::{
    Align, Attrs, Buffer, Cursor, Family, FontSystem, Metrics, Shaping, Style as FontStyle, Weight,
    Wrap,
};
use std::ops::Range;
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

/// Opaque face identity assigned by the font database.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FontId(pub(crate) fontdb::ID);

/// One shaped glyph in visual order.
#[derive(Clone, Debug, PartialEq)]
pub struct Glyph {
    pub font: FontId,
    pub glyph_id: u16,
    pub font_size: f32,
    pub font_weight: u16,
    /// Horizontal position in physical pixels from the layout origin.
    pub x: f32,
    /// Baseline position in physical pixels from the layout origin.
    pub baseline: f32,
    /// Shaped advance in physical pixels.
    pub width: f32,
    pub line_top: f32,
    pub line_height: f32,
    pub byte_range: Range<usize>,
    pub char_range: Range<usize>,
    pub rtl: bool,
    /// Rasterization flags from the shaping engine (fake italic, hinting, ...).
    pub raster_flags: u32,
}

/// One visual line produced by shaping and wrapping.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub top: f32,
    pub baseline: f32,
    pub height: f32,
    pub width: f32,
    pub rtl: bool,
    pub char_range: Range<usize>,
}

/// A caret position in layout coordinates (top-left origin).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Caret {
    pub x: f32,
    pub top: f32,
    pub height: f32,
    pub line: usize,
}

/// A selected range rectangle in layout coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SelectionRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Immutable shaped text produced by [`crate::TextEngine::layout`].
///
/// The layout retains the shaping buffer so hit testing, caret placement and
/// selection use the exact glyph positions that rasterization consumes.
#[derive(Clone, Debug)]
pub struct TextLayout {
    buffer: Buffer,
    text: String,
    style: TextStyle,
    width: f32,
    height: f32,
    lines: Vec<Line>,
    glyphs: Vec<Glyph>,
    char_offsets: Vec<usize>,
}

impl TextLayout {
    pub(crate) fn shape(
        font_system: &mut FontSystem,
        text: &str,
        style: &TextStyle,
        width: Option<f32>,
    ) -> TextResult<Self> {
        style.validate()?;
        let metrics = Metrics::new(style.size, style.line_height);
        let mut buffer = Buffer::new(font_system, metrics);
        buffer.set_size(width, None);
        buffer.set_wrap(match style.wrap {
            TextWrap::None => Wrap::None,
            TextWrap::Word => Wrap::Word,
            TextWrap::Glyph => Wrap::Glyph,
            TextWrap::WordOrGlyph => Wrap::WordOrGlyph,
        });
        let attrs = Attrs::new()
            .family(match style.family.as_deref() {
                Some(family) => Family::Name(family),
                None => Family::SansSerif,
            })
            .weight(Weight(style.weight))
            .style(if style.italic {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            });
        buffer.set_text(
            text,
            &attrs,
            Shaping::Advanced,
            Some(match style.align {
                TextAlign::Start => Align::Left,
                TextAlign::Center => Align::Center,
                TextAlign::End => Align::Right,
                TextAlign::Justify => Align::Justified,
            }),
        );
        buffer.shape_until_scroll(font_system, false);

        let char_offsets: Vec<usize> = text.char_indices().map(|(offset, _)| offset).collect();
        let mut lines = Vec::new();
        let mut glyphs = Vec::new();
        let mut max_width: f32 = 0.0;
        for run in buffer.layout_runs() {
            let char_start = run
                .glyphs
                .iter()
                .map(|glyph| byte_to_char_offsets(&char_offsets, glyph.start))
                .min()
                .unwrap_or(0);
            let char_end = run
                .glyphs
                .iter()
                .map(|glyph| byte_to_char_offsets(&char_offsets, glyph.end))
                .max()
                .unwrap_or(char_start);
            lines.push(Line {
                top: run.line_top,
                baseline: run.line_y,
                height: run.line_height,
                width: run.line_w,
                rtl: run.rtl,
                char_range: char_start..char_end,
            });
            max_width = max_width.max(run.line_w);
            for glyph in run.glyphs {
                glyphs.push(Glyph {
                    font: FontId(glyph.font_id),
                    glyph_id: glyph.glyph_id,
                    font_size: glyph.font_size,
                    font_weight: glyph.font_weight.0,
                    x: glyph.x,
                    baseline: run.line_y,
                    width: glyph.w,
                    line_top: run.line_top,
                    line_height: run.line_height,
                    byte_range: glyph.start..glyph.end,
                    char_range: byte_to_char_offsets(&char_offsets, glyph.start)
                        ..byte_to_char_offsets(&char_offsets, glyph.end),
                    rtl: glyph.level.is_rtl(),
                    raster_flags: glyph.cache_key_flags.bits(),
                });
            }
        }
        let height = lines
            .last()
            .map_or(style.line_height, |line| line.top + line.height)
            .max(style.line_height);
        Ok(Self {
            buffer,
            text: text.to_owned(),
            style: style.clone(),
            width: max_width,
            height,
            lines,
            glyphs,
            char_offsets,
        })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn style(&self) -> &TextStyle {
        &self.style
    }

    /// Width of the widest shaped line, not the requested wrapping width.
    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn height(&self) -> f32 {
        self.height
    }

    pub fn line_height(&self) -> f32 {
        self.style.line_height
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    pub fn glyphs(&self) -> &[Glyph] {
        &self.glyphs
    }

    pub fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    /// Converts a character index to a UTF-8 byte index at a char boundary.
    pub fn char_to_byte(&self, char_index: usize) -> usize {
        self.char_offsets
            .get(char_index)
            .copied()
            .unwrap_or(self.text.len())
    }

    /// Converts a UTF-8 byte index to the number of preceding characters.
    pub fn byte_to_char(&self, byte: usize) -> usize {
        byte_to_char_offsets(&self.char_offsets, byte)
    }

    /// Returns the visual caret for a logical character index.
    pub fn caret(&self, char_index: usize) -> Caret {
        let char_index = char_index.min(self.char_count());
        if self.text.is_empty() {
            return Caret {
                x: 0.0,
                top: 0.0,
                height: self.style.line_height,
                line: 0,
            };
        }
        let byte = self.char_to_byte(char_index);
        let (line, line_start) = line_of_byte(&self.text, byte);
        let line = line.min(self.buffer.lines.len().saturating_sub(1));
        let index = byte.saturating_sub(line_start);
        let cursor = Cursor::new(line, index);
        let (x, top) = self.buffer.cursor_position(&cursor).unwrap_or_else(|| {
            let glyph = self
                .glyphs
                .iter()
                .rfind(|glyph| glyph.char_range.start <= char_index);
            glyph.map_or((0.0, 0.0), |glyph| (glyph.x + glyph.width, glyph.line_top))
        });
        let height = self
            .lines
            .iter()
            .find(|candidate| (candidate.top - top).abs() < 0.5)
            .map_or(self.style.line_height, |line| line.height);
        Caret {
            x,
            top,
            height,
            line,
        }
    }

    /// Hit-tests a point in layout coordinates and returns a character index.
    /// Points outside the layout are clamped to the nearest logical boundary.
    pub fn hit(&self, x: f32, y: f32) -> usize {
        if self.text.is_empty() {
            return 0;
        }
        let Some(cursor) = self.buffer.hit(x, y) else {
            return if y < 0.0 || x < 0.0 {
                0
            } else {
                self.char_count()
            };
        };
        let line_start = self.line_start(cursor.line);
        self.byte_to_char(line_start + cursor.index)
    }

    /// Returns a character range for a dragged selection, ordered logically.
    pub fn ordered_selection(&self, anchor: usize, cursor: usize) -> Range<usize> {
        let anchor = anchor.min(self.char_count());
        let cursor = cursor.min(self.char_count());
        anchor.min(cursor)..anchor.max(cursor)
    }

    /// Returns visual selection rectangles for a logical character range.
    pub fn selection_rects(&self, anchor: usize, cursor: usize) -> Vec<SelectionRect> {
        let range = self.ordered_selection(anchor, cursor);
        if range.is_empty() {
            return Vec::new();
        }
        let mut rects: Vec<SelectionRect> = Vec::new();
        for glyph in &self.glyphs {
            if glyph.char_range.end <= range.start || glyph.char_range.start >= range.end {
                continue;
            }
            let rect = SelectionRect {
                x: glyph.x,
                y: glyph.line_top,
                width: glyph.width,
                height: glyph.line_height,
            };
            if let Some(last) = rects.last_mut()
                && (last.y - rect.y).abs() < 0.5
                && (rect.x - (last.x + last.width)).abs() < 0.5
            {
                last.width += rect.width;
            } else {
                rects.push(rect);
            }
        }
        rects
    }

    /// Previous grapheme-cluster boundary, in character indices.
    pub fn previous_boundary(&self, char_index: usize) -> usize {
        let byte = self.char_to_byte(char_index);
        let mut previous = 0;
        for (offset, _) in self.text.grapheme_indices(true) {
            if offset >= byte {
                break;
            }
            previous = self.byte_to_char(offset);
        }
        previous
    }

    /// Next grapheme-cluster boundary, in character indices.
    pub fn next_boundary(&self, char_index: usize) -> usize {
        let byte = self.char_to_byte(char_index);
        for (offset, _) in self.text.grapheme_indices(true) {
            if offset > byte {
                return self.byte_to_char(offset);
            }
        }
        self.char_count()
    }

    fn line_start(&self, line: usize) -> usize {
        let mut offset = 0;
        for (index, segment) in self.text.split('\n').enumerate() {
            if index == line {
                return offset;
            }
            offset += segment.len() + 1;
        }
        self.text.len()
    }
}

fn byte_to_char_offsets(offsets: &[usize], byte: usize) -> usize {
    offsets.partition_point(|offset| *offset < byte)
}

fn line_of_byte(text: &str, byte: usize) -> (usize, usize) {
    let byte = byte.min(text.len());
    let before = &text[..byte];
    let line = before.bytes().filter(|byte| *byte == b'\n').count();
    let start = before.rfind('\n').map_or(0, |index| index + 1);
    (line, start)
}

/// Cached layout statistics for diagnostics and benchmarks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LayoutStats {
    pub calls: u64,
    pub cache_hits: u64,
    pub cache_entries: usize,
}

/// Shared shaped layout handle. Layouts are immutable and cheap to clone.
pub type SharedLayout = Arc<TextLayout>;
