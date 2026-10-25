//! Optional `quasar-input` adapter. Games can also build `UiInput` manually.
use crate::UiInput;
use quasar_input::{FrameInput, InputState, Key};

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

/// Maps a runner frame snapshot into a UI snapshot.
///
/// This is the preferred path for applications using `quasar-app`: the same
/// edges are visible to UI and fixed simulation, and UI consumption can be
/// routed explicitly through `FrameInput`.
pub fn from_frame_input(input: &FrameInput, bindings: &UiBindings) -> UiInput {
    let pressed = |name: &str| input.action(name).pressed;
    UiInput {
        pointer: input
            .cursor_physical()
            .map(|(x, y)| crate::Vec2::new(x as f32, y as f32)),
        primary_down: input.mouse_held(quasar_input::MouseButton::Left),
        primary_pressed: input.mouse_pressed(quasar_input::MouseButton::Left),
        primary_released: input.mouse_released(quasar_input::MouseButton::Left),
        scroll: crate::Vec2::new(input.scroll_lines().0 as f32, input.scroll_lines().1 as f32),
        up: pressed(&bindings.up),
        down: pressed(&bindings.down),
        left: pressed(&bindings.left),
        right: pressed(&bindings.right),
        next: pressed(&bindings.next),
        previous: pressed(&bindings.previous),
        activate: pressed(&bindings.activate),
        cancel: pressed(&bindings.cancel),
        text: input.text().to_owned(),
        backspace: input.pressed(Key::Backspace),
        delete: input.pressed(Key::Delete),
        home: input.pressed(Key::Home),
        end: input.pressed(Key::End),
        shift: input.held(Key::ShiftLeft) || input.held(Key::ShiftRight),
        preedit: input.preedit().to_owned(),
        preedit_cursor: input.preedit_cursor(),
        select_all: (input.held(Key::ControlLeft) || input.held(Key::ControlRight))
            && input.pressed(Key::KeyA),
    }
}

/// Maps physical/action input into a UI snapshot. Text is passed separately from
/// physical key state, preserving IME/committed text.
///
/// Prefer [`from_frame_input`] with the runner's frame snapshot.
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

#[cfg(test)]
mod tests {
    use super::*;
    use quasar_input::{Binding, Bindings, MouseButton};

    #[test]
    fn frame_input_maps_actions_pointer_text_and_modifiers() {
        let mut bindings = Bindings::default();
        bindings.bind("ui_activate", Binding::key(Key::Enter));
        let mut state = InputState::default();
        state.set_bindings(bindings).unwrap();
        state.key(Key::Enter, true);
        state.key(Key::ShiftLeft, true);
        state.mouse_button(MouseButton::Left, true);
        state.cursor_physical = Some((12.0, 34.0));
        state.scroll_lines = (0.0, -2.0);
        state.text = "é".into();
        state.preedit = "e".into();
        state.preedit_cursor = 1;
        let frame = state.frame(1);
        let input = from_frame_input(&frame, &UiBindings::default());
        assert!(input.activate);
        assert!(input.primary_down);
        assert!(input.primary_pressed);
        assert_eq!(input.pointer, Some(crate::Vec2::new(12.0, 34.0)));
        assert_eq!(input.scroll, crate::Vec2::new(0.0, -2.0));
        assert_eq!(input.text, "é");
        assert_eq!(input.preedit, "e");
        assert_eq!(input.preedit_cursor, 1);
        assert!(input.shift);
    }

    #[test]
    fn frame_input_does_not_report_a_held_action_as_pressed() {
        let mut bindings = Bindings::default();
        bindings.bind("ui_activate", Binding::key(Key::Enter));
        let mut state = InputState::default();
        state.set_bindings(bindings).unwrap();
        state.key(Key::Enter, true);
        let mut frame = state.frame(1);
        assert!(from_frame_input(&frame, &UiBindings::default()).activate);
        // The fixed tick consumes the first-frame edge.
        frame.apply_consumption(&mut state);
        state.consume_edges();
        frame.clear_transient();
        let next = state.frame(2);
        assert!(!from_frame_input(&next, &UiBindings::default()).activate);
    }
}
