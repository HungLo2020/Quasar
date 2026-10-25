use crate::error::finite;
use crate::{AnimationError, AnimationResult, Clip, LoopMode, Playback, Player, Pose};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ParameterValue {
    Bool(bool),
    Float(f32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Compare {
    Less,
    LessOrEqual,
    Equal,
    GreaterOrEqual,
    Greater,
}

impl Compare {
    fn test(self, left: f32, right: f32) -> bool {
        match self {
            Self::Less => left < right,
            Self::LessOrEqual => left <= right,
            Self::Equal => (left - right).abs() <= f32::EPSILON,
            Self::GreaterOrEqual => left >= right,
            Self::Greater => left > right,
        }
    }
}

/// Explicit transition condition. Parameters are global to one state machine.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Condition {
    Always,
    Bool {
        parameter: String,
        equals: bool,
    },
    Float {
        parameter: String,
        compare: Compare,
        value: f32,
    },
    Trigger(String),
    StateTimeGreater(f32),
    All(Vec<Condition>),
    Any(Vec<Condition>),
}

impl Condition {
    fn triggers(&self, names: &mut Vec<String>) {
        match self {
            Self::Trigger(name) => names.push(name.clone()),
            Self::All(conditions) | Self::Any(conditions) => {
                for condition in conditions {
                    condition.triggers(names);
                }
            }
            _ => {}
        }
    }

    pub fn is_valid(&self) -> bool {
        match self {
            Self::Always => true,
            Self::Bool { parameter, .. } | Self::Trigger(parameter) => !parameter.is_empty(),
            Self::Float {
                parameter, value, ..
            } => !parameter.is_empty() && value.is_finite(),
            Self::StateTimeGreater(value) => value.is_finite() && *value >= 0.0,
            Self::All(conditions) | Self::Any(conditions) => {
                !conditions.is_empty() && conditions.iter().all(Self::is_valid)
            }
        }
    }

    fn matches(&self, parameters: &BTreeMap<String, ParameterValue>, state_time: f32) -> bool {
        match self {
            Self::Always => true,
            Self::Bool { parameter, equals } => {
                matches!(parameters.get(parameter), Some(ParameterValue::Bool(value)) if value == equals)
            }
            Self::Float {
                parameter,
                compare,
                value,
            } => match parameters.get(parameter) {
                Some(ParameterValue::Float(current)) => compare.test(*current, *value),
                _ => false,
            },
            Self::Trigger(parameter) => {
                matches!(parameters.get(parameter), Some(ParameterValue::Bool(true)))
            }
            Self::StateTimeGreater(value) => state_time > *value,
            Self::All(conditions) => conditions
                .iter()
                .all(|condition| condition.matches(parameters, state_time)),
            Self::Any(conditions) => conditions
                .iter()
                .any(|condition| condition.matches(parameters, state_time)),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Transition {
    /// `None` means "from the current state".
    pub from: Option<String>,
    pub to: String,
    /// Crossfade duration. Zero switches immediately.
    pub duration: f32,
    pub condition: Condition,
}

impl Transition {
    pub fn new(
        from: Option<String>,
        to: impl Into<String>,
        duration: f32,
        condition: Condition,
    ) -> AnimationResult<Self> {
        let to = to.into();
        finite(duration)?;
        if duration < 0.0
            || to.is_empty()
            || from.as_ref().is_some_and(|from| from.is_empty())
            || !condition.is_valid()
        {
            return Err(AnimationError::InvalidTransition);
        }
        Ok(Self {
            from,
            to,
            duration,
            condition,
        })
    }

    /// Creates a transition that may fire from any current state.
    pub fn from_any(
        to: impl Into<String>,
        duration: f32,
        condition: Condition,
    ) -> AnimationResult<Self> {
        Self::new(None, to, duration, condition)
    }
}

#[derive(Clone)]
struct StateDefinition {
    clip: Arc<Clip>,
    loop_mode: LoopMode,
    speed: f32,
}

struct Fade {
    from: Player,
    elapsed: f32,
    duration: f32,
}

struct Layer {
    name: String,
    states: BTreeMap<String, StateDefinition>,
    transitions: Vec<Transition>,
    current: Option<String>,
    player: Option<Player>,
    fade: Option<Fade>,
    weight: f32,
}

impl Layer {
    fn new(name: String) -> Self {
        Self {
            name,
            states: BTreeMap::new(),
            transitions: Vec::new(),
            current: None,
            player: None,
            fade: None,
            weight: 1.0,
        }
    }

    fn pose(&self) -> Pose {
        let Some(player) = &self.player else {
            return Pose::new();
        };
        let target = player.sample();
        if let Some(fade) = &self.fade {
            let duration = fade.duration;
            let t = if duration <= f32::EPSILON {
                1.0
            } else {
                (fade.elapsed / duration).clamp(0.0, 1.0)
            };
            let t = t * t * (3.0 - 2.0 * t);
            fade.from.sample().blend(&target, t)
        } else {
            target
        }
    }
}

/// Layered state machine. Each layer has independent states/transitions and a
/// blend weight; all layers share the parameter map. A game owns one machine per
/// character or object and advances it from its simulation tick.
pub struct StateMachine {
    layers: Vec<Layer>,
    parameters: BTreeMap<String, ParameterValue>,
}

impl Default for StateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl StateMachine {
    pub fn new() -> Self {
        Self {
            layers: Vec::new(),
            parameters: BTreeMap::new(),
        }
    }

    pub fn add_layer(&mut self, name: impl Into<String>) -> usize {
        let index = self.layers.len();
        self.layers.push(Layer::new(name.into()));
        index
    }

    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    pub fn add_state(
        &mut self,
        layer: usize,
        name: impl Into<String>,
        clip: Arc<Clip>,
        loop_mode: LoopMode,
        speed: f32,
    ) -> AnimationResult<()> {
        let layer = self
            .layers
            .get_mut(layer)
            .ok_or(AnimationError::InvalidLayer)?;
        finite(speed)?;
        let name = name.into();
        if name.is_empty() {
            return Err(AnimationError::InvalidLayer);
        }
        if layer.states.contains_key(&name) {
            return Err(AnimationError::DuplicateState);
        }
        layer.states.insert(
            name.clone(),
            StateDefinition {
                clip,
                loop_mode,
                speed,
            },
        );
        if layer.current.is_none() {
            layer.current = Some(name.clone());
            layer.player = Some(make_player(layer, &name)?);
        }
        Ok(())
    }

    pub fn set_initial(&mut self, layer: usize, state: impl Into<String>) -> AnimationResult<()> {
        let state = state.into();
        let layer = self
            .layers
            .get_mut(layer)
            .ok_or(AnimationError::InvalidLayer)?;
        if !layer.states.contains_key(&state) {
            return Err(AnimationError::UnknownState);
        }
        layer.player = Some(make_player(layer, &state)?);
        layer.current = Some(state);
        layer.fade = None;
        Ok(())
    }

    pub fn add_transition(&mut self, layer: usize, transition: Transition) -> AnimationResult<()> {
        let layer = self
            .layers
            .get_mut(layer)
            .ok_or(AnimationError::InvalidLayer)?;
        if !layer.states.contains_key(&transition.to)
            || transition
                .from
                .as_ref()
                .is_some_and(|from| !layer.states.contains_key(from))
        {
            return Err(AnimationError::UnknownState);
        }
        layer.transitions.push(transition);
        Ok(())
    }

    pub fn set_parameter(&mut self, name: impl Into<String>, value: ParameterValue) {
        self.parameters.insert(name.into(), value);
    }

    pub fn parameter(&self, name: &str) -> Option<ParameterValue> {
        self.parameters.get(name).copied()
    }

    pub fn trigger(&mut self, name: impl Into<String>) {
        self.parameters
            .insert(name.into(), ParameterValue::Bool(true));
    }

    pub fn clear_trigger(&mut self, name: &str) {
        if let Some(value) = self.parameters.get_mut(name) {
            *value = ParameterValue::Bool(false);
        }
    }

    pub fn set_layer_weight(&mut self, layer: usize, weight: f32) -> AnimationResult<()> {
        finite(weight)?;
        let layer = self
            .layers
            .get_mut(layer)
            .ok_or(AnimationError::InvalidLayer)?;
        layer.weight = weight.clamp(0.0, 1.0);
        Ok(())
    }

    pub fn layer_weight(&self, layer: usize) -> Option<f32> {
        self.layers.get(layer).map(|layer| layer.weight)
    }

    pub fn layer_name(&self, layer: usize) -> Option<&str> {
        self.layers.get(layer).map(|layer| layer.name.as_str())
    }

    pub fn current_state(&self, layer: usize) -> Option<&str> {
        self.layers.get(layer)?.current.as_deref()
    }

    pub fn state_time(&self, layer: usize) -> Option<f32> {
        self.layers.get(layer)?.player.as_ref().map(Player::time)
    }

    pub fn is_fading(&self, layer: usize) -> bool {
        self.layers
            .get(layer)
            .is_some_and(|layer| layer.fade.is_some())
    }

    pub fn update(&mut self, delta_seconds: f32) -> AnimationResult<Playback> {
        finite(delta_seconds)?;
        if delta_seconds < 0.0 {
            return Err(AnimationError::InvalidDuration);
        }
        let parameters = &self.parameters;
        let mut result = Playback::default();
        let mut used_triggers = Vec::new();
        for layer in &mut self.layers {
            let Some(player) = &mut layer.player else {
                continue;
            };
            let mut layer_playback = player.advance(delta_seconds)?;
            if let Some(fade) = &mut layer.fade {
                let from_playback = fade.from.advance(delta_seconds)?;
                merge_playback(&mut layer_playback, from_playback);
                fade.elapsed += delta_seconds;
                if fade.elapsed >= fade.duration {
                    layer.fade = None;
                }
            }
            if layer.fade.is_none()
                && let Some(current) = layer.current.clone()
            {
                let state_time = layer.player.as_ref().map_or(0.0, Player::time);
                let mut selected = None;
                for transition in &layer.transitions {
                    if transition
                        .from
                        .as_ref()
                        .is_some_and(|from| from != &current)
                    {
                        continue;
                    }
                    if transition.to == current {
                        continue;
                    }
                    if transition.condition.matches(parameters, state_time) {
                        transition.condition.triggers(&mut used_triggers);
                        selected = Some(transition.clone());
                        break;
                    }
                }
                if let Some(transition) = selected {
                    let target = layer
                        .states
                        .get(&transition.to)
                        .ok_or(AnimationError::UnknownState)?;
                    let mut target_player = Player::new(target.clip.clone(), target.loop_mode);
                    target_player.set_speed(target.speed)?;
                    let old_player = layer.player.take();
                    layer.player = Some(target_player);
                    layer.current = Some(transition.to);
                    if transition.duration > 0.0 {
                        if let Some(old_player) = old_player {
                            layer.fade = Some(Fade {
                                from: old_player,
                                elapsed: 0.0,
                                duration: transition.duration,
                            });
                        }
                    } else {
                        layer.fade = None;
                    }
                }
            }
            merge_playback(&mut result, layer_playback);
        }
        for trigger in used_triggers {
            self.clear_trigger(&trigger);
        }
        Ok(result)
    }

    /// Blended pose of all layers. Layers are combined in insertion order.
    pub fn pose(&self) -> Pose {
        let mut pose = Pose::new();
        let mut total_weight = 0.0f32;
        for layer in &self.layers {
            if layer.weight <= 0.0 || layer.player.is_none() {
                continue;
            }
            let layer_pose = layer.pose();
            if total_weight <= f32::EPSILON {
                pose = layer_pose;
                total_weight = layer.weight;
            } else {
                let weight = layer.weight / (total_weight + layer.weight);
                pose = pose.blend(&layer_pose, weight);
                total_weight += layer.weight;
            }
        }
        pose
    }

    /// Convenience for a single-property machine output.
    pub fn value(&self, property: &str) -> Option<crate::Value> {
        self.pose().get(property)
    }
}

fn make_player(layer: &Layer, name: &str) -> AnimationResult<Player> {
    let state = layer.states.get(name).ok_or(AnimationError::UnknownState)?;
    let mut player = Player::new(state.clip.clone(), state.loop_mode);
    player.set_speed(state.speed)?;
    Ok(player)
}

fn merge_playback(target: &mut Playback, source: Playback) {
    target.events.extend(source.events);
    target.wraps = target.wraps.saturating_add(source.wraps);
    target.skipped_loops = target.skipped_loops.saturating_add(source.skipped_loops);
    target.truncated |= source.truncated;
    target.events.sort_by(|left, right| {
        left.time
            .total_cmp(&right.time)
            .then_with(|| left.clip.cmp(&right.clip))
            .then_with(|| left.marker.cmp(&right.marker))
    });
}
