//! Portable gamepad state, hotplug routing and player assignment.
//!
//! The core is backend independent: feed [`GamepadEvent`] values from any
//! platform API, a test, or the optional `gilrs` adapter. One
//! [`PlayerInput`] owns one [`InputState`] so keyboard, mouse and gamepad
//! bindings combine into the same named actions. [`GamepadHub`] owns device
//! discovery, stable session ids and player assignment.
use crate::{Bindings, InputState};
#[cfg(feature = "diagnostics")]
use quasar_diagnostics::{Diagnostics, Field, Level};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    error::Error,
    fmt,
};

/// Stable local player slot. Slots are assigned by the game, not by a device.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlayerId(u8);
impl PlayerId {
    pub const fn new(index: u8) -> Self {
        Self(index)
    }
    pub const fn index(self) -> u8 {
        self.0
    }
}
impl From<u8> for PlayerId {
    fn from(index: u8) -> Self {
        Self::new(index)
    }
}
impl fmt::Display for PlayerId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Stable gamepad id for the lifetime of a [`GamepadHub`] session.
///
/// Ids are never reused for a different device during a session. A backend that
/// reconnects the same device (including the bundled `gilrs` adapter) should
/// preserve its id through [`GamepadHub::connect_with_id`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GamepadId(u64);
impl GamepadId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn value(self) -> u64 {
        self.0
    }
}
impl fmt::Display for GamepadId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Layout-independent gamepad buttons. Triggers also have analog axes; the
/// digital variants represent a controller's native trigger button.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GamepadButton {
    South,
    East,
    North,
    West,
    C,
    Z,
    LeftBumper,
    RightBumper,
    LeftTrigger,
    RightTrigger,
    Select,
    Start,
    Mode,
    LeftStick,
    RightStick,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
    Unknown,
}

/// Processed analog axes. Sticks use \[-1,1\]; triggers use \[0,1\] after
/// [`GamepadConfig`] calibration and deadzone handling.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GamepadAxis {
    LeftStickX,
    LeftStickY,
    RightStickX,
    RightStickY,
    LeftTrigger,
    RightTrigger,
}

/// Stable per-device identity used for persisted player preferences.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GamepadFingerprint {
    pub name: String,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    /// Backend UUID when the platform exposes one. This is what lets a saved
    /// preference follow the same physical controller after reconnecting.
    pub uuid: Option<[u8; 16]>,
}
impl GamepadFingerprint {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            vendor_id: None,
            product_id: None,
            uuid: None,
        }
    }
    pub fn with_ids(mut self, vendor_id: Option<u16>, product_id: Option<u16>) -> Self {
        self.vendor_id = vendor_id;
        self.product_id = product_id;
        self
    }
    pub fn with_uuid(mut self, uuid: Option<[u8; 16]>) -> Self {
        self.uuid = uuid;
        self
    }
}

/// Public information about one connected gamepad.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GamepadInfo {
    pub id: GamepadId,
    pub fingerprint: GamepadFingerprint,
}

/// Event accepted by [`GamepadHub::handle_event`] and produced by native
/// adapters. Connected events carry the fingerprint, not a gamepad handle.
#[derive(Clone, Debug, PartialEq)]
pub struct GamepadEvent {
    pub device: GamepadId,
    pub kind: GamepadEventKind,
}
impl GamepadEvent {
    pub fn connected(device: GamepadId, fingerprint: GamepadFingerprint) -> Self {
        Self {
            device,
            kind: GamepadEventKind::Connected(fingerprint),
        }
    }
    pub fn disconnected(device: GamepadId) -> Self {
        Self {
            device,
            kind: GamepadEventKind::Disconnected,
        }
    }
    pub fn button(device: GamepadId, button: GamepadButton, pressed: bool) -> Self {
        Self {
            device,
            kind: GamepadEventKind::Button { button, pressed },
        }
    }
    pub fn axis(device: GamepadId, axis: GamepadAxis, value: f32) -> Self {
        Self {
            device,
            kind: GamepadEventKind::Axis { axis, value },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum GamepadEventKind {
    Connected(GamepadFingerprint),
    Disconnected,
    Button {
        button: GamepadButton,
        pressed: bool,
    },
    Axis {
        axis: GamepadAxis,
        value: f32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GamepadError {
    InvalidConfig(String),
    Backend(String),
    DuplicateGamepad(GamepadId),
    UnknownGamepad(GamepadId),
    UnknownPlayer(PlayerId),
    AlreadyAssigned { device: GamepadId, player: PlayerId },
    DuplicatePlayer(PlayerId),
}

impl fmt::Display for GamepadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) => write!(formatter, "invalid gamepad config: {message}"),
            Self::Backend(message) => write!(formatter, "gamepad backend: {message}"),
            Self::DuplicateGamepad(id) => {
                write!(formatter, "gamepad {} is already connected", id.value())
            }
            Self::UnknownGamepad(id) => {
                write!(formatter, "gamepad {} is not connected", id.value())
            }
            Self::UnknownPlayer(id) => write!(formatter, "player {} does not exist", id.index()),
            Self::AlreadyAssigned { device, player } => write!(
                formatter,
                "gamepad {} is already assigned to player {}",
                device.value(),
                player.index()
            ),
            Self::DuplicatePlayer(id) => {
                write!(formatter, "player {} is configured twice", id.index())
            }
        }
    }
}
impl Error for GamepadError {}

/// Per-axis center/range calibration in raw backend units.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AxisCalibration {
    pub center: f32,
    pub min: f32,
    pub max: f32,
}
impl Default for AxisCalibration {
    fn default() -> Self {
        Self {
            center: 0.0,
            min: -1.0,
            max: 1.0,
        }
    }
}
impl AxisCalibration {
    fn validate(&self) -> Result<(), GamepadError> {
        if !self.center.is_finite()
            || !self.min.is_finite()
            || !self.max.is_finite()
            || self.min >= self.center
            || self.center >= self.max
        {
            return Err(GamepadError::InvalidConfig(
                "axis calibration needs finite min < center < max".into(),
            ));
        }
        Ok(())
    }
    fn apply(self, raw: f32) -> f32 {
        let raw = if raw.is_finite() {
            raw.clamp(self.min, self.max)
        } else {
            self.center
        };
        if raw >= self.center {
            ((raw - self.center) / (self.max - self.center)).clamp(0.0, 1.0)
        } else {
            -((self.center - raw) / (self.center - self.min)).clamp(0.0, 1.0)
        }
    }
}

/// Radial stick deadzone and optional inversion applied after calibration.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StickConfig {
    pub deadzone: f32,
    pub outer: f32,
    pub invert_x: bool,
    pub invert_y: bool,
    pub x: AxisCalibration,
    pub y: AxisCalibration,
}
impl Default for StickConfig {
    fn default() -> Self {
        Self {
            deadzone: 0.15,
            outer: 1.0,
            invert_x: false,
            invert_y: false,
            x: AxisCalibration::default(),
            y: AxisCalibration::default(),
        }
    }
}
impl StickConfig {
    fn validate(&self) -> Result<(), GamepadError> {
        if !self.deadzone.is_finite()
            || !self.outer.is_finite()
            || self.deadzone < 0.0
            || self.outer <= self.deadzone
            || self.outer > 1.0
        {
            return Err(GamepadError::InvalidConfig(
                "stick deadzone/outer must satisfy 0 <= deadzone < outer <= 1".into(),
            ));
        }
        self.x.validate()?;
        self.y.validate()
    }
    fn apply(self, raw_x: f32, raw_y: f32) -> (f32, f32) {
        let mut x = self.x.apply(raw_x);
        let mut y = self.y.apply(raw_y);
        if self.invert_x {
            x = -x;
        }
        if self.invert_y {
            y = -y;
        }
        let magnitude = x.hypot(y);
        if magnitude <= self.deadzone {
            return (0.0, 0.0);
        }
        let scale =
            ((magnitude - self.deadzone) / (self.outer - self.deadzone)).min(1.0) / magnitude;
        ((x * scale).clamp(-1.0, 1.0), (y * scale).clamp(-1.0, 1.0))
    }
}

/// Trigger rest/pressed range calibration plus a normalized deadzone.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TriggerConfig {
    pub rest: f32,
    pub pressed: f32,
    pub deadzone: f32,
}
impl Default for TriggerConfig {
    fn default() -> Self {
        Self {
            rest: -1.0,
            pressed: 1.0,
            deadzone: 0.1,
        }
    }
}
impl TriggerConfig {
    fn validate(&self) -> Result<(), GamepadError> {
        if !self.rest.is_finite()
            || !self.pressed.is_finite()
            || !self.deadzone.is_finite()
            || self.pressed <= self.rest
            || !(0.0..1.0).contains(&self.deadzone)
        {
            return Err(GamepadError::InvalidConfig(
                "trigger needs finite rest < pressed and 0 <= deadzone < 1".into(),
            ));
        }
        Ok(())
    }
    fn apply(self, raw: f32) -> f32 {
        let raw = if raw.is_finite() { raw } else { self.rest };
        let value = ((raw - self.rest) / (self.pressed - self.rest)).clamp(0.0, 1.0);
        if value <= self.deadzone {
            0.0
        } else {
            (value - self.deadzone) / (1.0 - self.deadzone)
        }
    }
}

/// Per-player deadzone, calibration and trigger policy.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GamepadConfig {
    pub left_stick: StickConfig,
    pub right_stick: StickConfig,
    pub left_trigger: TriggerConfig,
    pub right_trigger: TriggerConfig,
    /// Processed trigger value at which the digital trigger button is held.
    pub trigger_button_threshold: f32,
}
impl Default for GamepadConfig {
    fn default() -> Self {
        Self {
            left_stick: StickConfig::default(),
            right_stick: StickConfig::default(),
            left_trigger: TriggerConfig::default(),
            right_trigger: TriggerConfig::default(),
            trigger_button_threshold: 0.5,
        }
    }
}
impl GamepadConfig {
    pub fn validate(&self) -> Result<(), GamepadError> {
        if !self.trigger_button_threshold.is_finite()
            || !(0.0..=1.0).contains(&self.trigger_button_threshold)
        {
            return Err(GamepadError::InvalidConfig(
                "trigger_button_threshold must be in [0,1]".into(),
            ));
        }
        self.left_stick.validate()?;
        self.right_stick.validate()?;
        self.left_trigger.validate()?;
        self.right_trigger.validate()
    }
}

/// Raw and processed state for one assigned gamepad.
///
/// Backends feed normalized raw axes in \[-1,1\]. Stick calibration/deadzone and
/// trigger rest/pressed ranges are applied by the accessors below; a missing
/// trigger axis is treated as at rest.
#[derive(Clone, Debug, Default)]
pub struct GamepadState {
    buttons: HashSet<GamepadButton>,
    axes: BTreeMap<GamepadAxis, f32>,
    config: GamepadConfig,
}
impl GamepadState {
    pub fn new(config: GamepadConfig) -> Result<Self, GamepadError> {
        config.validate()?;
        Ok(Self {
            buttons: HashSet::new(),
            axes: BTreeMap::new(),
            config,
        })
    }
    pub fn config(&self) -> &GamepadConfig {
        &self.config
    }
    pub fn set_config(&mut self, config: GamepadConfig) -> Result<(), GamepadError> {
        config.validate()?;
        self.config = config;
        Ok(())
    }
    pub fn button(&self, button: GamepadButton) -> bool {
        self.buttons.contains(&button)
    }
    pub fn buttons(&self) -> impl Iterator<Item = GamepadButton> + '_ {
        self.buttons.iter().copied()
    }
    pub fn raw_axis(&self, axis: GamepadAxis) -> f32 {
        self.axes.get(&axis).copied().unwrap_or(0.0)
    }
    pub fn set_button(&mut self, button: GamepadButton, pressed: bool) -> bool {
        if pressed {
            self.buttons.insert(button)
        } else {
            self.buttons.remove(&button)
        }
    }
    pub fn set_axis(&mut self, axis: GamepadAxis, value: f32) -> bool {
        let value = if value.is_finite() {
            value.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        if self
            .axes
            .get(&axis)
            .is_none_or(|current| (*current - value).abs() > f32::EPSILON)
        {
            self.axes.insert(axis, value);
            true
        } else {
            false
        }
    }
    pub fn release_all(&mut self) {
        self.buttons.clear();
        self.axes.clear();
    }
    pub fn left_stick(&self) -> (f32, f32) {
        self.config.left_stick.apply(
            self.raw_axis(GamepadAxis::LeftStickX),
            self.raw_axis(GamepadAxis::LeftStickY),
        )
    }
    pub fn right_stick(&self) -> (f32, f32) {
        self.config.right_stick.apply(
            self.raw_axis(GamepadAxis::RightStickX),
            self.raw_axis(GamepadAxis::RightStickY),
        )
    }
    pub fn left_trigger(&self) -> f32 {
        self.config.left_trigger.apply(
            self.axes
                .get(&GamepadAxis::LeftTrigger)
                .copied()
                .unwrap_or(self.config.left_trigger.rest),
        )
    }
    pub fn right_trigger(&self) -> f32 {
        self.config.right_trigger.apply(
            self.axes
                .get(&GamepadAxis::RightTrigger)
                .copied()
                .unwrap_or(self.config.right_trigger.rest),
        )
    }
    pub fn processed_axis(&self, axis: GamepadAxis) -> f32 {
        match axis {
            GamepadAxis::LeftStickX => self.left_stick().0,
            GamepadAxis::LeftStickY => self.left_stick().1,
            GamepadAxis::RightStickX => self.right_stick().0,
            GamepadAxis::RightStickY => self.right_stick().1,
            GamepadAxis::LeftTrigger => self.left_trigger(),
            GamepadAxis::RightTrigger => self.right_trigger(),
        }
    }
}

/// Connected device registry. Ids are allocated monotonically and are never
/// reused for a different device in one session.
#[derive(Debug, Default)]
pub struct GamepadSet {
    next: u64,
    devices: BTreeMap<GamepadId, GamepadInfo>,
}
impl GamepadSet {
    pub fn connect(&mut self, fingerprint: GamepadFingerprint) -> GamepadId {
        loop {
            self.next = self.next.wrapping_add(1);
            if self.next == 0 {
                self.next = 1;
            }
            if !self.devices.contains_key(&GamepadId::new(self.next)) {
                break;
            }
        }
        let id = GamepadId::new(self.next);
        self.devices.insert(id, GamepadInfo { id, fingerprint });
        id
    }
    pub fn insert(&mut self, info: GamepadInfo) -> Result<(), GamepadError> {
        if self.devices.contains_key(&info.id) {
            return Err(GamepadError::DuplicateGamepad(info.id));
        }
        self.devices.insert(info.id, info);
        Ok(())
    }
    pub fn disconnect(&mut self, id: GamepadId) -> Option<GamepadInfo> {
        self.devices.remove(&id)
    }
    pub fn get(&self, id: GamepadId) -> Option<&GamepadInfo> {
        self.devices.get(&id)
    }
    pub fn contains(&self, id: GamepadId) -> bool {
        self.devices.contains_key(&id)
    }
    pub fn iter(&self) -> impl Iterator<Item = &GamepadInfo> {
        self.devices.values()
    }
    pub fn len(&self) -> usize {
        self.devices.len()
    }
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }
}

/// Persisted player-to-device preference. Session ids are deliberately absent;
/// fingerprints are matched against future connections.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GamepadPreferences {
    pub players: Vec<PlayerGamepadPreference>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerGamepadPreference {
    pub player: PlayerId,
    pub fingerprint: GamepadFingerprint,
}

/// One player's input state and assigned gamepad.
#[derive(Debug)]
pub struct PlayerInput {
    pub id: PlayerId,
    pub input: InputState,
    gamepad: Option<GamepadId>,
    state: GamepadState,
}
impl PlayerInput {
    pub fn new(id: PlayerId) -> Self {
        Self {
            id,
            input: InputState::default(),
            gamepad: None,
            state: GamepadState::default(),
        }
    }
    pub fn with_config(id: PlayerId, config: GamepadConfig) -> Result<Self, GamepadError> {
        config.validate()?;
        Ok(Self {
            state: GamepadState::new(config)?,
            ..Self::new(id)
        })
    }
    pub fn gamepad(&self) -> Option<GamepadId> {
        self.gamepad
    }
    pub fn gamepad_state(&self) -> &GamepadState {
        &self.state
    }
    pub fn set_config(&mut self, config: GamepadConfig) -> Result<(), GamepadError> {
        self.state.set_config(config)?;
        self.refresh_axes();
        Ok(())
    }
    /// Clears keyboard, mouse and gamepad state for this player. Use on focus
    /// loss so stale held buttons cannot survive a focus transition.
    pub fn release_all(&mut self) {
        self.input.release_all();
        self.state.release_all();
    }
    fn refresh_axes(&mut self) {
        let (left_x, left_y) = self.state.left_stick();
        let (right_x, right_y) = self.state.right_stick();
        let left_trigger = self.state.left_trigger();
        let right_trigger = self.state.right_trigger();
        let threshold = self.state.config().trigger_button_threshold;
        self.input.gamepad_axes([
            (GamepadAxis::LeftStickX, left_x),
            (GamepadAxis::LeftStickY, left_y),
            (GamepadAxis::RightStickX, right_x),
            (GamepadAxis::RightStickY, right_y),
            (GamepadAxis::LeftTrigger, left_trigger),
            (GamepadAxis::RightTrigger, right_trigger),
        ]);
        self.input
            .gamepad_button(GamepadButton::LeftTrigger, left_trigger >= threshold);
        self.input
            .gamepad_button(GamepadButton::RightTrigger, right_trigger >= threshold);
    }
    /// Assigns or clears the controller. Clearing releases only gamepad-derived
    /// input, preserving keyboard and mouse held state.
    pub fn assign(&mut self, gamepad: Option<GamepadId>) {
        if self.gamepad != gamepad {
            self.input.release_gamepad();
            self.state.release_all();
            self.gamepad = gamepad;
        }
    }
    pub fn button(&mut self, button: GamepadButton, pressed: bool) {
        self.state.set_button(button, pressed);
        self.input.gamepad_button(button, pressed);
    }
    pub fn axis(&mut self, axis: GamepadAxis, raw: f32) {
        if !self.state.set_axis(axis, raw) {
            return;
        }
        match axis {
            GamepadAxis::LeftStickX | GamepadAxis::LeftStickY => {
                let (x, y) = self.state.left_stick();
                self.input
                    .gamepad_axes([(GamepadAxis::LeftStickX, x), (GamepadAxis::LeftStickY, y)]);
            }
            GamepadAxis::RightStickX | GamepadAxis::RightStickY => {
                let (x, y) = self.state.right_stick();
                self.input
                    .gamepad_axes([(GamepadAxis::RightStickX, x), (GamepadAxis::RightStickY, y)]);
            }
            GamepadAxis::LeftTrigger => {
                let value = self.state.left_trigger();
                let threshold = self.state.config().trigger_button_threshold;
                self.input.gamepad_axis(GamepadAxis::LeftTrigger, value);
                self.input
                    .gamepad_button(GamepadButton::LeftTrigger, value >= threshold);
            }
            GamepadAxis::RightTrigger => {
                let value = self.state.right_trigger();
                let threshold = self.state.config().trigger_button_threshold;
                self.input.gamepad_axis(GamepadAxis::RightTrigger, value);
                self.input
                    .gamepad_button(GamepadButton::RightTrigger, value >= threshold);
            }
        }
    }
    /// Applies an event when it belongs to this player's assigned device.
    pub fn handle_event(&mut self, event: &GamepadEvent) -> bool {
        if self.gamepad != Some(event.device) {
            return false;
        }
        match &event.kind {
            GamepadEventKind::Button { button, pressed } => self.button(*button, *pressed),
            GamepadEventKind::Axis { axis, value } => self.axis(*axis, *value),
            GamepadEventKind::Disconnected => self.assign(None),
            GamepadEventKind::Connected(_) => {}
        }
        true
    }
}

#[derive(Debug, Default)]
struct PlayerAssignments {
    preferences: BTreeMap<PlayerId, GamepadFingerprint>,
    active: BTreeMap<PlayerId, GamepadId>,
}

/// Device discovery, player assignment and event routing.
#[derive(Debug, Default)]
pub struct GamepadHub {
    devices: GamepadSet,
    players: BTreeMap<PlayerId, PlayerInput>,
    assignments: PlayerAssignments,
    #[cfg(feature = "diagnostics")]
    diagnostics: Option<Diagnostics>,
}
impl GamepadHub {
    pub fn new() -> Self {
        Self::default()
    }
    /// Attaches an explicit diagnostics handle for hotplug/assignment events.
    #[cfg(feature = "diagnostics")]
    pub fn set_diagnostics(&mut self, diagnostics: Option<Diagnostics>) {
        self.diagnostics = diagnostics;
    }
    #[cfg(feature = "diagnostics")]
    pub fn diagnostics(&self) -> Option<&Diagnostics> {
        self.diagnostics.as_ref()
    }
    pub fn devices(&self) -> &GamepadSet {
        &self.devices
    }
    pub fn players(&self) -> impl Iterator<Item = &PlayerInput> {
        self.players.values()
    }
    pub fn players_mut(&mut self) -> impl Iterator<Item = &mut PlayerInput> {
        self.players.values_mut()
    }
    pub fn player(&self, id: PlayerId) -> Option<&PlayerInput> {
        self.players.get(&id)
    }
    pub fn player_mut(&mut self, id: PlayerId) -> Option<&mut PlayerInput> {
        self.players.get_mut(&id)
    }
    pub fn add_player(&mut self, id: PlayerId) -> Result<(), GamepadError> {
        self.add_player_with(id, InputState::default())
    }
    pub fn add_player_with_bindings(
        &mut self,
        id: PlayerId,
        bindings: Bindings,
    ) -> Result<(), GamepadError> {
        if self.players.contains_key(&id) {
            return Err(GamepadError::DuplicatePlayer(id));
        }
        let mut input = InputState::default();
        input
            .set_bindings(bindings)
            .map_err(|error| GamepadError::InvalidConfig(error.to_string()))?;
        self.players.insert(
            id,
            PlayerInput {
                input,
                ..PlayerInput::new(id)
            },
        );
        Ok(())
    }
    fn add_player_with(&mut self, id: PlayerId, input: InputState) -> Result<(), GamepadError> {
        if self.players.contains_key(&id) {
            return Err(GamepadError::DuplicatePlayer(id));
        }
        self.players.insert(
            id,
            PlayerInput {
                input,
                ..PlayerInput::new(id)
            },
        );
        Ok(())
    }
    pub fn remove_player(&mut self, id: PlayerId) -> Option<PlayerInput> {
        self.assignments.active.remove(&id);
        self.assignments.preferences.remove(&id);
        self.players.remove(&id)
    }

    /// Registers a device and allocates a fresh session id.
    pub fn connect(&mut self, fingerprint: GamepadFingerprint) -> GamepadId {
        let id = self.devices.connect(fingerprint);
        self.reconcile_assignments();
        #[cfg(feature = "diagnostics")]
        if let Some(diagnostics) = &self.diagnostics
            && let Some(info) = self.devices.get(id)
        {
            diagnostics.event_with(
                Level::Info,
                "quasar.input",
                "gamepad_connected",
                [
                    Field::new("gamepad", id.value()),
                    Field::new("name", info.fingerprint.name.as_str()),
                    Field::new(
                        "vendor",
                        info.fingerprint.vendor_id.map_or(u64::MAX, u64::from),
                    ),
                    Field::new(
                        "product",
                        info.fingerprint.product_id.map_or(u64::MAX, u64::from),
                    ),
                ],
            );
        }
        id
    }
    /// Registers a device with a backend-preserved session id. This is how a
    /// reconnect keeps the same [`GamepadId`].
    pub fn connect_with_id(
        &mut self,
        id: GamepadId,
        fingerprint: GamepadFingerprint,
    ) -> Result<(), GamepadError> {
        self.devices.insert(GamepadInfo { id, fingerprint })?;
        self.reconcile_assignments();
        #[cfg(feature = "diagnostics")]
        if let Some(diagnostics) = &self.diagnostics
            && let Some(info) = self.devices.get(id)
        {
            diagnostics.event_with(
                Level::Info,
                "quasar.input",
                "gamepad_connected",
                [
                    Field::new("gamepad", id.value()),
                    Field::new("name", info.fingerprint.name.as_str()),
                ],
            );
        }
        Ok(())
    }
    pub fn disconnect(&mut self, id: GamepadId) -> bool {
        if self.devices.disconnect(id).is_none() {
            return false;
        }
        #[cfg(feature = "diagnostics")]
        if let Some(diagnostics) = &self.diagnostics {
            diagnostics.event_with(
                Level::Info,
                "quasar.input",
                "gamepad_disconnected",
                [Field::new("gamepad", id.value())],
            );
        }
        let players: Vec<PlayerId> = self
            .assignments
            .active
            .iter()
            .filter_map(|(player, device)| (*device == id).then_some(*player))
            .collect();
        for player in players {
            self.assignments.active.remove(&player);
            if let Some(input) = self.players.get_mut(&player) {
                input.assign(None);
            }
        }
        true
    }
    pub fn device(&self, id: GamepadId) -> Option<&GamepadInfo> {
        self.devices.get(id)
    }
    /// Releases input for every player while preserving device assignments and
    /// reconnect preferences. Intended for window focus loss.
    pub fn release_all(&mut self) {
        for player in self.players.values_mut() {
            player.release_all();
        }
    }
    pub fn assignment(&self, player: PlayerId) -> Option<GamepadId> {
        self.assignments.active.get(&player).copied()
    }
    /// Assigns a connected device and remembers its fingerprint for reconnect.
    pub fn assign(&mut self, player: PlayerId, device: GamepadId) -> Result<(), GamepadError> {
        let Some(info) = self.devices.get(device) else {
            return Err(GamepadError::UnknownGamepad(device));
        };
        let fingerprint = info.fingerprint.clone();
        let Some(input) = self.players.get_mut(&player) else {
            return Err(GamepadError::UnknownPlayer(player));
        };
        if let Some((other, _)) = self
            .assignments
            .active
            .iter()
            .find(|(other, assigned)| **assigned == device && **other != player)
        {
            return Err(GamepadError::AlreadyAssigned {
                device,
                player: *other,
            });
        }
        self.assignments.active.insert(player, device);
        self.assignments.preferences.insert(player, fingerprint);
        input.assign(Some(device));
        #[cfg(feature = "diagnostics")]
        if let Some(diagnostics) = &self.diagnostics {
            diagnostics.event_with(
                Level::Info,
                "quasar.input",
                "gamepad_assigned",
                [
                    Field::new("player", u64::from(player.index())),
                    Field::new("gamepad", device.value()),
                ],
            );
        }
        Ok(())
    }
    /// Remembers a preferred device without requiring it to be connected now.
    pub fn prefer(
        &mut self,
        player: PlayerId,
        fingerprint: GamepadFingerprint,
    ) -> Result<(), GamepadError> {
        if !self.players.contains_key(&player) {
            return Err(GamepadError::UnknownPlayer(player));
        }
        self.assignments.preferences.insert(player, fingerprint);
        self.reconcile_assignments();
        Ok(())
    }
    /// Clears both the active assignment and the reconnect preference.
    pub fn clear_assignment(&mut self, player: PlayerId) -> Option<GamepadId> {
        self.assignments.preferences.remove(&player);
        let device = self.assignments.active.remove(&player);
        if let Some(input) = self.players.get_mut(&player) {
            input.assign(None);
        }
        device
    }
    pub fn preferences(&self) -> GamepadPreferences {
        GamepadPreferences {
            players: self
                .assignments
                .preferences
                .iter()
                .map(|(player, fingerprint)| PlayerGamepadPreference {
                    player: *player,
                    fingerprint: fingerprint.clone(),
                })
                .collect(),
        }
    }
    pub fn apply_preferences(
        &mut self,
        preferences: GamepadPreferences,
    ) -> Result<(), GamepadError> {
        let mut seen = HashSet::new();
        for preference in &preferences.players {
            if !seen.insert(preference.player) {
                return Err(GamepadError::DuplicatePlayer(preference.player));
            }
            if !self.players.contains_key(&preference.player) {
                return Err(GamepadError::UnknownPlayer(preference.player));
            }
        }
        let players: Vec<PlayerId> = self.players.keys().copied().collect();
        for player in players {
            self.assignments.preferences.remove(&player);
            self.assignments.active.remove(&player);
            if let Some(input) = self.players.get_mut(&player) {
                input.assign(None);
            }
        }
        for preference in preferences.players {
            self.assignments
                .preferences
                .insert(preference.player, preference.fingerprint);
        }
        self.reconcile_assignments();
        Ok(())
    }

    /// Sends a button event to the player currently assigned to `device`.
    /// Unassigned devices are valid but events are ignored until assigned.
    pub fn button(&mut self, device: GamepadId, button: GamepadButton, pressed: bool) -> bool {
        let Some(player) = self.player_for_device(device) else {
            return false;
        };
        if let Some(input) = self.players.get_mut(&player) {
            input.button(button, pressed);
            return true;
        }
        false
    }
    pub fn axis(&mut self, device: GamepadId, axis: GamepadAxis, value: f32) -> bool {
        let Some(player) = self.player_for_device(device) else {
            return false;
        };
        if let Some(input) = self.players.get_mut(&player) {
            input.axis(axis, value);
            return true;
        }
        false
    }
    pub fn handle_event(&mut self, event: &GamepadEvent) -> Result<bool, GamepadError> {
        let result = match &event.kind {
            GamepadEventKind::Connected(fingerprint) => {
                if self.devices.contains(event.device) {
                    Err(GamepadError::DuplicateGamepad(event.device))
                } else {
                    self.connect_with_id(event.device, fingerprint.clone())
                        .map(|()| true)
                }
            }
            GamepadEventKind::Disconnected => Ok(self.disconnect(event.device)),
            GamepadEventKind::Button { button, pressed } => {
                Ok(self.button(event.device, *button, *pressed))
            }
            GamepadEventKind::Axis { axis, value } => Ok(self.axis(event.device, *axis, *value)),
        };
        #[cfg(feature = "diagnostics")]
        if let Err(error) = &result
            && let Some(diagnostics) = &self.diagnostics
        {
            diagnostics.failure_with_fields(
                "quasar.input",
                "gamepad_event",
                error,
                [Field::new("gamepad", event.device.value())],
            );
        }
        result
    }
    fn player_for_device(&self, device: GamepadId) -> Option<PlayerId> {
        self.assignments
            .active
            .iter()
            .find_map(|(player, assigned)| (*assigned == device).then_some(*player))
    }
    fn reconcile_assignments(&mut self) {
        let active: HashSet<GamepadId> = self.assignments.active.values().copied().collect();
        let mut used = active;
        let mut assignments = Vec::new();
        for (player, fingerprint) in &self.assignments.preferences {
            if self.assignments.active.contains_key(player) {
                continue;
            }
            if let Some(info) = self
                .devices
                .iter()
                .find(|info| info.fingerprint == *fingerprint && !used.contains(&info.id))
            {
                used.insert(info.id);
                assignments.push((*player, info.id));
            }
        }
        for (player, device) in assignments {
            self.assignments.active.insert(player, device);
            if let Some(input) = self.players.get_mut(&player) {
                input.assign(Some(device));
            }
        }
    }
}

#[cfg(feature = "gilrs")]
mod gilrs_backend {
    use super::*;
    use std::collections::HashMap;

    /// Optional `gilrs`/evdev/XInput/GameController native backend.
    ///
    /// Call [`Self::poll`] once per frame. The adapter preserves a Quasar
    /// [`GamepadId`] when `gilrs` reuses its id for a reconnected device.
    pub struct GilrsGamepads {
        inner: gilrs::Gilrs,
        ids: HashMap<gilrs::GamepadId, GamepadId>,
    }
    impl fmt::Debug for GilrsGamepads {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("GilrsGamepads")
                .field("gamepads", &self.ids.len())
                .finish()
        }
    }
    impl GilrsGamepads {
        pub fn new() -> Result<Self, GamepadError> {
            let inner =
                gilrs::Gilrs::new().map_err(|error| GamepadError::Backend(error.to_string()))?;
            Ok(Self {
                inner,
                ids: HashMap::new(),
            })
        }
        /// Polls all pending native events into the hub. Returns the number of
        /// backend events consumed.
        pub fn poll(&mut self, hub: &mut GamepadHub) -> Result<usize, GamepadError> {
            // Register controllers that are already present even if the backend
            // has not queued a Connected event for them.
            let current: Vec<gilrs::GamepadId> = self.inner.gamepads().map(|(id, _)| id).collect();
            for id in current {
                self.connect_gamepad(hub, id);
            }
            let mut count = 0;
            while let Some(event) = self.inner.next_event() {
                count += 1;
                match event.event {
                    gilrs::EventType::Connected => {
                        self.connect_gamepad(hub, event.id);
                    }
                    gilrs::EventType::Disconnected => {
                        if let Some(id) = self.ids.get(&event.id).copied() {
                            hub.disconnect(id);
                        }
                    }
                    gilrs::EventType::ButtonPressed(button, _) => {
                        if let Some(id) = self.ensure(hub, event.id) {
                            hub.button(id, map_button(button), true);
                        }
                    }
                    gilrs::EventType::ButtonReleased(button, _) => {
                        if let Some(id) = self.ensure(hub, event.id) {
                            hub.button(id, map_button(button), false);
                        }
                    }
                    gilrs::EventType::AxisChanged(axis, value, _) => {
                        if let Some(id) = self.ensure(hub, event.id)
                            && let Some(axis) = map_axis(axis)
                        {
                            hub.axis(id, axis, value);
                        }
                    }
                    gilrs::EventType::ButtonChanged(button, value, _) => {
                        // Native trigger values are also delivered as axis
                        // changes on supported mappings. Only map a changed button
                        // when it is a trigger and no axis has set it yet.
                        if let Some(id) = self.ensure(hub, event.id)
                            && let Some(axis) = trigger_axis(button)
                        {
                            hub.axis(id, axis, value);
                        }
                    }
                    _ => {}
                }
            }
            Ok(count)
        }
        /// Registers currently connected controllers without consuming events.
        pub fn connected(&mut self, hub: &mut GamepadHub) -> usize {
            let ids: Vec<gilrs::GamepadId> = self.inner.gamepads().map(|(id, _)| id).collect();
            ids.into_iter()
                .filter(|id| self.connect_gamepad(hub, *id).is_some())
                .count()
        }
        fn ensure(&mut self, hub: &mut GamepadHub, backend: gilrs::GamepadId) -> Option<GamepadId> {
            if let Some(id) = self.ids.get(&backend).copied()
                && hub.device(id).is_some()
            {
                return Some(id);
            }
            self.connect_gamepad(hub, backend)
        }
        fn connect_gamepad(
            &mut self,
            hub: &mut GamepadHub,
            backend: gilrs::GamepadId,
        ) -> Option<GamepadId> {
            let gamepad = self.inner.gamepad(backend);
            let fingerprint = GamepadFingerprint {
                name: gamepad.name().to_owned(),
                vendor_id: gamepad.vendor_id(),
                product_id: gamepad.product_id(),
                uuid: Some(gamepad.uuid()),
            };
            if let Some(id) = self.ids.get(&backend).copied() {
                if hub.device(id).is_none() && hub.connect_with_id(id, fingerprint).is_err() {
                    return None;
                }
                return Some(id);
            }
            let id = hub.connect(fingerprint);
            self.ids.insert(backend, id);
            Some(id)
        }
    }

    fn map_button(button: gilrs::Button) -> GamepadButton {
        match button {
            gilrs::Button::South => GamepadButton::South,
            gilrs::Button::East => GamepadButton::East,
            gilrs::Button::North => GamepadButton::North,
            gilrs::Button::West => GamepadButton::West,
            gilrs::Button::C => GamepadButton::C,
            gilrs::Button::Z => GamepadButton::Z,
            gilrs::Button::LeftTrigger => GamepadButton::LeftBumper,
            gilrs::Button::LeftTrigger2 => GamepadButton::LeftTrigger,
            gilrs::Button::RightTrigger => GamepadButton::RightBumper,
            gilrs::Button::RightTrigger2 => GamepadButton::RightTrigger,
            gilrs::Button::Select => GamepadButton::Select,
            gilrs::Button::Start => GamepadButton::Start,
            gilrs::Button::Mode => GamepadButton::Mode,
            gilrs::Button::LeftThumb => GamepadButton::LeftStick,
            gilrs::Button::RightThumb => GamepadButton::RightStick,
            gilrs::Button::DPadUp => GamepadButton::DPadUp,
            gilrs::Button::DPadDown => GamepadButton::DPadDown,
            gilrs::Button::DPadLeft => GamepadButton::DPadLeft,
            gilrs::Button::DPadRight => GamepadButton::DPadRight,
            _ => GamepadButton::Unknown,
        }
    }
    fn map_axis(axis: gilrs::Axis) -> Option<GamepadAxis> {
        Some(match axis {
            gilrs::Axis::LeftStickX => GamepadAxis::LeftStickX,
            gilrs::Axis::LeftStickY => GamepadAxis::LeftStickY,
            gilrs::Axis::RightStickX => GamepadAxis::RightStickX,
            gilrs::Axis::RightStickY => GamepadAxis::RightStickY,
            gilrs::Axis::LeftZ => GamepadAxis::LeftTrigger,
            gilrs::Axis::RightZ => GamepadAxis::RightTrigger,
            _ => return None,
        })
    }
    fn trigger_axis(button: gilrs::Button) -> Option<GamepadAxis> {
        match button {
            gilrs::Button::LeftTrigger2 => Some(GamepadAxis::LeftTrigger),
            gilrs::Button::RightTrigger2 => Some(GamepadAxis::RightTrigger),
            _ => None,
        }
    }
}

#[cfg(feature = "gilrs")]
pub use gilrs_backend::GilrsGamepads;
