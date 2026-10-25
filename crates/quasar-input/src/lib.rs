//! Portable device state and named action mapping. No window or graphics dependencies.
//! Feed physical button changes from any backend (or a script), then read named
//! action values/edges. Consume edges after a simulation tick, not after rendering.
//!
//! Controller types, player routing and the pure [`GamepadHub`] event path are
//! always available. Enable the optional `gilrs` feature for the native
//! `GilrsGamepads` adapter; without it the core remains free of platform device
//! libraries and accepts synthetic [`GamepadEvent`] values.
mod bindings;
mod gamepad;
mod key;
pub use bindings::{Binding, BindingError, Bindings};
#[cfg(feature = "gilrs")]
pub use gamepad::GilrsGamepads;
pub use gamepad::{
    AxisCalibration, GamepadAxis, GamepadButton, GamepadConfig, GamepadError, GamepadEvent,
    GamepadEventKind, GamepadFingerprint, GamepadHub, GamepadId, GamepadInfo, GamepadPreferences,
    GamepadSet, GamepadState, PlayerGamepadPreference, PlayerId, PlayerInput, StickConfig,
    TriggerConfig,
};
pub use key::Key;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
    Other(u16),
}
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum Button {
    Key(Key),
    Mouse(MouseButton),
    Gamepad(GamepadButton),
}

impl Button {
    /// A gamepad button on the player owned by this [`InputState`].
    ///
    /// There is intentionally no device id here: assign a device to a
    /// [`PlayerInput`]/[`GamepadHub`] and keep one input state per player.
    pub const fn gamepad(button: GamepadButton) -> Self {
        Self::Gamepad(button)
    }
}

/// Edges are accumulated across native events, even for a press/release between ticks.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct ActionState {
    pub value: f32,
    pub pressed: bool,
    pub released: bool,
}
impl ActionState {
    pub fn held(self) -> bool {
        self.value != 0.0
    }
}

#[derive(Debug, Default)]
pub struct InputState {
    held: HashSet<Button>,
    pressed: HashSet<Button>,
    released: HashSet<Button>,
    bindings: Bindings,
    gamepad_axes: HashMap<GamepadAxis, f32>,
    actions: HashMap<String, ActionState>,
    pub cursor_physical: Option<(f64, f64)>,
    pub relative_motion: (f64, f64),
    pub scroll_lines: (f64, f64),
    pub scroll_pixels: (f64, f64),
    /// Text from the backend, separate from physical keyboard bindings.
    pub text: String,
    /// Uncommitted IME composition text.
    pub preedit: String,
    /// Cursor position within `preedit`, in character indices.
    pub preedit_cursor: usize,
}
impl InputState {
    /// Replaces all bindings after validation. Held devices initialize action
    /// values without generating spurious presses; pending edges are discarded.
    pub fn set_bindings(&mut self, bindings: Bindings) -> Result<(), BindingError> {
        bindings.validate()?;
        self.actions = bindings
            .values(&self.held, &self.gamepad_axes)
            .map(|(name, value)| {
                (
                    name.to_owned(),
                    ActionState {
                        value,
                        ..Default::default()
                    },
                )
            })
            .collect();
        // A rebind is a configuration boundary: stale raw edges from the old
        // binding set must not surface through raw button queries later.
        self.pressed.clear();
        self.released.clear();
        self.bindings = bindings;
        Ok(())
    }
    pub fn bindings(&self) -> &Bindings {
        &self.bindings
    }
    /// Unknown/disabled actions return neutral state.
    pub fn action(&self, name: &str) -> ActionState {
        self.actions.get(name).copied().unwrap_or_default()
    }
    /// Consume a single action's press for immediate UI/desktop commands, without
    /// clearing simulation actions or the raw device edges.
    pub fn take_pressed(&mut self, name: &str) -> bool {
        self.actions
            .get_mut(name)
            .is_some_and(|state| std::mem::take(&mut state.pressed))
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
    pub fn key(&mut self, key: Key, down: bool) {
        self.button(Button::Key(key), down);
    }
    pub fn mouse_button(&mut self, button: MouseButton, down: bool) {
        self.button(Button::Mouse(button), down);
    }
    pub fn gamepad_button(&mut self, button: GamepadButton, down: bool) {
        self.button(Button::Gamepad(button), down);
    }
    /// Updates one processed analog value. Sticks are normally fed as a pair with
    /// [`Self::gamepad_axes`] so a diagonal transition produces one action update.
    pub fn gamepad_axis(&mut self, axis: GamepadAxis, value: f32) {
        self.gamepad_axes([(axis, value)]);
    }
    /// Atomically updates several processed analog values and then evaluates actions.
    pub fn gamepad_axes(&mut self, values: impl IntoIterator<Item = (GamepadAxis, f32)>) {
        let mut changed = false;
        for (axis, value) in values {
            let value = if value.is_finite() {
                value.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            if self
                .gamepad_axes
                .get(&axis)
                .is_none_or(|current| (*current - value).abs() > f32::EPSILON)
            {
                changed = true;
                if value == 0.0 {
                    self.gamepad_axes.remove(&axis);
                } else {
                    self.gamepad_axes.insert(axis, value);
                }
            }
        }
        if changed {
            self.update_actions();
        }
    }
    pub fn gamepad_axis_value(&self, axis: GamepadAxis) -> f32 {
        self.gamepad_axes.get(&axis).copied().unwrap_or(0.0)
    }
    /// Releases every gamepad button/axis in this state while leaving keyboard and
    /// mouse state intact. Used when a controller disconnects or is reassigned.
    pub fn release_gamepad(&mut self) {
        let buttons: Vec<Button> = self
            .held
            .iter()
            .copied()
            .filter(|button| matches!(button, Button::Gamepad(_)))
            .collect();
        for button in buttons {
            self.held.remove(&button);
            self.released.insert(button);
        }
        self.gamepad_axes.clear();
        self.update_actions();
    }
    /// Repeated down/up events do not create new edges. Actions are evaluated on
    /// every actual change so chords, alternate bindings, and short taps retain
    /// their true transitions until the consumer advances.
    pub fn button(&mut self, button: Button, down: bool) {
        let changed = if down {
            if self.held.insert(button) {
                self.pressed.insert(button);
                true
            } else {
                false
            }
        } else if self.held.remove(&button) {
            self.released.insert(button);
            true
        } else {
            false
        };
        if changed {
            self.update_actions();
        }
    }
    fn update_actions(&mut self) {
        for (name, value) in self.bindings.values(&self.held, &self.gamepad_axes) {
            let state = self
                .actions
                .get_mut(name)
                .expect("bindings and action states are installed together");
            state.pressed |= state.value == 0.0 && value != 0.0;
            state.released |= state.value != 0.0 && value == 0.0;
            state.value = value;
        }
    }
    /// Cancels pending presses and clears devices on focus loss. Releases remain
    /// observable, while stale commands cannot fire after focus returns.
    pub fn release_all(&mut self) {
        self.released.extend(self.held.drain());
        self.gamepad_axes.clear();
        self.pressed.clear();
        self.update_actions();
        for state in self.actions.values_mut() {
            state.pressed = false;
        }
        self.cursor_physical = None;
        self.preedit.clear();
        self.preedit_cursor = 0;
        self.clear_motion_and_text();
    }
    /// Consume once per simulation tick/UI update, retaining held values.
    pub fn consume_edges(&mut self) {
        self.pressed.clear();
        self.released.clear();
        for state in self.actions.values_mut() {
            state.pressed = false;
            state.released = false;
        }
        self.clear_motion_and_text();
    }
    fn clear_motion_and_text(&mut self) {
        self.relative_motion = (0.0, 0.0);
        self.scroll_lines = (0.0, 0.0);
        self.scroll_pixels = (0.0, 0.0);
        self.text.clear();
    }
}

#[cfg(test)]
mod tests;
