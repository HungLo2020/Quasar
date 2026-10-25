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

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayoutItem {
    pub width: Length,
    pub height: Length,
    pub margin: Edges,
    pub offset: Vec2,
    pub align: Align,
}

impl LayoutItem {
    pub const fn auto() -> Self {
        Self {
            width: Length::Auto,
            height: Length::Auto,
            margin: Edges::ZERO,
            offset: Vec2::ZERO,
            align: Align::Start,
        }
    }
    pub const fn fill() -> Self {
        Self {
            width: Length::Fill,
            height: Length::Fill,
            margin: Edges::ZERO,
            offset: Vec2::ZERO,
            align: Align::Stretch,
        }
    }
    pub const fn size(width: Length, height: Length) -> Self {
        Self {
            width,
            height,
            margin: Edges::ZERO,
            offset: Vec2::ZERO,
            align: Align::Start,
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

use crate::theme::TextStyle;

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
}

pub struct MonospaceMeasure;

impl TextMeasure for MonospaceMeasure {
    fn measure(&self, text: &str, size: f32) -> Vec2 {
        Vec2::new(text.chars().count() as f32 * size * 0.6, size)
    }
}
