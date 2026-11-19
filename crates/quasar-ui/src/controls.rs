//! A ready controls screen: lists actions, captures rebinds, flags conflicts
//! and resets to defaults.
//!
//! [`ControlsScreen::build`] adds one [`Widget::Binding`] row per listed
//! action (for the keyboard/mouse or the gamepad column), a conflict notice
//! and a reset button under a parent panel. Feed it every [`UiEvent`]
//! ([`ControlsScreen::handle_event`]) and, while a row is capturing, the
//! buttons pressed this frame ([`ControlsScreen::capture`]). Buttons bound to
//! two listed actions are reported by [`ControlsScreen::conflicts`] and the
//! affected rows get the theme's danger border. Escape (or the gamepad's
//! Select) cancels a capture.
//!
//! Axis bindings (`A / D`) capture two presses: negative, then positive.
//! Analogue rows (a [`Binding::GamepadAxis`] such as the left stick) ignore
//! buttons and capture a stick or trigger deflection instead: feed
//! [`ControlsScreen::capture_axes`] the gamepad's processed axes (for
//! example `GamepadAxis::ALL.map(|a| (a, state.processed_axis(a)))`). The
//! player pushes the control in its positive direction; pushing the other
//! way binds it inverted. The reset button's layout is set with
//! [`ControlsScreen::with_reset_item`] and its node is
//! [`ControlsScreen::reset_node`], for styling it like surrounding menus.
//! With the `controls-file` feature, [`save_bindings`] and [`load_bindings`]
//! persist the result as TOML or JSON, merging saved files over the current
//! defaults so new actions added by an update get their default bindings.
use crate::{
    ActionId, Border, LabelWrap, LayoutItem, Length, NodeId, NodeStyle, PanelLayout,
    StackDirection, TextStyle, Ui, UiEvent, UiResult, UiText, Widget,
};
use quasar_input::{
    Binding, BindingConflict, Bindings, Button, DeviceFamily, GamepadAxis, GamepadButton,
    InputDevice, Key,
};

/// Deflection that counts as an analogue capture.
pub const AXIS_CAPTURE_THRESHOLD: f32 = 0.6;
/// Every axis must first rest below this before a capture can trigger, so a
/// stick still held from menu navigation is not captured.
const AXIS_REST: f32 = 0.3;

/// One listed action.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlEntry {
    pub action: String,
    pub label: UiText,
}

impl ControlEntry {
    pub fn new(action: impl Into<String>, label: UiText) -> Self {
        Self {
            action: action.into(),
            label,
        }
    }
}

/// What a controls-screen event changed.
#[derive(Clone, Debug, PartialEq)]
pub enum ControlsOutcome {
    None,
    /// A row started capturing.
    Capturing {
        action: String,
    },
    /// Waiting for the second (positive) button of an axis binding.
    CapturingSecond {
        action: String,
    },
    Rebound {
        action: String,
        binding: Binding,
    },
    Cancelled,
    Reset,
}

#[derive(Clone, Debug)]
struct Capture {
    row: usize,
    first: Option<Button>,
    /// For analogue rows: axes have returned to rest since the capture began.
    armed: bool,
}

/// The kind of binding a row edits on the current device.
#[derive(Clone, Copy, Debug, PartialEq)]
enum RowKind {
    Button,
    ButtonAxis,
    Analog { scale: f32 },
}

pub struct ControlsScreen {
    entries: Vec<ControlEntry>,
    bindings: Bindings,
    defaults: Bindings,
    device: InputDevice,
    family: DeviceFamily,
    rows: Vec<NodeId>,
    notice: Option<NodeId>,
    reset: Option<ActionId>,
    reset_item: LayoutItem,
    reset_node: Option<NodeId>,
    capture: Option<Capture>,
}

impl ControlsScreen {
    /// `bindings` are the player's current bindings; missing actions take
    /// their `defaults`.
    pub fn new(entries: Vec<ControlEntry>, bindings: &Bindings, defaults: Bindings) -> Self {
        Self {
            entries,
            bindings: bindings.merged_with_defaults(&defaults),
            defaults,
            device: InputDevice::KeyboardMouse,
            family: DeviceFamily::KeyboardMouse,
            rows: Vec::new(),
            notice: None,
            reset: None,
            reset_item: LayoutItem::auto(),
            reset_node: None,
            capture: None,
        }
    }

    /// Lays out the reset button with `item` (for example
    /// `LayoutItem::size(Length::Fill, Length::fixed(40.0))` to match
    /// full-width menu buttons). Call before [`Self::build`].
    pub fn with_reset_item(mut self, item: LayoutItem) -> Self {
        self.reset_item = item;
        self
    }
    /// The reset button's node after [`Self::build`], for
    /// [`Ui::set_style`] or [`Ui::set_colors`].
    pub fn reset_node(&self) -> Option<NodeId> {
        self.reset_node
    }
    /// Row nodes in entry order after [`Self::build`].
    pub fn row_nodes(&self) -> &[NodeId] {
        &self.rows
    }

    pub fn bindings(&self) -> &Bindings {
        &self.bindings
    }
    pub fn device(&self) -> InputDevice {
        self.device
    }
    pub fn is_capturing(&self) -> bool {
        self.capture.is_some()
    }
    fn actions(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.action.as_str()).collect()
    }
    /// Buttons used by more than one listed action.
    pub fn conflicts(&self) -> Vec<BindingConflict> {
        self.bindings.conflicts_among(&self.actions())
    }

    /// Adds the screen under `parent` and returns its panel. `reset` is the
    /// action id of the reset button.
    pub fn build(&mut self, ui: &mut Ui, parent: NodeId, reset: ActionId) -> UiResult<NodeId> {
        let panel = ui.add_child(
            parent,
            Widget::Panel {
                layout: PanelLayout::Stack {
                    direction: StackDirection::Vertical,
                    gap: 4.,
                },
                padding: crate::Edges::ZERO,
                clip: false,
                scrollable: true,
            },
            LayoutItem::size(Length::Fill, Length::Auto),
        )?;
        self.rows.clear();
        for entry in &self.entries {
            let row = ui.add_child(
                panel,
                Widget::Binding {
                    label: entry.label.clone(),
                    action: entry.action.clone(),
                    value: String::new(),
                },
                LayoutItem::size(Length::Fill, Length::Auto),
            )?;
            self.rows.push(row);
        }
        self.notice = Some(ui.add_child(
            panel,
            Widget::Label {
                text: UiText::literal(""),
                style: TextStyle::Small,
                wrap: LabelWrap::Word,
                max_width: None,
            },
            LayoutItem::size(Length::Fill, Length::Auto),
        )?);
        self.reset_node = Some(ui.add_child(
            panel,
            Widget::Button {
                text: UiText::keyed("controls.reset", "Reset to defaults"),
                action: Some(reset),
            },
            self.reset_item,
        )?);
        self.reset = Some(reset);
        self.refresh(ui)?;
        Ok(panel)
    }

    /// Switches the edited column (keyboard/mouse or gamepad) and the names
    /// shown for it.
    pub fn set_device(
        &mut self,
        ui: &mut Ui,
        device: InputDevice,
        family: DeviceFamily,
    ) -> UiResult<()> {
        self.device = device;
        self.family = if device == InputDevice::KeyboardMouse {
            DeviceFamily::KeyboardMouse
        } else if family == DeviceFamily::KeyboardMouse {
            DeviceFamily::GenericGamepad
        } else {
            family
        };
        self.cancel(ui);
        self.refresh(ui)
    }

    fn display(&self, action: &str) -> String {
        let names: Vec<String> = self
            .bindings
            .actions
            .get(action)
            .into_iter()
            .flatten()
            .filter(|b| b.device() == Some(self.device))
            .map(|b| b.display_name(self.family))
            .collect();
        if names.is_empty() {
            "—".into()
        } else {
            names.join(", ")
        }
    }

    /// Rewrites every row value, conflict border and the notice.
    pub fn refresh(&self, ui: &mut Ui) -> UiResult<()> {
        let conflicts = self.conflicts();
        let danger = ui.theme().danger;
        for (entry, row) in self.entries.iter().zip(&self.rows) {
            let capturing = self
                .capture
                .as_ref()
                .is_some_and(|c| self.rows[c.row] == *row);
            let value = if capturing {
                ui.resolve_text(&UiText::keyed("controls.press", "Press a button…"))
                    .to_owned()
            } else {
                self.display(&entry.action)
            };
            ui.set_binding_value(*row, value)?;
            let conflicted = conflicts.iter().any(|c| c.actions.contains(&entry.action));
            ui.set_style(
                *row,
                conflicted.then_some(NodeStyle {
                    background: None,
                    border: Some(Border {
                        width: 1.,
                        color: danger,
                    }),
                    radius: 0.,
                }),
            )?;
        }
        if let Some(notice) = self.notice {
            let text = if conflicts.is_empty() {
                String::new()
            } else {
                let label = |action: &str| {
                    self.entries
                        .iter()
                        .find(|e| e.action == action)
                        .map_or(action.to_owned(), |e| ui.resolve_text(&e.label).to_owned())
                };
                conflicts
                    .iter()
                    .map(|c| {
                        format!(
                            "{}: {}",
                            c.button.display_name(self.family),
                            c.actions
                                .iter()
                                .map(|a| label(a))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            ui.set_text(notice, UiText::literal(text))?;
        }
        Ok(())
    }

    /// Reacts to a UI event: starts a capture when a row is activated and
    /// restores defaults when the reset button is pressed.
    pub fn handle_event(&mut self, ui: &mut Ui, event: &UiEvent) -> UiResult<ControlsOutcome> {
        match event {
            UiEvent::RebindRequested { id, .. } => {
                let Some(row) = self.rows.iter().position(|r| r == id) else {
                    return Ok(ControlsOutcome::None);
                };
                self.capture = Some(Capture {
                    row,
                    first: None,
                    armed: false,
                });
                self.refresh(ui)?;
                Ok(ControlsOutcome::Capturing {
                    action: self.entries[row].action.clone(),
                })
            }
            UiEvent::Pressed(action) if Some(*action) == self.reset => {
                self.bindings = self.defaults.clone();
                self.cancel(ui);
                self.refresh(ui)?;
                Ok(ControlsOutcome::Reset)
            }
            _ => Ok(ControlsOutcome::None),
        }
    }

    fn cancel(&mut self, ui: &mut Ui) {
        if self.capture.take().is_some() {
            ui.cancel_rebind();
        }
    }

    /// Offers this frame's pressed buttons to an active capture. Buttons of
    /// the other device are ignored; Escape or Select cancels.
    pub fn capture(&mut self, ui: &mut Ui, pressed: &[Button]) -> UiResult<ControlsOutcome> {
        let Some(capture) = self.capture.clone() else {
            return Ok(ControlsOutcome::None);
        };
        if pressed.iter().any(|b| {
            matches!(
                b,
                Button::Key(Key::Escape) | Button::Gamepad(GamepadButton::Select)
            )
        }) {
            self.cancel(ui);
            self.refresh(ui)?;
            return Ok(ControlsOutcome::Cancelled);
        }
        let Some(&button) = pressed.iter().find(|b| {
            b.device() == self.device && !matches!(b, Button::Gamepad(GamepadButton::Unknown))
        }) else {
            return Ok(ControlsOutcome::None);
        };
        let action = self.entries[capture.row].action.clone();
        let binding = match (self.row_kind(&action), capture.first) {
            // Analogue rows take a stick deflection (see `capture_axes`).
            (RowKind::Analog { .. }, _) => return Ok(ControlsOutcome::None),
            (RowKind::ButtonAxis, None) => {
                self.capture = Some(Capture {
                    first: Some(button),
                    ..capture
                });
                return Ok(ControlsOutcome::CapturingSecond { action });
            }
            (RowKind::ButtonAxis, Some(negative)) if negative != button => Binding::Axis {
                negative,
                positive: button,
            },
            (RowKind::ButtonAxis, Some(_)) => return Ok(ControlsOutcome::None),
            (RowKind::Button, _) => Binding::Button { button },
        };
        self.finish(ui, action, binding)
    }

    /// Offers this frame's processed gamepad axes (stick and trigger values)
    /// to an active capture of an analogue row. The strongest deflection
    /// past [`AXIS_CAPTURE_THRESHOLD`] is bound, inverted when pushed in the
    /// negative direction. Other rows ignore axes.
    pub fn capture_axes(
        &mut self,
        ui: &mut Ui,
        axes: &[(GamepadAxis, f32)],
    ) -> UiResult<ControlsOutcome> {
        let Some(capture) = self.capture.clone() else {
            return Ok(ControlsOutcome::None);
        };
        if self.device == InputDevice::KeyboardMouse {
            return Ok(ControlsOutcome::None);
        }
        let action = self.entries[capture.row].action.clone();
        let RowKind::Analog { scale } = self.row_kind(&action) else {
            return Ok(ControlsOutcome::None);
        };
        if !capture.armed {
            if axes
                .iter()
                .all(|(_, v)| !v.is_finite() || v.abs() < AXIS_REST)
            {
                self.capture = Some(Capture {
                    armed: true,
                    ..capture
                });
            }
            return Ok(ControlsOutcome::None);
        }
        let Some(&(axis, value)) = axes
            .iter()
            .filter(|(_, v)| v.is_finite() && v.abs() >= AXIS_CAPTURE_THRESHOLD)
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        else {
            return Ok(ControlsOutcome::None);
        };
        let magnitude = if scale.abs() > 0.0 { scale.abs() } else { 1.0 };
        let binding = Binding::GamepadAxis {
            axis,
            scale: magnitude.copysign(value),
        };
        self.finish(ui, action, binding)
    }

    fn finish(
        &mut self,
        ui: &mut Ui,
        action: String,
        binding: Binding,
    ) -> UiResult<ControlsOutcome> {
        self.bindings
            .rebind(&action, self.device, 0, binding.clone());
        self.capture = None;
        ui.finish_rebind(binding.display_name(self.family));
        self.refresh(ui)?;
        Ok(ControlsOutcome::Rebound { action, binding })
    }

    /// What a capture on `action` edits for the current device: its first
    /// binding there decides (analogue axis, two-button axis or button).
    fn row_kind(&self, action: &str) -> RowKind {
        match self
            .bindings
            .actions
            .get(action)
            .into_iter()
            .flatten()
            .find(|b| b.device() == Some(self.device))
        {
            Some(Binding::GamepadAxis { scale, .. }) => RowKind::Analog { scale: *scale },
            Some(Binding::Axis { .. }) => RowKind::ButtonAxis,
            _ => RowKind::Button,
        }
    }

    /// Whether the active capture wants an analogue input.
    pub fn wants_axis(&self) -> bool {
        self.capture.as_ref().is_some_and(|c| {
            matches!(
                self.row_kind(&self.entries[c.row].action),
                RowKind::Analog { .. }
            )
        })
    }
}

/// Saves bindings as TOML or JSON (by extension), atomically.
#[cfg(feature = "controls-file")]
pub fn save_bindings(
    path: impl AsRef<std::path::Path>,
    bindings: &Bindings,
) -> Result<(), quasar_config::ConfigError> {
    quasar_config::save_file_typed(path, bindings)
}

/// Loads saved bindings merged over `defaults`; a missing file yields the
/// defaults, a malformed one is an error (so it is never silently replaced).
#[cfg(feature = "controls-file")]
pub fn load_bindings(
    path: impl AsRef<std::path::Path>,
    defaults: &Bindings,
) -> Result<Bindings, quasar_config::ConfigError> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(defaults.clone());
    }
    let saved: Bindings = quasar_config::load_file_typed(path)?;
    saved.validate().map_err(|e| {
        quasar_config::ConfigError::new(quasar_config::ConfigErrorKind::Validation, e.to_string())
    })?;
    Ok(saved.merged_with_defaults(defaults))
}
