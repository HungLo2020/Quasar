//! Frame-level input snapshot and explicit consumer consumption.
//!
//! `InputState` accumulates native transitions until a simulation tick. A
//! `FrameInput` snapshot is captured once per runner frame. Variable-rate UI
//! and rendering can inspect it even when no fixed tick is due, while fixed
//! simulation ticks see the same edges exactly once.
use crate::{ActionState, Button, GamepadAxis, InputRead, InputState, Key, MouseButton};
use std::collections::{HashMap, HashSet};

/// Consumption requests recorded by one or more input consumers.
///
/// The runner applies these to the underlying [`InputState`] after all frame
/// callbacks. Consumed edges are not retried on the next frame. Device positions and held
/// values remain authoritative until a native transition changes them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConsumedInput {
    pub all: bool,
    pub actions: HashSet<String>,
    pub buttons: HashSet<Button>,
    pub text: bool,
    pub pointer: bool,
    pub relative_motion: bool,
    pub scroll: bool,
}

impl ConsumedInput {
    fn clear(&mut self) {
        self.all = false;
        self.actions.clear();
        self.buttons.clear();
        self.text = false;
        self.pointer = false;
        self.relative_motion = false;
        self.scroll = false;
    }
}

/// Deterministic input snapshot for one application frame.
///
/// It owns copies of transient edges, action values and pointer deltas, and a
/// copy of held devices. The copy buffers are reused each frame by the runner;
/// steady-state frames do not allocate when capacity is already sufficient.
#[derive(Clone, Debug, Default)]
pub struct FrameInput {
    frame: u64,
    held: HashSet<Button>,
    pressed: HashSet<Button>,
    released: HashSet<Button>,
    actions: HashMap<String, ActionState>,
    gamepad_axes: HashMap<GamepadAxis, f32>,
    cursor_physical: Option<(f64, f64)>,
    relative_motion: (f64, f64),
    scroll_lines: (f64, f64),
    scroll_pixels: (f64, f64),
    text: String,
    preedit: String,
    preedit_cursor: usize,
    consumed: ConsumedInput,
}

impl InputRead for FrameInput {
    fn action(&self, name: &str) -> ActionState {
        FrameInput::action(self, name)
    }
    fn held(&self, key: Key) -> bool {
        FrameInput::held(self, key)
    }
    fn pressed(&self, key: Key) -> bool {
        FrameInput::pressed(self, key)
    }
    fn released(&self, key: Key) -> bool {
        FrameInput::released(self, key)
    }
    fn mouse_held(&self, button: MouseButton) -> bool {
        FrameInput::mouse_held(self, button)
    }
    fn mouse_pressed(&self, button: MouseButton) -> bool {
        FrameInput::mouse_pressed(self, button)
    }
    fn mouse_released(&self, button: MouseButton) -> bool {
        FrameInput::mouse_released(self, button)
    }
    fn cursor_physical(&self) -> Option<(f64, f64)> {
        self.cursor_physical
    }
    fn relative_motion(&self) -> (f64, f64) {
        self.relative_motion
    }
}

impl FrameInput {
    /// Captures the pending transitions and current held state.
    pub fn capture(&mut self, state: &InputState, frame: u64) {
        self.frame = frame;
        self.held.clear();
        self.held.extend(state.held.iter().copied());
        self.pressed.clear();
        self.pressed.extend(state.pressed.iter().copied());
        self.released.clear();
        self.released.extend(state.released.iter().copied());
        // Update in place so steady-state frames do not clone action names.
        for (name, value) in &state.actions {
            if let Some(current) = self.actions.get_mut(name) {
                *current = *value;
            } else {
                self.actions.insert(name.clone(), *value);
            }
        }
        self.actions
            .retain(|name, _| state.actions.contains_key(name));
        self.gamepad_axes.clear();
        self.gamepad_axes.extend(
            state
                .gamepad_axes
                .iter()
                .map(|(axis, value)| (*axis, *value)),
        );
        self.cursor_physical = state.cursor_physical;
        self.relative_motion = state.relative_motion;
        self.scroll_lines = state.scroll_lines;
        self.scroll_pixels = state.scroll_pixels;
        self.text.clone_from(&state.text);
        self.preedit.clone_from(&state.preedit);
        self.preedit_cursor = state.preedit_cursor;
        self.consumed.clear();
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn consumed(&self) -> &ConsumedInput {
        &self.consumed
    }

    pub fn held(&self, key: Key) -> bool {
        self.button_held(Button::Key(key))
    }

    pub fn pressed(&self, key: Key) -> bool {
        self.button_pressed(Button::Key(key))
    }

    pub fn released(&self, key: Key) -> bool {
        self.button_released(Button::Key(key))
    }

    pub fn mouse_held(&self, button: MouseButton) -> bool {
        self.button_held(Button::Mouse(button))
    }

    pub fn mouse_pressed(&self, button: MouseButton) -> bool {
        self.button_pressed(Button::Mouse(button))
    }

    pub fn mouse_released(&self, button: MouseButton) -> bool {
        self.button_released(Button::Mouse(button))
    }

    pub fn button_held(&self, button: Button) -> bool {
        self.held.contains(&button)
    }

    pub fn button_pressed(&self, button: Button) -> bool {
        self.pressed.contains(&button)
    }

    pub fn button_released(&self, button: Button) -> bool {
        self.released.contains(&button)
    }

    pub fn action(&self, name: &str) -> ActionState {
        self.actions.get(name).copied().unwrap_or_default()
    }

    pub fn gamepad_axis(&self, axis: GamepadAxis) -> f32 {
        self.gamepad_axes.get(&axis).copied().unwrap_or(0.0)
    }

    pub fn cursor_physical(&self) -> Option<(f64, f64)> {
        self.cursor_physical
    }

    pub fn relative_motion(&self) -> (f64, f64) {
        self.relative_motion
    }

    pub fn scroll_lines(&self) -> (f64, f64) {
        self.scroll_lines
    }

    pub fn scroll_pixels(&self) -> (f64, f64) {
        self.scroll_pixels
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn preedit(&self) -> &str {
        &self.preedit
    }

    pub fn preedit_cursor(&self) -> usize {
        self.preedit_cursor
    }

    /// Marks one action's edge and value as handled by a consumer.
    pub fn consume_action(&mut self, name: &str) -> bool {
        let found = self
            .actions
            .remove(name)
            .is_some_and(|state| state.pressed || state.released || state.value != 0.0);
        if found {
            self.consumed.actions.insert(name.to_owned());
        }
        found
    }

    /// Marks one physical button as fully handled (held, press and release).
    pub fn consume_button(&mut self, button: Button) -> bool {
        let found = self.held.remove(&button)
            | self.pressed.remove(&button)
            | self.released.remove(&button);
        if found {
            self.consumed.buttons.insert(button);
        }
        found
    }

    pub fn consume_pressed(&mut self, key: Key) -> bool {
        let button = Button::Key(key);
        let found = self.pressed.remove(&button);
        if found {
            self.consumed.buttons.insert(button);
        }
        found
    }

    /// Consumes every key action/edge currently visible to this frame.
    pub fn consume_keyboard(&mut self) -> bool {
        let mut found = false;
        let keys: Vec<Button> = self
            .held
            .iter()
            .chain(self.pressed.iter())
            .chain(self.released.iter())
            .copied()
            .filter(|button| matches!(button, Button::Key(_)))
            .collect();
        for button in keys {
            found |= self.consume_button(button);
        }
        let mut consumed_actions = Vec::new();
        for (name, action) in &mut self.actions {
            if action.pressed || action.released || action.value != 0.0 {
                found = true;
                consumed_actions.push(name.clone());
                action.pressed = false;
                action.released = false;
                action.value = 0.0;
            }
        }
        self.consumed.actions.extend(consumed_actions);
        found
    }

    /// Consumes mouse buttons and pointer position for UI/render-only handling.
    pub fn consume_pointer(&mut self) -> bool {
        let mut found = self.cursor_physical.take().is_some();
        let mouse: Vec<Button> = self
            .held
            .iter()
            .chain(self.pressed.iter())
            .chain(self.released.iter())
            .copied()
            .filter(|button| matches!(button, Button::Mouse(_)))
            .collect();
        for button in mouse {
            found |= self.consume_button(button);
        }
        if found {
            self.consumed.pointer = true;
        }
        found
    }

    pub fn consume_text(&mut self) -> bool {
        let found = !self.text.is_empty();
        if found {
            self.text.clear();
            self.consumed.text = true;
        }
        found
    }

    pub fn consume_relative_motion(&mut self) -> bool {
        let found = self.relative_motion != (0.0, 0.0);
        if found {
            self.relative_motion = (0.0, 0.0);
            self.consumed.relative_motion = true;
        }
        found
    }

    pub fn consume_scroll(&mut self) -> bool {
        let found = self.scroll_lines != (0.0, 0.0) || self.scroll_pixels != (0.0, 0.0);
        if found {
            self.scroll_lines = (0.0, 0.0);
            self.scroll_pixels = (0.0, 0.0);
            self.consumed.scroll = true;
        }
        found
    }

    /// Consumes all currently visible input. Intended for modal UI or a fully
    /// paused gameplay state.
    pub fn consume_all(&mut self) {
        self.consumed.all = true;
        self.held.clear();
        self.pressed.clear();
        self.released.clear();
        self.actions.clear();
        self.gamepad_axes.clear();
        self.relative_motion = (0.0, 0.0);
        self.scroll_lines = (0.0, 0.0);
        self.scroll_pixels = (0.0, 0.0);
        self.text.clear();
        self.preedit.clear();
        self.preedit_cursor = 0;
        self.cursor_physical = None;
    }

    /// Clears transient edges after the first fixed tick, retaining held values
    /// for subsequent ticks in the same frame.
    pub fn clear_transient(&mut self) {
        self.pressed.clear();
        self.released.clear();
        for state in self.actions.values_mut() {
            state.pressed = false;
            state.released = false;
        }
        self.relative_motion = (0.0, 0.0);
        self.scroll_lines = (0.0, 0.0);
        self.scroll_pixels = (0.0, 0.0);
        self.text.clear();
        self.consumed.clear();
    }

    pub fn apply_consumption(&self, state: &mut InputState) {
        if self.consumed.all {
            state.consume_edges();
            return;
        }
        for action in &self.consumed.actions {
            if let Some(current) = state.actions.get_mut(action) {
                current.pressed = false;
                current.released = false;
            }
        }
        for button in &self.consumed.buttons {
            state.pressed.remove(button);
            state.released.remove(button);
        }
        if self.consumed.text {
            state.text.clear();
        }
        if self.consumed.relative_motion {
            state.relative_motion = (0.0, 0.0);
        }
        if self.consumed.scroll {
            state.scroll_lines = (0.0, 0.0);
            state.scroll_pixels = (0.0, 0.0);
        }
    }
}
