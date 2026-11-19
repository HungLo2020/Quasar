//! Standalone single-line text editing.
//!
//! [`Widget::TextInput`](crate::Widget::TextInput) keeps its text, cursor
//! and focus in a persistent [`Ui`](crate::Ui) tree. A UI that rebuilds its
//! widgets every frame (immediate-mode style) cannot keep that state, so
//! [`LineEditor`] offers the same editing as a value the game owns: feed it
//! the frame's [`UiInput`] and it applies typing, IME commits, cursor
//! movement (by grapheme where the measure supports it), Home/End,
//! selection with Shift, select-all, Backspace and Delete. Caret and
//! selection positions come from the [`TextMeasure`], so they are exact for
//! shaped, proportional fonts.
//!
//! ```
//! use quasar_ui::{MonospaceMeasure, LineEditor, UiInput};
//!
//! let mut line = LineEditor::new("spawn");
//! let typed = UiInput { text: " drone".into(), ..UiInput::default() };
//! let edit = line.apply(&typed, &MonospaceMeasure);
//! assert!(edit.changed);
//! assert_eq!(line.text(), "spawn drone");
//! assert!(line.apply(&UiInput { activate: true, ..UiInput::default() }, &MonospaceMeasure).submitted);
//! ```
use crate::{TextMeasure, TextStyle, UiInput};

/// What one [`LineEditor::apply`] call did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineEdit {
    /// The text changed.
    pub changed: bool,
    /// The cursor or selection moved (or the text changed).
    pub moved: bool,
    /// Enter / activate was pressed.
    pub submitted: bool,
    /// Escape / cancel was pressed.
    pub cancelled: bool,
    /// Up (-1) or down (+1) was pressed, for history or suggestions.
    pub vertical: i8,
    /// Next / previous (Tab) was pressed, for completion.
    pub tab: i8,
}

/// An editable line of text with a cursor and selection, owned by the
/// game. Indices are in characters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineEditor {
    text: String,
    cursor: usize,
    anchor: usize,
    max_len: usize,
}

impl LineEditor {
    /// `text` with the cursor at its end and no length limit.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let end = text.chars().count();
        Self {
            text,
            cursor: end,
            anchor: end,
            max_len: usize::MAX,
        }
    }
    /// Limits the text to `max_len` characters.
    pub fn with_max_len(mut self, max_len: usize) -> Self {
        self.max_len = max_len;
        self
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn anchor(&self) -> usize {
        self.anchor
    }
    /// The selected character range `(start, end)`; empty when equal.
    pub fn selection(&self) -> (usize, usize) {
        (self.cursor.min(self.anchor), self.cursor.max(self.anchor))
    }
    pub fn selected_text(&self) -> &str {
        let (start, end) = self.selection();
        &self.text[byte_index(&self.text, start)..byte_index(&self.text, end)]
    }
    /// Replaces the text and puts the cursor at its end.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.chars().count();
        self.anchor = self.cursor;
    }
    /// Moves the cursor (clamped), collapsing the selection.
    pub fn set_cursor(&mut self, cursor: usize) {
        self.cursor = cursor.min(self.text.chars().count());
        self.anchor = self.cursor;
    }
    pub fn clear(&mut self) {
        self.set_text(String::new());
    }
    /// Takes the text out, leaving the editor empty (for "submit").
    pub fn take(&mut self) -> String {
        let text = std::mem::take(&mut self.text);
        self.cursor = 0;
        self.anchor = 0;
        text
    }

    /// Applies one frame of input.
    pub fn apply(&mut self, input: &UiInput, measure: &dyn TextMeasure) -> LineEdit {
        let before = (self.cursor, self.anchor);
        let changed = edit_line(
            &mut self.text,
            &mut self.cursor,
            &mut self.anchor,
            self.max_len,
            input,
            measure,
        );
        LineEdit {
            changed,
            moved: changed || before != (self.cursor, self.anchor),
            submitted: input.activate,
            cancelled: input.cancel,
            vertical: i8::from(input.down) - i8::from(input.up),
            tab: i8::from(input.next) - i8::from(input.previous),
        }
    }

    /// Caret x offset (physical units) from the start of the text, using
    /// the measure's shaped caret when it has one.
    pub fn caret_x(&self, measure: &dyn TextMeasure, style: TextStyle, size: f32) -> f32 {
        caret_x(measure, &self.text, style, size, self.cursor)
    }
    /// Selection span `(x, width)` in physical units, if any.
    pub fn selection_x(
        &self,
        measure: &dyn TextMeasure,
        style: TextStyle,
        size: f32,
    ) -> Option<(f32, f32)> {
        let (start, end) = self.selection();
        (start != end).then(|| {
            let a = caret_x(measure, &self.text, style, size, start);
            let b = caret_x(measure, &self.text, style, size, end);
            (a, b - a)
        })
    }
    /// Cursor index nearest to `x` (physical units from the text start),
    /// for click-to-place.
    pub fn index_at(
        &self,
        measure: &dyn TextMeasure,
        style: TextStyle,
        size: f32,
        x: f32,
    ) -> usize {
        if let Some(index) = measure.caret_index(&self.text, style, size, x) {
            return index.min(self.text.chars().count());
        }
        let mut best = (0usize, f32::INFINITY);
        for index in 0..=self.text.chars().count() {
            let distance = (caret_x(measure, &self.text, style, size, index) - x).abs();
            if distance < best.1 {
                best = (index, distance);
            }
        }
        best.0
    }
}

fn caret_x(
    measure: &dyn TextMeasure,
    text: &str,
    style: TextStyle,
    size: f32,
    index: usize,
) -> f32 {
    measure
        .caret_x(text, style, size, index)
        .unwrap_or_else(|| {
            let prefix = &text[..byte_index(text, index)];
            measure.measure_style(prefix, style, size).x
        })
}

pub(crate) fn byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(index, _)| index)
}

/// The editing rules shared by [`LineEditor`] and `TextInput`. Returns
/// whether the text changed.
pub(crate) fn edit_line(
    text: &mut String,
    cursor_field: &mut usize,
    anchor_field: &mut usize,
    max_len: usize,
    input: &UiInput,
    measure: &dyn TextMeasure,
) -> bool {
    let original_len = text.len();
    let mut changed = false;
    let char_len = text.chars().count();
    let mut cursor = (*cursor_field).min(char_len);
    let mut anchor = (*anchor_field).min(char_len);
    if input.select_all {
        cursor = char_len;
        anchor = 0;
    }
    if input.home {
        cursor = 0;
        if !input.shift {
            anchor = 0;
        }
    }
    if input.end {
        cursor = char_len;
        if !input.shift {
            anchor = char_len;
        }
    }
    if input.left {
        cursor = measure.previous_boundary(text, cursor);
        if !input.shift {
            anchor = cursor;
        }
    }
    if input.right {
        cursor = measure.next_boundary(text, cursor);
        if !input.shift {
            anchor = cursor;
        }
    }
    if input.backspace || input.delete {
        let (start, end) = if cursor != anchor {
            (cursor.min(anchor), cursor.max(anchor))
        } else if input.backspace && cursor > 0 {
            (measure.previous_boundary(text, cursor), cursor)
        } else if input.delete && cursor < char_len {
            (cursor, measure.next_boundary(text, cursor))
        } else {
            (cursor, cursor)
        };
        if start != end {
            let start_byte = byte_index(text, start);
            let end_byte = byte_index(text, end);
            text.replace_range(start_byte..end_byte, "");
            cursor = start;
            anchor = start;
            changed = true;
        }
    }
    if !input.text.is_empty() {
        let (start, end) = (cursor.min(anchor), cursor.max(anchor));
        let start_byte = byte_index(text, start);
        let end_byte = byte_index(text, end);
        // Room left once the selection is replaced; input beyond it is
        // truncated (so a field limited to N holds exactly N characters).
        let room = max_len.saturating_sub(text.chars().count().saturating_sub(end - start));
        if room > 0 || start != end {
            let filtered: String = input
                .text
                .chars()
                .filter(|ch| !ch.is_control())
                .take(room)
                .collect();
            if !filtered.is_empty() || start != end {
                text.replace_range(start_byte..end_byte, &filtered);
                cursor = start + filtered.chars().count();
                anchor = cursor;
                changed = true;
            }
        }
    }
    *cursor_field = cursor.min(text.chars().count());
    *anchor_field = anchor.min(text.chars().count());
    changed || text.len() != original_len
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MonospaceMeasure, Vec2};

    struct Proportional;
    impl TextMeasure for Proportional {
        fn measure(&self, text: &str, size: f32) -> Vec2 {
            // 'i' is narrow, 'w' wide, everything else average.
            let width: f32 = text
                .chars()
                .map(|c| match c {
                    'i' | 'l' => 0.3,
                    'w' | 'm' => 0.9,
                    _ => 0.6,
                })
                .sum();
            Vec2::new(width * size, size)
        }
    }

    fn input(edit: impl FnOnce(&mut UiInput)) -> UiInput {
        let mut input = UiInput::default();
        edit(&mut input);
        input
    }

    #[test]
    fn edits_like_a_text_field() {
        let m = &MonospaceMeasure;
        let mut line = LineEditor::new("hello").with_max_len(8);
        assert_eq!(line.cursor(), 5);
        line.apply(&input(|i| i.home = true), m);
        line.apply(&input(|i| i.text = ">".into()), m);
        assert_eq!(line.text(), ">hello");
        line.apply(&input(|i| i.end = true), m);
        line.apply(&input(|i| i.text = " world".into()), m);
        assert_eq!(line.text(), ">hello w", "max length");
        let edit = line.apply(&input(|i| i.backspace = true), m);
        assert!(edit.changed && edit.moved);
        line.apply(
            &input(|i| {
                i.left = true;
                i.shift = true;
            }),
            m,
        );
        assert_eq!(line.selected_text(), " ");
        line.apply(&input(|i| i.delete = true), m);
        assert_eq!(line.text(), ">hello");
        line.apply(&input(|i| i.select_all = true), m);
        line.apply(&input(|i| i.text = "x".into()), m);
        assert_eq!(line.text(), "x");
        let control = line.apply(&input(|i| i.text = "\u{8}".into()), m);
        assert!(!control.changed, "control characters are dropped");
        let keys = line.apply(
            &input(|i| {
                i.up = true;
                i.next = true;
                i.cancel = true;
            }),
            m,
        );
        assert_eq!((keys.vertical, keys.tab, keys.cancelled), (-1, 1, true));
        assert_eq!(line.take(), "x");
        assert_eq!(line.text(), "");
    }

    #[test]
    fn carets_are_measured_for_proportional_fonts() {
        let mut line = LineEditor::new("iiwm");
        line.set_cursor(2);
        let size = 10.0;
        assert!((line.caret_x(&Proportional, TextStyle::Body, size) - 6.0).abs() < 1e-4);
        line.set_cursor(4);
        assert!((line.caret_x(&Proportional, TextStyle::Body, size) - 24.0).abs() < 1e-4);
        assert_eq!(line.index_at(&Proportional, TextStyle::Body, size, 14.0), 3);
        line.apply(
            &UiInput {
                home: true,
                shift: true,
                ..UiInput::default()
            },
            &Proportional,
        );
        let (x, width) = line
            .selection_x(&Proportional, TextStyle::Body, size)
            .unwrap();
        assert!(x.abs() < 1e-4 && (width - 24.0).abs() < 1e-4);
    }
}
