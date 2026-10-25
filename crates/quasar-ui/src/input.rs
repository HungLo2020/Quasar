//! Optional `quasar-input` adapter. Games can also build `UiInput` manually.
use crate::UiInput;
use quasar_input::{InputState, Key};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiBindings {
    pub up: String,
    pub down: String,
    pub left: String,
    pub right: String,
    pub next: String,
    pub previous: String,
    pub activate: String,
    pub cancel: String,
}

impl Default for UiBindings {
    fn default() -> Self {
        Self::new(
            "ui_up",
            "ui_down",
            "ui_left",
            "ui_right",
            "ui_next",
            "ui_previous",
            "ui_activate",
            "ui_cancel",
        )
    }
}

impl UiBindings {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        up: impl Into<String>,
        down: impl Into<String>,
        left: impl Into<String>,
        right: impl Into<String>,
        next: impl Into<String>,
        previous: impl Into<String>,
        activate: impl Into<String>,
        cancel: impl Into<String>,
    ) -> Self {
        Self {
            up: up.into(),
            down: down.into(),
            left: left.into(),
            right: right.into(),
            next: next.into(),
            previous: previous.into(),
            activate: activate.into(),
            cancel: cancel.into(),
        }
    }
}

/// Maps physical/action input into a UI snapshot. Text is passed separately from
/// physical key state, preserving IME/committed text.
pub fn from_input_state(input: &InputState, bindings: &UiBindings) -> UiInput {
    let pressed = |name: &str| input.action(name).pressed;
    UiInput {
        pointer: input
            .cursor_physical
            .map(|(x, y)| crate::Vec2::new(x as f32, y as f32)),
        primary_down: input.mouse_held(quasar_input::MouseButton::Left),
        primary_pressed: input.mouse_pressed(quasar_input::MouseButton::Left),
        primary_released: input.mouse_released(quasar_input::MouseButton::Left),
        scroll: crate::Vec2::new(input.scroll_lines.0 as f32, input.scroll_lines.1 as f32),
        up: pressed(&bindings.up),
        down: pressed(&bindings.down),
        left: pressed(&bindings.left),
        right: pressed(&bindings.right),
        next: pressed(&bindings.next),
        previous: pressed(&bindings.previous),
        activate: pressed(&bindings.activate),
        cancel: pressed(&bindings.cancel),
        text: input.text.clone(),
        backspace: input.pressed(Key::Backspace),
        delete: input.pressed(Key::Delete),
        home: input.pressed(Key::Home),
        end: input.pressed(Key::End),
        shift: input.held(Key::ShiftLeft) || input.held(Key::ShiftRight),
        preedit: input.preedit.clone(),
        preedit_cursor: input.preedit_cursor,
        select_all: (input.held(Key::ControlLeft) || input.held(Key::ControlRight))
            && input.pressed(Key::KeyA),
    }
}
