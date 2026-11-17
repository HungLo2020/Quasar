use crate::{Button, GamepadAxis, GamepadButton, Key};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    error::Error,
    fmt,
};

/// One source for a named action. Multiple sources are summed and clamped to \[-1,1\].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Binding {
    /// A digital button contributes +1 while held.
    Button { button: Button },
    /// Two opposing buttons contribute -1/0/+1; simultaneous presses cancel.
    Axis { negative: Button, positive: Button },
    /// All buttons must be held; pressing the last activates the action.
    Chord { buttons: Vec<Button> },
    /// A processed analog axis, multiplied by `scale` (use -1 to invert).
    /// Sticks are in \[-1,1\] and triggers are in \[0,1\] after configuration.
    GamepadAxis { axis: GamepadAxis, scale: f32 },
}
impl Binding {
    pub fn key(key: Key) -> Self {
        Self::Button {
            button: Button::Key(key),
        }
    }
    pub fn axis(negative: Key, positive: Key) -> Self {
        Self::Axis {
            negative: Button::Key(negative),
            positive: Button::Key(positive),
        }
    }
    pub fn chord(keys: impl IntoIterator<Item = Key>) -> Self {
        Self::Chord {
            buttons: keys.into_iter().map(Button::Key).collect(),
        }
    }
    pub fn gamepad_button(button: GamepadButton) -> Self {
        Self::Button {
            button: Button::Gamepad(button),
        }
    }
    /// Two gamepad buttons as one axis, e.g. D-pad left/right for movement.
    pub fn gamepad_button_axis(negative: GamepadButton, positive: GamepadButton) -> Self {
        Self::Axis {
            negative: Button::Gamepad(negative),
            positive: Button::Gamepad(positive),
        }
    }
    /// Two arbitrary buttons (keys, mouse or gamepad) as one axis.
    pub fn button_axis(negative: Button, positive: Button) -> Self {
        Self::Axis { negative, positive }
    }
    pub fn gamepad_axis(axis: GamepadAxis) -> Self {
        Self::GamepadAxis { axis, scale: 1.0 }
    }
    pub fn gamepad_axis_scaled(axis: GamepadAxis, scale: f32) -> Self {
        Self::GamepadAxis { axis, scale }
    }
    pub(crate) fn value(&self, held: &HashSet<Button>, axes: &HashMap<GamepadAxis, f32>) -> f32 {
        match self {
            Self::Button { button } => u8::from(held.contains(button)) as f32,
            Self::Axis { negative, positive } => {
                i8::from(held.contains(positive)) as f32 - i8::from(held.contains(negative)) as f32
            }
            Self::Chord { buttons } => {
                u8::from(buttons.iter().all(|button| held.contains(button))) as f32
            }
            Self::GamepadAxis { axis, scale } => axes.get(axis).copied().unwrap_or(0.0) * scale,
        }
    }
}

/// Serializable named actions. An empty list deliberately disables an action.
/// This is configuration only: no OS, file I/O, or game-specific actions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bindings {
    pub actions: BTreeMap<String, Vec<Binding>>,
}
impl Bindings {
    pub fn bind(&mut self, action: impl Into<String>, binding: Binding) -> &mut Self {
        self.actions.entry(action.into()).or_default().push(binding);
        self
    }
    pub fn validate(&self) -> Result<(), BindingError> {
        for (action, bindings) in &self.actions {
            if action.trim().is_empty() {
                return Err(BindingError("action names must not be empty".into()));
            }
            for binding in bindings {
                let invalid = match binding {
                    Binding::Axis { negative, positive } => negative == positive,
                    Binding::Chord { buttons } => {
                        buttons.is_empty()
                            || buttons.iter().collect::<HashSet<_>>().len() != buttons.len()
                    }
                    Binding::GamepadAxis { scale, .. } => !scale.is_finite(),
                    Binding::Button { .. } => false,
                };
                if invalid {
                    return Err(BindingError(format!(
                        "action '{action}' has an empty/duplicate chord, identical axis buttons, or a non-finite gamepad scale"
                    )));
                }
            }
        }
        Ok(())
    }
    pub(crate) fn values<'a>(
        &'a self,
        held: &'a HashSet<Button>,
        axes: &'a HashMap<GamepadAxis, f32>,
    ) -> impl Iterator<Item = (&'a str, f32)> + 'a {
        self.actions.iter().map(move |(name, bindings)| {
            let value = bindings
                .iter()
                .map(|b| b.value(held, axes))
                .sum::<f32>()
                .clamp(-1.0, 1.0);
            (name.as_str(), value)
        })
    }
}

/// One physical button used by more than one action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingConflict {
    pub button: Button,
    /// Actions that read `button`, sorted.
    pub actions: Vec<String>,
}

impl Binding {
    /// Physical buttons this binding reads (analog axes read none).
    pub fn buttons(&self) -> Vec<Button> {
        match self {
            Self::Button { button } => vec![*button],
            Self::Axis { negative, positive } => vec![*negative, *positive],
            Self::Chord { buttons } => buttons.clone(),
            Self::GamepadAxis { .. } => Vec::new(),
        }
    }
}

impl Bindings {
    /// Buttons bound to more than one of `actions` (for example the actions
    /// of one controls screen; UI and gameplay actions may share buttons
    /// on purpose, so only the listed actions are compared). Chord members
    /// count: a chord sharing a button with a plain binding conflicts.
    pub fn conflicts_among(&self, actions: &[&str]) -> Vec<BindingConflict> {
        let mut users: BTreeMap<String, (Button, Vec<String>)> = BTreeMap::new();
        for action in actions {
            for binding in self.actions.get(*action).into_iter().flatten() {
                for button in binding.buttons() {
                    let entry = users
                        .entry(format!("{button:?}"))
                        .or_insert_with(|| (button, Vec::new()));
                    if !entry.1.iter().any(|a| a == action) {
                        entry.1.push((*action).to_owned());
                    }
                }
            }
        }
        users
            .into_values()
            .filter(|(_, actions)| actions.len() > 1)
            .map(|(button, mut actions)| {
                actions.sort();
                BindingConflict { button, actions }
            })
            .collect()
    }

    /// Saved bindings over `defaults`: actions present here keep their
    /// (possibly empty) bindings; actions only in `defaults` (added by a
    /// game update) get their defaults; actions no longer in `defaults`
    /// are dropped.
    pub fn merged_with_defaults(&self, defaults: &Bindings) -> Bindings {
        Bindings {
            actions: defaults
                .actions
                .iter()
                .map(|(action, default)| {
                    let chosen = self.actions.get(action).unwrap_or(default);
                    (action.clone(), chosen.clone())
                })
                .collect(),
        }
    }

    /// Replaces `action`'s `slot`-th binding read by `device` (appending
    /// when there is none) with `binding`. Returns the replaced binding.
    pub fn rebind(
        &mut self,
        action: &str,
        device: crate::InputDevice,
        slot: usize,
        binding: Binding,
    ) -> Option<Binding> {
        let list = self.actions.entry(action.to_owned()).or_default();
        let index = list
            .iter()
            .enumerate()
            .filter(|(_, b)| b.device() == Some(device))
            .map(|(i, _)| i)
            .nth(slot);
        match index {
            Some(i) => Some(std::mem::replace(&mut list[i], binding)),
            None => {
                list.push(binding);
                None
            }
        }
    }

    /// Removes every binding of `button` from the listed actions except
    /// `keep` (resolving a conflict in favour of `keep`).
    pub fn unbind_button_except(&mut self, button: Button, actions: &[&str], keep: &str) {
        for action in actions {
            if *action == keep {
                continue;
            }
            if let Some(list) = self.actions.get_mut(*action) {
                list.retain(|b| !b.buttons().contains(&button));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingError(pub String);
impl fmt::Display for BindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl Error for BindingError {}
