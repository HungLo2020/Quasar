//! Pong owns action names and defaults; Quasar owns device state and mapping.
use crate::Result;
#[cfg(any(feature = "desktop", test))]
use quasar_input::InputRead;
use quasar_input::{Binding, Bindings, Key};
#[cfg(feature = "desktop")]
use quasar_input::{Button, GamepadButton, MouseButton};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const LEFT: &str = "left_paddle";
pub const RIGHT: &str = "right_paddle";
pub const SERVE: &str = "serve";
pub const PAUSE: &str = "pause";
pub const RESTART: &str = "restart";
pub const QUIT: &str = "quit";
pub const FULLSCREEN: &str = "fullscreen";
pub const FILTER: &str = "filter";
pub const SCALE: &str = "scale";
const ACTIONS: [&str; 9] = [
    LEFT, RIGHT, SERVE, PAUSE, RESTART, QUIT, FULLSCREEN, FILTER, SCALE,
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Controls {
    pub version: u32,
    pub bindings: Bindings,
}
impl Default for Controls {
    fn default() -> Self {
        let mut bindings = Bindings::default();
        bindings
            .bind(LEFT, Binding::axis(Key::KeyW, Key::KeyS))
            .bind(RIGHT, Binding::axis(Key::ArrowUp, Key::ArrowDown))
            .bind(SERVE, Binding::key(Key::Space))
            .bind(PAUSE, Binding::key(Key::KeyP))
            .bind(PAUSE, Binding::key(Key::Escape))
            .bind(RESTART, Binding::key(Key::KeyR))
            .bind(QUIT, Binding::key(Key::KeyQ))
            .bind(FULLSCREEN, Binding::key(Key::F11))
            .bind(FULLSCREEN, Binding::chord([Key::AltLeft, Key::Enter]))
            .bind(FULLSCREEN, Binding::chord([Key::AltRight, Key::Enter]))
            .bind(FILTER, Binding::key(Key::F2))
            .bind(SCALE, Binding::key(Key::F3));
        Self {
            version: 1,
            bindings,
        }
    }
}
impl Controls {
    pub fn validate(&self) -> Result {
        if self.version != 1 {
            return Err("unsupported controls version".into());
        }
        self.bindings.validate()?;
        for action in ACTIONS {
            if !self.bindings.actions.contains_key(action) {
                return Err(format!(
                    "missing action '{action}'; use an empty list to disable it explicitly"
                )
                .into());
            }
        }
        for action in self.bindings.actions.keys() {
            if !ACTIONS.contains(&action.as_str()) {
                return Err(format!("unknown Pong action '{action}'").into());
            }
        }
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        let controls: Self =
            quasar_storage::read_json(path)?.ok_or("controls file does not exist")?;
        controls.validate()?;
        Ok(controls)
    }
}

#[cfg(any(feature = "desktop", test))]
pub fn game_input(input: &impl InputRead) -> pong::Input {
    pong::Input {
        left: input.action(LEFT).value as i8,
        right: input.action(RIGHT).value as i8,
        serve: input.action(SERVE).pressed,
        pause: input.action(PAUSE).pressed,
        restart: input.action(RESTART).pressed,
    }
}

#[cfg(feature = "desktop")]
impl Controls {
    pub fn hints(&self) -> pong::render::ControlHints {
        let label = |name| {
            self.bindings
                .actions
                .get(name)
                .and_then(|bindings| bindings.first())
                .map(binding_label)
                .unwrap_or("UNBOUND".into())
        };
        pong::render::ControlHints {
            left: label(LEFT),
            right: label(RIGHT),
            serve: label(SERVE),
            pause: label(PAUSE),
            restart: label(RESTART),
            fullscreen: label(FULLSCREEN),
            filter: label(FILTER),
            scale: label(SCALE),
        }
    }
}
#[cfg(feature = "desktop")]
fn binding_label(binding: &Binding) -> String {
    match binding {
        Binding::Button { button } => button_label(*button),
        Binding::Axis { negative, positive } => {
            format!("{}/{}", button_label(*negative), button_label(*positive))
        }
        Binding::Chord { buttons } => buttons
            .iter()
            .map(|button| button_label(*button))
            .collect::<Vec<_>>()
            .join("+"),
        Binding::GamepadAxis { axis, scale } => {
            let label = format!("{axis:?}")
                .to_uppercase()
                .replace("STICK", "STICK ");
            if (*scale - 1.0).abs() < f32::EPSILON {
                label
            } else {
                format!("{label}*{scale}")
            }
        }
    }
}
#[cfg(feature = "desktop")]
fn button_label(button: Button) -> String {
    match button {
        Button::Key(key) => match key {
            Key::ArrowUp => "UP".into(),
            Key::ArrowDown => "DOWN".into(),
            Key::ArrowLeft => "LEFT".into(),
            Key::ArrowRight => "RIGHT".into(),
            Key::AltLeft => "LALT".into(),
            Key::AltRight => "RALT".into(),
            Key::ControlLeft => "LCTRL".into(),
            Key::ControlRight => "RCTRL".into(),
            other => {
                let label = format!("{other:?}");
                label
                    .strip_prefix("Key")
                    .or_else(|| label.strip_prefix("Digit"))
                    .unwrap_or(&label)
                    .to_uppercase()
            }
        },
        Button::Mouse(MouseButton::Left) => "MOUSE1".into(),
        Button::Mouse(MouseButton::Right) => "MOUSE2".into(),
        Button::Mouse(MouseButton::Middle) => "MOUSE3".into(),
        Button::Mouse(other) => format!("MOUSE {other:?}").to_uppercase(),
        Button::Gamepad(button) => gamepad_button_label(button),
    }
}

#[cfg(feature = "desktop")]
fn gamepad_button_label(button: GamepadButton) -> String {
    match button {
        GamepadButton::South => "PAD SOUTH".into(),
        GamepadButton::East => "PAD EAST".into(),
        GamepadButton::North => "PAD NORTH".into(),
        GamepadButton::West => "PAD WEST".into(),
        GamepadButton::LeftBumper => "LB".into(),
        GamepadButton::RightBumper => "RB".into(),
        GamepadButton::LeftTrigger => "LT".into(),
        GamepadButton::RightTrigger => "RT".into(),
        GamepadButton::LeftStick => "LS".into(),
        GamepadButton::RightStick => "RS".into(),
        GamepadButton::DPadUp => "DPAD UP".into(),
        GamepadButton::DPadDown => "DPAD DOWN".into(),
        GamepadButton::DPadLeft => "DPAD LEFT".into(),
        GamepadButton::DPadRight => "DPAD RIGHT".into(),
        GamepadButton::Select => "SELECT".into(),
        GamepadButton::Start => "START".into(),
        GamepadButton::Mode => "MODE".into(),
        GamepadButton::C => "C".into(),
        GamepadButton::Z => "Z".into(),
        GamepadButton::Unknown => "PAD ?".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quasar_input::InputState;
    #[test]
    fn custom_controls_drive_gameplay_without_native_events() {
        let mut controls = Controls::default();
        controls
            .bindings
            .actions
            .insert(LEFT.into(), vec![Binding::axis(Key::KeyI, Key::KeyK)]);
        controls
            .bindings
            .actions
            .insert(SERVE.into(), vec![Binding::key(Key::Enter)]);
        controls.validate().unwrap();
        let mut input = InputState::default();
        input.set_bindings(controls.bindings).unwrap();
        let mut game = pong::Game::new(pong::Mode::Local);
        input.key(Key::Space, true);
        game.tick(game_input(&input));
        input.consume_edges();
        assert_eq!(game.phase, pong::Phase::Ready);
        input.key(Key::Enter, true);
        game.tick(game_input(&input));
        input.consume_edges();
        assert_eq!(game.phase, pong::Phase::Playing);
        let previous = game.paddles[0];
        input.key(Key::KeyI, true);
        game.tick(game_input(&input));
        assert!(game.paddles[0] < previous);
        input.release_all();
        assert_eq!(game_input(&input).left, 0);
        assert!(!game_input(&input).serve);
    }
    #[test]
    fn unknown_missing_and_unsupported_controls_are_rejected() {
        let mut controls = Controls {
            version: 2,
            ..Default::default()
        };
        assert!(controls.validate().is_err());
        controls.version = 1;
        controls.bindings.actions.remove(SERVE);
        assert!(controls.validate().is_err());
        controls.bindings.actions.insert(SERVE.into(), vec![]);
        assert!(controls.validate().is_ok());
        controls.bindings.actions.insert("typo".into(), vec![]);
        assert!(controls.validate().is_err());
    }
}
