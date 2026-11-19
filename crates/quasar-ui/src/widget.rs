use crate::math::{Edges, Rect, Vec2};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeId(pub(crate) u64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ActionId(pub u64);

impl ActionId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Layer {
    #[default]
    Normal,
    Overlay,
    Hud,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Length {
    #[default]
    Auto,
    Fixed(f32),
    Percent(f32),
    Fill,
}

impl Length {
    pub const fn fixed(value: f32) -> Self {
        Self::Fixed(value)
    }
    pub const fn percent(value: f32) -> Self {
        Self::Percent(value)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
    Stretch,
}

/// Nine-point (plus stretch) placement inside a parent's content area.
///
/// Anchors apply to roots and to children of [`PanelLayout::Absolute`]
/// panels; stack and grid layouts position children themselves and ignore
/// them. The item's margin insets the anchored edge(s) and its offset is
/// added afterwards, so `Anchor::BottomRight` with a 16-unit margin keeps a
/// HUD element 16 units from both edges at any viewport size.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Anchor {
    #[default]
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
    /// Fills the parent area minus margins on both axes.
    Stretch,
}

impl Anchor {
    /// Horizontal and vertical factors: 0 = start, 0.5 = centre, 1 = end.
    /// `None` means stretch on that axis.
    pub const fn factors(self) -> (Option<f32>, Option<f32>) {
        match self {
            Self::TopLeft => (Some(0.0), Some(0.0)),
            Self::Top => (Some(0.5), Some(0.0)),
            Self::TopRight => (Some(1.0), Some(0.0)),
            Self::Left => (Some(0.0), Some(0.5)),
            Self::Center => (Some(0.5), Some(0.5)),
            Self::Right => (Some(1.0), Some(0.5)),
            Self::BottomLeft => (Some(0.0), Some(1.0)),
            Self::Bottom => (Some(0.5), Some(1.0)),
            Self::BottomRight => (Some(1.0), Some(1.0)),
            Self::Stretch => (None, None),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayoutItem {
    pub width: Length,
    pub height: Length,
    pub margin: Edges,
    pub offset: Vec2,
    pub align: Align,
    /// Optional anchor for roots and absolute-panel children. When `None`,
    /// `align` keeps its previous meaning.
    pub anchor: Option<Anchor>,
}

impl LayoutItem {
    pub const fn auto() -> Self {
        Self {
            width: Length::Auto,
            height: Length::Auto,
            margin: Edges::ZERO,
            offset: Vec2::ZERO,
            align: Align::Start,
            anchor: None,
        }
    }
    pub const fn fill() -> Self {
        Self {
            width: Length::Fill,
            height: Length::Fill,
            margin: Edges::ZERO,
            offset: Vec2::ZERO,
            align: Align::Stretch,
            anchor: None,
        }
    }
    pub const fn size(width: Length, height: Length) -> Self {
        Self {
            width,
            height,
            margin: Edges::ZERO,
            offset: Vec2::ZERO,
            align: Align::Start,
            anchor: None,
        }
    }
    pub const fn with_margin(mut self, margin: Edges) -> Self {
        self.margin = margin;
        self
    }
    pub const fn with_align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }
    pub const fn with_offset(mut self, offset: Vec2) -> Self {
        self.offset = offset;
        self
    }
    pub const fn with_anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = Some(anchor);
        self
    }
    /// An auto-sized item placed at `anchor`.
    pub const fn anchored(anchor: Anchor) -> Self {
        Self::auto().with_anchor(anchor)
    }
}

/// How a [`Widget::Label`] breaks lines.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum LabelWrap {
    /// One line per explicit `\n`; text never wraps.
    #[default]
    None,
    /// Greedy word wrapping at the label's width limit. Words longer than the
    /// limit break between characters.
    Word,
}

/// Former name of [`LabelWrap`]. It clashed with `quasar_text::TextWrap`, a
/// different type with overlapping variants.
#[deprecated(since = "0.2.25", note = "renamed to `LabelWrap`")]
pub type TextWrap = LabelWrap;

#[cfg(feature = "text")]
impl From<LabelWrap> for quasar_text::TextWrap {
    fn from(wrap: LabelWrap) -> Self {
        match wrap {
            LabelWrap::None => quasar_text::TextWrap::None,
            LabelWrap::Word => quasar_text::TextWrap::WordOrGlyph,
        }
    }
}

#[cfg(feature = "text")]
impl From<quasar_text::TextWrap> for LabelWrap {
    /// Any wrapping policy maps to [`LabelWrap::Word`], which already breaks
    /// overlong words between characters.
    fn from(wrap: quasar_text::TextWrap) -> Self {
        match wrap {
            quasar_text::TextWrap::None => LabelWrap::None,
            _ => LabelWrap::Word,
        }
    }
}

/// Border painted by a [`NodeStyle`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    /// Width in logical units (scaled by the UI scale).
    pub width: f32,
    pub color: Color,
}

/// One run of a [`Widget::RichText`].
#[derive(Clone, Debug, PartialEq)]
pub enum RichSpan {
    /// Words; wrapping happens at spaces.
    Text(UiText),
    /// An inline image of `width` x `height` logical units, drawn from the
    /// renderer texture registered for `image` (as [`Widget::Image`]), and
    /// centred on the text row.
    Image {
        image: u64,
        source: Option<Rect>,
        width: f32,
        height: f32,
    },
    /// Text in a key-cap box, for input prompts.
    Key(UiText),
    /// A forced line break.
    Break,
}

impl RichSpan {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(UiText::literal(text.into()))
    }
    pub fn key(text: impl Into<String>) -> Self {
        Self::Key(UiText::literal(text.into()))
    }
    pub const fn image(image: u64, width: f32, height: f32) -> Self {
        Self::Image {
            image,
            source: None,
            width,
            height,
        }
    }
}

/// One positioned piece of laid-out rich text (physical units, relative
/// to the widget's top-left corner).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RichItem {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub kind: RichKind,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RichKind {
    Text(String),
    Key(String),
    Image { image: u64, source: Option<Rect> },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RichLayout {
    pub items: Vec<RichItem>,
    pub size: Vec2,
}

/// Key-cap padding as a fraction of the font size.
pub(crate) const KEY_PAD: f32 = 0.35;

/// Lays out `spans` into rows no wider than `limit` (physical units).
/// `measure` returns a text run's physical width at the widget's size.
pub(crate) fn layout_rich(
    spans: &[RichSpan],
    resolve: impl Fn(&UiText) -> String,
    measure: impl Fn(&str) -> f32,
    font: f32,
    scale: f32,
    limit: f32,
) -> RichLayout {
    struct Piece {
        kind: RichKind,
        width: f32,
        height: f32,
        /// Whether a space precedes this piece on the same row.
        spaced: bool,
        /// Starts a new row.
        breaks: bool,
    }
    let space = measure(" ");
    let mut pieces: Vec<Piece> = Vec::new();
    let mut pending_space = false;
    let mut pending_break = false;
    for span in spans {
        match span {
            RichSpan::Text(text) => {
                let text = resolve(text);
                let mut first = true;
                for (index, line) in text.split('\n').enumerate() {
                    if index > 0 {
                        pending_break = true;
                        pending_space = false;
                    }
                    let starts_with_space = line.starts_with(char::is_whitespace);
                    for word in line.split_whitespace() {
                        let spaced = if first && !starts_with_space {
                            pending_space
                        } else {
                            pending_space || !first || starts_with_space
                        };
                        pieces.push(Piece {
                            kind: RichKind::Text(word.to_owned()),
                            width: measure(word),
                            height: font,
                            spaced: spaced && !pending_break,
                            breaks: std::mem::take(&mut pending_break),
                        });
                        first = false;
                        pending_space = false;
                    }
                    pending_space =
                        line.ends_with(char::is_whitespace) || (first && starts_with_space);
                }
            }
            RichSpan::Key(text) => {
                let text = resolve(text);
                pieces.push(Piece {
                    width: measure(&text) + font * KEY_PAD * 2.0,
                    height: font * 1.25,
                    kind: RichKind::Key(text),
                    spaced: pending_space && !pending_break,
                    breaks: std::mem::take(&mut pending_break),
                });
                pending_space = false;
            }
            RichSpan::Image {
                image,
                source,
                width,
                height,
            } => {
                pieces.push(Piece {
                    kind: RichKind::Image {
                        image: *image,
                        source: *source,
                    },
                    width: width.max(0.0) * scale,
                    height: height.max(0.0) * scale,
                    spaced: pending_space && !pending_break,
                    breaks: std::mem::take(&mut pending_break),
                });
                pending_space = false;
            }
            RichSpan::Break => {
                pending_break = true;
                pending_space = false;
            }
        }
    }
    // Greedy rows; text pieces on one row merge into a single run.
    let mut rows: Vec<Vec<(f32, Piece)>> = vec![Vec::new()];
    let mut x = 0.0f32;
    for piece in pieces {
        let gap = if piece.spaced { space } else { 0.0 };
        let row_empty = rows.last().is_none_or(Vec::is_empty);
        if piece.breaks || (!row_empty && x + gap + piece.width > limit + 0.5) {
            rows.push(Vec::new());
            x = 0.0;
        }
        let row = rows.last_mut().expect("a row");
        let gap = if row.is_empty() { 0.0 } else { gap };
        if let (Some((_, last)), RichKind::Text(word)) = (row.last_mut(), &piece.kind)
            && let RichKind::Text(run) = &mut last.kind
        {
            if gap > 0.0 {
                run.push(' ');
            }
            run.push_str(word);
            last.width += gap + piece.width;
            x += gap + piece.width;
            continue;
        }
        row.push((x + gap, piece));
        x += gap + rows.last().expect("a row").last().expect("piece").1.width;
    }
    let mut layout = RichLayout::default();
    let mut y = 0.0f32;
    for row in rows {
        let height = row.iter().map(|(_, p)| p.height).fold(font, f32::max);
        let mut right = 0.0f32;
        for (x, piece) in row {
            right = right.max(x + piece.width);
            layout.items.push(RichItem {
                x,
                y: y + (height - piece.height) * 0.5,
                width: piece.width,
                height: piece.height,
                kind: piece.kind,
            });
        }
        layout.size.x = layout.size.x.max(right);
        y += height;
    }
    layout.size.y = y;
    layout
}

/// Per-node colour overrides for a widget's own painting; `None` fields
/// keep the theme colour. Set with [`crate::Ui::set_colors`] (or
/// [`crate::Ui::set_text_color`] for just the text).
///
/// - `text`: label text of any style, and the text of buttons, checkboxes,
///   sliders, selects, bindings and fields (for example rarity-coloured
///   item names).
/// - `accent`: selected values (select option, binding value, tab).
/// - `background`, `hover`, `active`: a button's (and select's / binding's)
///   fill at rest, hovered or focused, and pressed. A button whose
///   [`NodeStyle`] has a background paints no theme fill at rest, so the
///   style shows through and only hover/press tints are drawn over it.
/// - `track`, `fill`: checkbox box and mark, slider and progress track and
///   fill, text-field background (`track`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WidgetColors {
    pub text: Option<Color>,
    pub accent: Option<Color>,
    pub background: Option<Color>,
    pub hover: Option<Color>,
    pub active: Option<Color>,
    pub track: Option<Color>,
    pub fill: Option<Color>,
}

impl WidgetColors {
    pub const fn text(color: Color) -> Self {
        Self {
            text: Some(color),
            accent: None,
            background: None,
            hover: None,
            active: None,
            track: None,
            fill: None,
        }
    }
    /// Button colours at rest, hovered and pressed.
    pub const fn button(background: Color, hover: Color, active: Color) -> Self {
        Self {
            text: None,
            accent: None,
            background: Some(background),
            hover: Some(hover),
            active: Some(active),
            track: None,
            fill: None,
        }
    }
    pub const fn with_text(mut self, color: Color) -> Self {
        self.text = Some(color);
        self
    }
    pub const fn with_track(mut self, color: Color) -> Self {
        self.track = Some(color);
        self
    }
    pub const fn with_fill(mut self, color: Color) -> Self {
        self.fill = Some(color);
        self
    }
    pub const fn with_accent(mut self, color: Color) -> Self {
        self.accent = Some(color);
        self
    }
}

/// The colours one node paints with: the theme merged with the node's
/// [`WidgetColors`] and style.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub(crate) struct Paint {
    pub text: Color,
    pub dim_text: Color,
    pub accent: Color,
    pub button: Color,
    pub button_hover: Color,
    pub button_active: Color,
    pub track: Color,
    pub fill: Color,
    /// Corner radius (logical units) for widget fills.
    pub radius: f32,
}

#[cfg_attr(not(feature = "render2d"), allow(dead_code))]
impl Paint {
    pub(crate) fn new(
        theme: &crate::Theme,
        colors: Option<&WidgetColors>,
        style: Option<&NodeStyle>,
    ) -> Self {
        let c = colors.copied().unwrap_or_default();
        let styled_background = style.is_some_and(|s| s.background.is_some());
        Self {
            text: c.text.unwrap_or(theme.text),
            dim_text: c.text.unwrap_or(theme.dim_text),
            accent: c.accent.unwrap_or(theme.accent),
            button: c.background.unwrap_or(if styled_background {
                Color::TRANSPARENT
            } else {
                theme.button
            }),
            button_hover: c.hover.unwrap_or(theme.button_hover),
            button_active: c.active.unwrap_or(theme.button_active),
            track: c.track.unwrap_or(theme.track),
            fill: c.fill.unwrap_or(theme.fill),
            radius: style.map_or(0.0, |s| s.radius),
        }
    }
    /// Like [`crate::Theme::color`], with the node's text override.
    pub(crate) fn color(&self, style: crate::TextStyle) -> Color {
        match style {
            crate::TextStyle::Body | crate::TextStyle::Heading => self.text,
            crate::TextStyle::Small => self.dim_text,
            crate::TextStyle::Dim => self.dim_text.with_alpha(190),
        }
    }
}

/// Per-node visual override.
///
/// A node with a style paints exactly this background, border and corner
/// radius instead of the theme's panel colour. `background: None` is
/// transparent, so a full-screen HUD root can host widgets without covering
/// the scene. Styles also apply behind non-panel widgets.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NodeStyle {
    pub background: Option<Color>,
    pub border: Option<Border>,
    /// Corner radius in logical units; 0 draws square corners.
    pub radius: f32,
}

impl NodeStyle {
    /// No background, no border.
    pub const fn transparent() -> Self {
        Self {
            background: None,
            border: None,
            radius: 0.0,
        }
    }
    pub const fn filled(color: Color) -> Self {
        Self {
            background: Some(color),
            border: None,
            radius: 0.0,
        }
    }
    pub const fn with_border(mut self, width: f32, color: Color) -> Self {
        self.border = Some(Border { width, color });
        self
    }
    pub const fn with_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }
}

/// Horizontal spans that cover `rect` with rounded corners of `radius`
/// (physical units). Renderers draw these as plain rectangles, so rounded
/// corners need no shader support. Returns one rectangle when `radius` is 0.
pub fn rounded_rect_spans(rect: Rect, radius: f32) -> Vec<Rect> {
    if !rect.valid() || rect.width <= 0.0 || rect.height <= 0.0 {
        return Vec::new();
    }
    let radius = radius.max(0.0).min(rect.width * 0.5).min(rect.height * 0.5);
    if radius < 0.5 {
        return vec![rect];
    }
    let rows = radius.ceil() as usize;
    let row_height = radius / rows as f32;
    let mut spans = Vec::with_capacity(rows * 2 + 1);
    let inset = |row: usize| {
        // Distance from the corner circle's centre to the row centre.
        let dy = radius - (row as f32 + 0.5) * row_height;
        radius - (radius * radius - dy * dy).max(0.0).sqrt()
    };
    for row in 0..rows {
        let dx = inset(row);
        spans.push(Rect::new(
            rect.x + dx,
            rect.y + row as f32 * row_height,
            rect.width - dx * 2.0,
            row_height,
        ));
    }
    spans.push(Rect::new(
        rect.x,
        rect.y + radius,
        rect.width,
        rect.height - radius * 2.0,
    ));
    for row in 0..rows {
        let dx = inset(row);
        spans.push(Rect::new(
            rect.x + dx,
            rect.bottom() - (row as f32 + 1.0) * row_height,
            rect.width - dx * 2.0,
            row_height,
        ));
    }
    spans.retain(|span| span.width > 0.0 && span.height > 0.0);
    spans
}

/// Spans that cover a `width`-thick border around `rect` with rounded corners.
pub fn rounded_border_spans(rect: Rect, radius: f32, width: f32) -> Vec<Rect> {
    if width <= 0.0 || !rect.valid() {
        return Vec::new();
    }
    let width = width.min(rect.width * 0.5).min(rect.height * 0.5);
    let radius = radius.max(0.0).min(rect.width * 0.5).min(rect.height * 0.5);
    if radius < 0.5 {
        return vec![
            Rect::new(rect.x, rect.y, rect.width, width),
            Rect::new(rect.x, rect.bottom() - width, rect.width, width),
            Rect::new(rect.x, rect.y + width, width, rect.height - width * 2.0),
            Rect::new(
                rect.right() - width,
                rect.y + width,
                width,
                rect.height - width * 2.0,
            ),
        ];
    }
    let outer = rounded_rect_spans(rect, radius);
    let inner_rect = Rect::new(
        rect.x + width,
        rect.y + width,
        rect.width - width * 2.0,
        rect.height - width * 2.0,
    );
    let inner_radius = (radius - width).max(0.0);
    let mut spans = Vec::new();
    for span in outer {
        let middle = span.y + span.height * 0.5;
        let inner = if middle < inner_rect.y || middle > inner_rect.bottom() {
            None
        } else {
            let dy = if middle < inner_rect.y + inner_radius {
                inner_rect.y + inner_radius - middle
            } else if middle > inner_rect.bottom() - inner_radius {
                middle - (inner_rect.bottom() - inner_radius)
            } else {
                0.0
            };
            let dx = if inner_radius > 0.0 && dy > 0.0 {
                inner_radius - (inner_radius * inner_radius - dy * dy).max(0.0).sqrt()
            } else {
                0.0
            };
            Some((inner_rect.x + dx, inner_rect.right() - dx))
        };
        match inner {
            Some((left, right)) if right > left => {
                spans.push(Rect::new(span.x, span.y, left - span.x, span.height));
                spans.push(Rect::new(right, span.y, span.right() - right, span.height));
            }
            _ => spans.push(span),
        }
    }
    spans.retain(|span| span.width > 0.0 && span.height > 0.0);
    spans
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum StackDirection {
    #[default]
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PanelLayout {
    Stack { direction: StackDirection, gap: f32 },
    Grid { columns: u32, gap: f32 },
    Absolute,
}

impl Default for PanelLayout {
    fn default() -> Self {
        Self::Stack {
            direction: StackDirection::Vertical,
            gap: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct UiText {
    pub key: Option<String>,
    pub fallback: String,
}

impl UiText {
    pub fn literal(value: impl Into<String>) -> Self {
        Self {
            key: None,
            fallback: value.into(),
        }
    }
    pub fn keyed(key: impl Into<String>, fallback: impl Into<String>) -> Self {
        Self {
            key: Some(key.into()),
            fallback: fallback.into(),
        }
    }
    pub fn key(key: impl Into<String>) -> Self {
        Self {
            key: Some(key.into()),
            fallback: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Widget {
    Panel {
        layout: PanelLayout,
        padding: Edges,
        clip: bool,
        scrollable: bool,
    },
    Label {
        text: UiText,
        style: TextStyle,
        /// Line breaking. [`LabelWrap::Word`] wraps at the narrowest of the
        /// available width, a fixed/percent item width and `max_width`, and
        /// the label's height grows with its line count during layout.
        wrap: LabelWrap,
        /// Optional wrap limit in logical units.
        max_width: Option<f32>,
    },
    /// Text with inline images and key caps (button prompts such as
    /// "Press \[F\] to open"), laid out in rows and word-wrapped like a
    /// label. See [`RichSpan`].
    RichText {
        spans: Vec<RichSpan>,
        style: TextStyle,
        wrap: LabelWrap,
        /// Optional wrap limit in logical units.
        max_width: Option<f32>,
    },
    Button {
        text: UiText,
        action: Option<ActionId>,
    },
    Slider {
        label: UiText,
        value: f32,
        min: f32,
        max: f32,
        step: f32,
        action: Option<ActionId>,
    },
    Checkbox {
        label: UiText,
        checked: bool,
        action: Option<ActionId>,
    },
    Select {
        label: UiText,
        options: Vec<UiText>,
        selected: usize,
        action: Option<ActionId>,
    },
    Tabs {
        tabs: Vec<UiText>,
        selected: usize,
        action: Option<ActionId>,
    },
    TextInput {
        text: String,
        placeholder: UiText,
        cursor: usize,
        anchor: usize,
        max_len: usize,
        password: bool,
        action: Option<ActionId>,
    },
    Progress {
        label: UiText,
        value: f32,
        max: f32,
    },
    Image {
        image: u64,
        source: Option<Rect>,
        size: Vec2,
    },
    List {
        items: Vec<UiText>,
        selected: usize,
        scroll: usize,
        action: Option<ActionId>,
    },
    Binding {
        label: UiText,
        action: String,
        value: String,
    },
    Spacer,
}

impl Widget {
    /// A single-line label (explicit `\n` still breaks lines).
    pub fn label(text: UiText, style: TextStyle) -> Self {
        Self::Label {
            text,
            style,
            wrap: LabelWrap::None,
            max_width: None,
        }
    }
    /// A word-wrapped label, optionally limited to `max_width` logical units.
    pub fn wrapped_label(text: UiText, style: TextStyle, max_width: Option<f32>) -> Self {
        Self::Label {
            text,
            style,
            wrap: LabelWrap::Word,
            max_width,
        }
    }

    pub fn focusable(&self) -> bool {
        matches!(
            self,
            Self::Button { .. }
                | Self::Slider { .. }
                | Self::Checkbox { .. }
                | Self::Select { .. }
                | Self::Tabs { .. }
                | Self::TextInput { .. }
                | Self::List { .. }
                | Self::Binding { .. }
        )
    }
}

use crate::theme::{Color, TextStyle};

#[derive(Clone, Copy, Debug, Default)]
pub struct WidgetState {
    pub hovered: bool,
    pub pressed: bool,
    pub hover_t: f32,
    pub focus_t: f32,
    pub press_t: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutEntry {
    pub id: NodeId,
    pub rect: Rect,
    pub clip: Rect,
    pub layer: Layer,
}

#[derive(Clone, Debug, PartialEq)]
pub enum UiValue {
    Float(f32),
    Bool(bool),
    UInt(u32),
    Text(String),
}

impl UiValue {
    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Self::Float(value) => Some(*value),
            Self::UInt(value) => Some(*value as f32),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum UiEvent {
    Pressed(ActionId),
    Changed {
        id: NodeId,
        value: UiValue,
    },
    TextChanged {
        id: NodeId,
        text: String,
    },
    Submitted {
        id: NodeId,
        text: String,
    },
    FocusChanged(Option<NodeId>),
    RebindRequested {
        id: NodeId,
        action: String,
    },
    RebindCaptured {
        id: NodeId,
        action: String,
        value: String,
    },
    Cancelled,
}

#[derive(Clone, Debug, Default)]
pub struct UiInput {
    pub pointer: Option<Vec2>,
    pub primary_down: bool,
    pub primary_pressed: bool,
    pub primary_released: bool,
    pub scroll: Vec2,
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub next: bool,
    pub previous: bool,
    pub activate: bool,
    pub cancel: bool,
    pub text: String,
    pub backspace: bool,
    pub delete: bool,
    pub home: bool,
    pub end: bool,
    pub shift: bool,
    /// Committed IME preedit text, rendered but not yet inserted into the field.
    pub preedit: String,
    /// Preedit cursor as a character index.
    pub preedit_cursor: usize,
    pub select_all: bool,
}

impl UiInput {
    pub fn with_pointer(mut self, x: f32, y: f32) -> Self {
        self.pointer = Some(Vec2::new(x, y));
        self
    }
    pub fn pressed(mut self) -> Self {
        self.primary_pressed = true;
        self.primary_down = true;
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct UiFrame {
    pub events: Vec<UiEvent>,
    pub focus: Option<NodeId>,
    pub consumed: bool,
    pub consumed_pointer: bool,
    pub consumed_keyboard: bool,
    pub consumed_text: bool,
    pub modal: bool,
    pub capturing: bool,
}

pub trait TextMeasure {
    /// Returns the physical text extent for a physical font size.
    fn measure(&self, text: &str, size: f32) -> Vec2;

    /// Measures with a semantic style. Bitmap and monospace implementations
    /// can ignore the style; shaped text uses it for family and weight.
    fn measure_style(&self, text: &str, _style: TextStyle, size: f32) -> Vec2 {
        self.measure(text, size)
    }

    /// Returns the character index nearest to a horizontal layout position, or
    /// `None` when the implementation has no shaped hit-testing.
    fn caret_index(&self, _text: &str, _style: TextStyle, _size: f32, _x: f32) -> Option<usize> {
        None
    }

    /// Returns the horizontal caret position for a character index, or `None`.
    fn caret_x(&self, _text: &str, _style: TextStyle, _size: f32, _index: usize) -> Option<f32> {
        None
    }

    /// Previous editing boundary (grapheme cluster where supported).
    fn previous_boundary(&self, text: &str, index: usize) -> usize {
        index.min(text.chars().count()).saturating_sub(1)
    }

    /// Next editing boundary (grapheme cluster where supported).
    fn next_boundary(&self, text: &str, index: usize) -> usize {
        (index + 1).min(text.chars().count())
    }

    /// Breaks `text` into lines no wider than `max_width` (physical units).
    /// The default is greedy word wrapping over [`Self::measure_style`], which
    /// suits bitmap and monospace measures; shaped measures override it.
    fn wrap_lines(&self, text: &str, style: TextStyle, size: f32, max_width: f32) -> Vec<String> {
        wrap_greedy(text, max_width, |part| {
            self.measure_style(part, style, size).x
        })
    }

    /// Extent of `text` wrapped at `max_width` (physical units).
    fn measure_wrapped(&self, text: &str, style: TextStyle, size: f32, max_width: f32) -> Vec2 {
        let lines = self.wrap_lines(text, style, size, max_width);
        let line_height = self.measure_style("M", style, size).y.max(0.0);
        let width = lines
            .iter()
            .map(|line| self.measure_style(line, style, size).x)
            .fold(0.0, f32::max);
        Vec2::new(width, line_height * lines.len().max(1) as f32)
    }
}

/// Greedy word wrapping with a caller-supplied width function.
///
/// Explicit `\n` always breaks. Runs of spaces between words collapse to the
/// single space measured with the next word; a word wider than `max_width`
/// is split between characters so no line overflows unless one character
/// alone is wider than the limit. Non-finite or non-positive limits disable
/// wrapping.
pub fn wrap_greedy(text: &str, max_width: f32, width: impl Fn(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if !max_width.is_finite() || max_width <= 0.0 {
            lines.push(paragraph.to_owned());
            continue;
        }
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if width(&candidate) <= max_width {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            if width(word) <= max_width {
                line = word.to_owned();
                continue;
            }
            for ch in word.chars() {
                let mut next = line.clone();
                next.push(ch);
                if !line.is_empty() && width(&next) > max_width {
                    lines.push(std::mem::take(&mut line));
                    line.push(ch);
                } else {
                    line = next;
                }
            }
        }
        lines.push(line);
    }
    lines
}

pub struct MonospaceMeasure;

impl TextMeasure for MonospaceMeasure {
    fn measure(&self, text: &str, size: f32) -> Vec2 {
        Vec2::new(text.chars().count() as f32 * size * 0.6, size)
    }
}
