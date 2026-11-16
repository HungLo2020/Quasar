//! Active-device tracking and human-readable binding names for prompts.
//!
//! [`InputState::last_active_device`](crate::InputState::last_active_device)
//! reports whether the player last used the keyboard/mouse or a gamepad, so
//! prompts can switch between "Press E" and "Press Ⓐ" style hints.
//! [`Binding::display_name`] names a binding for a [`DeviceFamily`], and
//! [`Bindings::format_prompt`] replaces `{action}` placeholders in text.
//! Icons and fonts remain the game's choice: these are plain strings.
use crate::{
    Binding, Bindings, Button, GamepadAxis, GamepadButton, GamepadFingerprint, Key, MouseButton,
};
use serde::{Deserialize, Serialize};

/// The kind of device that produced the most recent meaningful input.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum InputDevice {
    KeyboardMouse,
    Gamepad,
}

/// Naming convention for prompts. Gamepad families differ in face-button and
/// shoulder names; `GenericGamepad` uses positional names.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
pub enum DeviceFamily {
    #[default]
    KeyboardMouse,
    Xbox,
    PlayStation,
    Nintendo,
    GenericGamepad,
}

impl DeviceFamily {
    /// Family from a USB vendor id (Microsoft, Sony, Nintendo); others are
    /// generic.
    pub const fn from_vendor_id(vendor_id: Option<u16>) -> Self {
        match vendor_id {
            Some(0x045E) => Self::Xbox,
            Some(0x054C) => Self::PlayStation,
            Some(0x057E) => Self::Nintendo,
            _ => Self::GenericGamepad,
        }
    }

    /// The device kind this family names.
    pub const fn device(self) -> InputDevice {
        match self {
            Self::KeyboardMouse => InputDevice::KeyboardMouse,
            _ => InputDevice::Gamepad,
        }
    }

    /// Family for prompts given the active device and, for gamepads, the
    /// connected controller's fingerprint.
    pub fn for_device(device: InputDevice, gamepad: Option<&GamepadFingerprint>) -> Self {
        match device {
            InputDevice::KeyboardMouse => Self::KeyboardMouse,
            InputDevice::Gamepad => {
                gamepad.map_or(Self::GenericGamepad, GamepadFingerprint::family)
            }
        }
    }
}

impl GamepadFingerprint {
    /// Prompt family inferred from the vendor id.
    pub fn family(&self) -> DeviceFamily {
        DeviceFamily::from_vendor_id(self.vendor_id)
    }
}

impl Button {
    /// The device kind this button belongs to.
    pub const fn device(self) -> InputDevice {
        match self {
            Self::Key(_) | Self::Mouse(_) => InputDevice::KeyboardMouse,
            Self::Gamepad(_) => InputDevice::Gamepad,
        }
    }

    /// Prompt name. Keyboard and mouse buttons ignore `family`.
    pub fn display_name(self, family: DeviceFamily) -> String {
        match self {
            Self::Key(key) => key.display_name().into_owned(),
            Self::Mouse(button) => button.display_name(),
            Self::Gamepad(button) => button.display_name(family).to_owned(),
        }
    }
}

impl MouseButton {
    pub fn display_name(self) -> String {
        match self {
            Self::Left => "Left Mouse".into(),
            Self::Right => "Right Mouse".into(),
            Self::Middle => "Middle Mouse".into(),
            Self::Back => "Mouse Back".into(),
            Self::Forward => "Mouse Forward".into(),
            Self::Other(code) => format!("Mouse {code}"),
        }
    }
}

impl Key {
    /// Short label for prompts ("W", "Space", "Left Shift"), as opposed to
    /// the canonical [`Key::name`] code.
    pub fn display_name(self) -> std::borrow::Cow<'static, str> {
        use std::borrow::Cow;
        let name = self.name();
        if let Some(letter) = name.strip_prefix("Key") {
            return Cow::Borrowed(letter);
        }
        if let Some(digit) = name.strip_prefix("Digit") {
            return Cow::Borrowed(digit);
        }
        if let Some(rest) = name.strip_prefix("Numpad") {
            return Cow::Owned(format!("Num {rest}"));
        }
        if let Some(direction) = name.strip_prefix("Arrow") {
            return Cow::Borrowed(direction);
        }
        for (suffix, side) in [("Left", "Left"), ("Right", "Right")] {
            if let Some(base) = name.strip_suffix(suffix) {
                let base = match base {
                    "Shift" => "Shift",
                    "Control" => "Ctrl",
                    "Alt" => "Alt",
                    "Super" | "Meta" => "Super",
                    _ => continue,
                };
                return Cow::Owned(format!("{side} {base}"));
            }
        }
        Cow::Borrowed(match name {
            "Escape" => "Esc",
            "Backquote" => "`",
            "Backslash" => "\\",
            "BracketLeft" => "[",
            "BracketRight" => "]",
            "Comma" => ",",
            "Period" => ".",
            "Slash" => "/",
            "Semicolon" => ";",
            "Quote" => "'",
            "Minus" => "-",
            "Equal" => "=",
            "Enter" => "Enter",
            "PageUp" => "Page Up",
            "PageDown" => "Page Down",
            "CapsLock" => "Caps Lock",
            "PrintScreen" => "Print Screen",
            other => other,
        })
    }
}

impl GamepadButton {
    /// Prompt name in the family's convention.
    pub fn display_name(self, family: DeviceFamily) -> &'static str {
        use DeviceFamily::*;
        use GamepadButton::*;
        match (self, family) {
            (South, Xbox) => "A",
            (East, Xbox) => "B",
            (West, Xbox) => "X",
            (North, Xbox) => "Y",
            (South, PlayStation) => "Cross",
            (East, PlayStation) => "Circle",
            (West, PlayStation) => "Square",
            (North, PlayStation) => "Triangle",
            (South, Nintendo) => "B",
            (East, Nintendo) => "A",
            (West, Nintendo) => "Y",
            (North, Nintendo) => "X",
            (South, _) => "South",
            (East, _) => "East",
            (West, _) => "West",
            (North, _) => "North",
            (LeftBumper, Xbox) => "LB",
            (RightBumper, Xbox) => "RB",
            (LeftBumper, PlayStation) => "L1",
            (RightBumper, PlayStation) => "R1",
            (LeftBumper, Nintendo) => "L",
            (RightBumper, Nintendo) => "R",
            (LeftBumper, _) => "Left Bumper",
            (RightBumper, _) => "Right Bumper",
            (LeftTrigger, Xbox) => "LT",
            (RightTrigger, Xbox) => "RT",
            (LeftTrigger, PlayStation) => "L2",
            (RightTrigger, PlayStation) => "R2",
            (LeftTrigger, Nintendo) => "ZL",
            (RightTrigger, Nintendo) => "ZR",
            (LeftTrigger, _) => "Left Trigger",
            (RightTrigger, _) => "Right Trigger",
            (Select, Xbox) => "View",
            (Start, Xbox) => "Menu",
            (Select, PlayStation) => "Create",
            (Start, PlayStation) => "Options",
            (Select, Nintendo) => "Minus",
            (Start, Nintendo) => "Plus",
            (Select, _) => "Select",
            (Start, _) => "Start",
            (Mode, Xbox) => "Xbox",
            (Mode, PlayStation) => "PS",
            (Mode, Nintendo) => "Home",
            (Mode, _) => "Mode",
            (LeftStick, PlayStation) => "L3",
            (RightStick, PlayStation) => "R3",
            (LeftStick, _) => "LS",
            (RightStick, _) => "RS",
            (DPadUp, _) => "D-Pad Up",
            (DPadDown, _) => "D-Pad Down",
            (DPadLeft, _) => "D-Pad Left",
            (DPadRight, _) => "D-Pad Right",
            (C, _) => "C",
            (Z, _) => "Z",
            (Unknown, _) => "Unknown",
        }
    }
}

impl GamepadAxis {
    pub fn display_name(self, family: DeviceFamily) -> &'static str {
        match self {
            Self::LeftStickX => "Left Stick X",
            Self::LeftStickY => "Left Stick Y",
            Self::RightStickX => "Right Stick X",
            Self::RightStickY => "Right Stick Y",
            Self::LeftTrigger => GamepadButton::LeftTrigger.display_name(family),
            Self::RightTrigger => GamepadButton::RightTrigger.display_name(family),
        }
    }
}

impl Binding {
    /// Device kind the binding reads, or `None` for a chord that mixes
    /// devices.
    pub fn device(&self) -> Option<InputDevice> {
        match self {
            Self::Button { button } => Some(button.device()),
            Self::Axis { negative, positive } => {
                (negative.device() == positive.device()).then_some(negative.device())
            }
            Self::Chord { buttons } => {
                let first = buttons.first()?.device();
                buttons
                    .iter()
                    .all(|button| button.device() == first)
                    .then_some(first)
            }
            Self::GamepadAxis { .. } => Some(InputDevice::Gamepad),
        }
    }

    /// Prompt text: `"E"`, `"A / D"`, `"Ctrl + S"`, `"Left Stick X"`.
    pub fn display_name(&self, family: DeviceFamily) -> String {
        match self {
            Self::Button { button } => button.display_name(family),
            Self::Axis { negative, positive } => format!(
                "{} / {}",
                negative.display_name(family),
                positive.display_name(family)
            ),
            Self::Chord { buttons } => buttons
                .iter()
                .map(|button| button.display_name(family))
                .collect::<Vec<_>>()
                .join(" + "),
            Self::GamepadAxis { axis, .. } => axis.display_name(family).to_owned(),
        }
    }
}

impl Bindings {
    /// Names of every binding of `action` that belongs to `family`'s device,
    /// in binding order. Empty for unknown actions.
    pub fn display_names(&self, action: &str, family: DeviceFamily) -> Vec<String> {
        self.actions
            .get(action)
            .into_iter()
            .flatten()
            .filter(|binding| binding.device() == Some(family.device()))
            .map(|binding| binding.display_name(family))
            .collect()
    }

    /// The first binding name of `action` for `family`, if any.
    pub fn display_name(&self, action: &str, family: DeviceFamily) -> Option<String> {
        self.display_names(action, family).into_iter().next()
    }

    /// Replaces each `{action}` in `template` with that action's first
    /// binding name for `family`. Unknown or unbound actions stay as
    /// `{action}` so missing bindings remain visible; `{{` and `}}` escape
    /// braces.
    pub fn format_prompt(&self, template: &str, family: DeviceFamily) -> String {
        let mut out = String::with_capacity(template.len());
        let mut rest = template;
        while let Some(start) = rest.find(['{', '}']) {
            out.push_str(&rest[..start]);
            let tail = &rest[start..];
            if tail.starts_with("{{") || tail.starts_with("}}") {
                out.push_str(&tail[..1]);
                rest = &tail[2..];
                continue;
            }
            if let Some(after) = tail.strip_prefix('}') {
                out.push('}');
                rest = after;
                continue;
            }
            match tail[1..].find('}') {
                Some(end) => {
                    let action = &tail[1..1 + end];
                    match self.display_name(action, family) {
                        Some(name) => out.push_str(&name),
                        None => out.push_str(&tail[..end + 2]),
                    }
                    rest = &tail[end + 2..];
                }
                None => {
                    out.push_str(tail);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    }
}
