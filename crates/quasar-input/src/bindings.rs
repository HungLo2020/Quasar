use crate::{Button, Key};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    error::Error,
    fmt,
};

/// One source for a named action. Multiple sources are summed and clamped to [-1,1].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Binding {
    /// A digital button contributes +1 while held.
    Button { button: Button },
    /// Two opposing buttons contribute -1/0/+1; simultaneous presses cancel.
    Axis { negative: Button, positive: Button },
    /// All buttons must be held; pressing the last activates the action.
    Chord { buttons: Vec<Button> },
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
    pub(crate) fn value(&self, held: &HashSet<Button>) -> f32 {
        match self {
            Self::Button { button } => u8::from(held.contains(button)) as f32,
            Self::Axis { negative, positive } => {
                i8::from(held.contains(positive)) as f32 - i8::from(held.contains(negative)) as f32
            }
            Self::Chord { buttons } => {
                u8::from(buttons.iter().all(|button| held.contains(button))) as f32
            }
        }
    }
}

/// Serializable named actions. An empty list deliberately disables an action.
/// This is configuration only: no OS, file I/O, or game-specific actions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
                    Binding::Button { .. } => false,
                };
                if invalid {
                    return Err(BindingError(format!(
                        "action '{action}' has an empty/duplicate chord or identical axis buttons"
                    )));
                }
            }
        }
        Ok(())
    }
    pub(crate) fn values(&self, held: &HashSet<Button>) -> impl Iterator<Item = (&str, f32)> {
        self.actions.iter().map(|(name, bindings)| {
            let value = bindings
                .iter()
                .map(|b| b.value(held))
                .sum::<f32>()
                .clamp(-1.0, 1.0);
            (name.as_str(), value)
        })
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
