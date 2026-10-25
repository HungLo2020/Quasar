use quasar_input::{Binding, Bindings, InputRead, Key};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Controls {
    pub version: u32,
    pub bindings: Bindings,
}
impl Default for Controls {
    fn default() -> Self {
        let mut b = Bindings::default();
        b.bind("move_x", Binding::axis(Key::KeyA, Key::KeyD))
            .bind("move_x", Binding::axis(Key::ArrowLeft, Key::ArrowRight))
            .bind("move_y", Binding::axis(Key::KeyW, Key::KeyS))
            .bind("move_y", Binding::axis(Key::ArrowUp, Key::ArrowDown))
            .bind("pause", Binding::key(Key::KeyP))
            .bind("pause", Binding::key(Key::Escape))
            .bind("restart", Binding::key(Key::KeyR))
            .bind("quit", Binding::key(Key::KeyQ))
            .bind("fullscreen", Binding::key(Key::F11))
            .bind("filter", Binding::key(Key::F2))
            .bind("scale", Binding::key(Key::F3));
        Self {
            version: 1,
            bindings: b,
        }
    }
}
impl Controls {
    pub fn validate(&self) -> crate::Result {
        self.bindings.validate()?;
        let names = [
            "move_x",
            "move_y",
            "pause",
            "restart",
            "quit",
            "fullscreen",
            "filter",
            "scale",
        ];
        if self.version != 1
            || names
                .iter()
                .any(|n| !self.bindings.actions.contains_key(*n))
            || self
                .bindings
                .actions
                .keys()
                .any(|n| !names.contains(&n.as_str()))
        {
            return Err("invalid Tilequest controls version/actions".into());
        }
        Ok(())
    }
    pub fn input(&self, input: &impl InputRead) -> crate::Input {
        crate::Input {
            x: input.action("move_x").value.round() as i8,
            y: input.action("move_y").value.round() as i8,
            pause: input.action("pause").pressed,
            restart: input.action("restart").pressed,
        }
    }
}
