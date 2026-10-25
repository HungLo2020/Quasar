use crate::error::finite;
use crate::{AnimationError, AnimationResult, Pose, Value};
use std::sync::Arc;

/// Interpolation between adjacent keyframes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Interpolation {
    Step,
    Linear,
    Smooth,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Keyframe {
    pub time: f32,
    pub value: Value,
}

impl Keyframe {
    pub fn new(time: f32, value: Value) -> Self {
        Self { time, value }
    }
}

/// A named property channel.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Track {
    property: Arc<str>,
    interpolation: Interpolation,
    keyframes: Vec<Keyframe>,
}

impl Track {
    pub fn new(
        property: impl Into<String>,
        interpolation: Interpolation,
        keyframes: Vec<Keyframe>,
    ) -> AnimationResult<Self> {
        let property: Arc<str> = Arc::from(property.into());
        if property.is_empty() {
            return Err(AnimationError::EmptyTrack);
        }
        if keyframes.is_empty() {
            return Err(AnimationError::EmptyTrack);
        }
        let mut previous = None;
        let kind = keyframes[0].value;
        for keyframe in &keyframes {
            finite(keyframe.time)?;
            if keyframe.time < 0.0 {
                return Err(AnimationError::InvalidKeyframeOrder);
            }
            if !keyframe.value.is_finite() {
                return Err(AnimationError::NonFinite);
            }
            if !keyframe.value.same_kind(kind) {
                return Err(AnimationError::ValueTypeMismatch);
            }
            if previous.is_some_and(|time| keyframe.time <= time) {
                return Err(AnimationError::InvalidKeyframeOrder);
            }
            previous = Some(keyframe.time);
        }
        if matches!(kind, Value::Bool(_) | Value::U32(_)) && interpolation != Interpolation::Step {
            return Err(AnimationError::UnsupportedInterpolation);
        }
        Ok(Self {
            property,
            interpolation,
            keyframes,
        })
    }

    pub fn property(&self) -> &str {
        &self.property
    }

    pub fn interpolation(&self) -> Interpolation {
        self.interpolation
    }

    pub fn keyframes(&self) -> &[Keyframe] {
        &self.keyframes
    }

    pub fn sample(&self, time: f32) -> Value {
        let first = self.keyframes[0];
        let last = self.keyframes[self.keyframes.len() - 1];
        if time <= first.time {
            return first.value;
        }
        if time >= last.time {
            return last.value;
        }
        let right = self
            .keyframes
            .partition_point(|keyframe| keyframe.time <= time);
        if right < self.keyframes.len() && self.keyframes[right].time == time {
            return self.keyframes[right].value;
        }
        let left = right - 1;
        let left_key = self.keyframes[left];
        let right_key = self.keyframes[right];
        let span = right_key.time - left_key.time;
        if span <= f32::EPSILON {
            return right_key.value;
        }
        let mut t = (time - left_key.time) / span;
        if self.interpolation == Interpolation::Smooth {
            t = t * t * (3.0 - 2.0 * t);
        }
        if self.interpolation == Interpolation::Step {
            return left_key.value;
        }
        left_key
            .value
            .lerp(right_key.value, t)
            .unwrap_or(right_key.value)
    }
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Marker {
    pub time: f32,
    pub name: String,
}

impl Marker {
    pub fn new(time: f32, name: impl Into<String>) -> Self {
        Self {
            time,
            name: name.into(),
        }
    }
}

/// Immutable animation timeline shared by players and state definitions.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Clip {
    name: String,
    duration: f32,
    tracks: Vec<Track>,
    markers: Vec<Marker>,
}

impl Clip {
    pub fn new(
        name: impl Into<String>,
        duration: f32,
        mut tracks: Vec<Track>,
        mut markers: Vec<Marker>,
    ) -> AnimationResult<Self> {
        let name = name.into();
        finite(duration)?;
        if duration <= 0.0 {
            return Err(AnimationError::InvalidDuration);
        }
        if name.is_empty() || (tracks.is_empty() && markers.is_empty()) {
            return Err(AnimationError::EmptyClip);
        }
        let mut properties = std::collections::BTreeSet::new();
        for track in &tracks {
            if !properties.insert(track.property.to_string()) {
                return Err(AnimationError::DuplicateProperty);
            }
            if track
                .keyframes
                .last()
                .is_some_and(|keyframe| keyframe.time > duration)
            {
                return Err(AnimationError::InvalidKeyframeOrder);
            }
        }
        for marker in &markers {
            finite(marker.time)?;
            if marker.time < 0.0 || marker.time > duration || marker.name.is_empty() {
                return Err(AnimationError::InvalidMarker);
            }
        }
        markers.sort_by(|left, right| {
            left.time
                .total_cmp(&right.time)
                .then_with(|| left.name.cmp(&right.name))
        });
        tracks.sort_by(|left, right| left.property.cmp(&right.property));
        Ok(Self {
            name,
            duration,
            tracks,
            markers,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn duration(&self) -> f32 {
        self.duration
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn markers(&self) -> &[Marker] {
        &self.markers
    }

    /// Samples all tracks at `time`, clamped to the clip range.
    pub fn sample(&self, time: f32) -> Pose {
        let time = time.clamp(0.0, self.duration);
        let mut pose = Pose::new();
        for track in &self.tracks {
            pose.insert_shared(track.property.clone(), track.sample(time));
        }
        pose
    }

    /// Samples into a caller-owned pose, reusing its key allocations. Stale
    /// properties from a different clip are retained; use a fresh pose when
    /// property sets differ.
    pub fn sample_into(&self, time: f32, pose: &mut Pose) {
        let time = time.clamp(0.0, self.duration);
        if pose.fill_from_tracks(&self.tracks, time) {
            return;
        }
        for track in &self.tracks {
            pose.insert_shared(track.property.clone(), track.sample(time));
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LoopMode {
    #[default]
    Once,
    Loop,
    PingPong,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackEvent {
    pub clip: String,
    pub marker: String,
    pub time: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Playback {
    pub events: Vec<PlaybackEvent>,
    /// Number of end/start boundaries crossed this advance.
    pub wraps: u64,
    /// Full cycles whose markers were intentionally not enumerated for a very
    /// large timestep.
    pub skipped_loops: u64,
    /// Ping-pong segment expansion stopped at its safety cap.
    pub truncated: bool,
}

const PINGPONG_SEGMENT_LIMIT: usize = 1024;

/// Caller-owned playback cursor.
#[derive(Clone, Debug)]
pub struct Player {
    clip: Arc<Clip>,
    time: f32,
    speed: f32,
    loop_mode: LoopMode,
    playing: bool,
    direction: f32,
}

impl Player {
    pub fn new(clip: Arc<Clip>, loop_mode: LoopMode) -> Self {
        Self {
            clip,
            time: 0.0,
            speed: 1.0,
            loop_mode,
            playing: true,
            direction: 1.0,
        }
    }

    pub fn clip(&self) -> &Arc<Clip> {
        &self.clip
    }

    pub fn replace_clip(&mut self, clip: Arc<Clip>, loop_mode: LoopMode) {
        self.clip = clip;
        self.loop_mode = loop_mode;
        self.time = 0.0;
        self.direction = 1.0;
    }

    pub fn time(&self) -> f32 {
        self.time
    }

    pub fn speed(&self) -> f32 {
        self.speed
    }

    pub fn set_speed(&mut self, speed: f32) -> AnimationResult<()> {
        self.speed = finite(speed)?;
        if self.time == 0.0 {
            self.direction = if self.speed < 0.0 { -1.0 } else { 1.0 };
        }
        Ok(())
    }

    pub fn loop_mode(&self) -> LoopMode {
        self.loop_mode
    }

    pub fn set_loop_mode(&mut self, loop_mode: LoopMode) {
        self.loop_mode = loop_mode;
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn play(&mut self) {
        self.playing = true;
    }

    pub fn pause(&mut self) {
        self.playing = false;
    }

    pub fn reset(&mut self) {
        self.time = 0.0;
        self.direction = if self.speed < 0.0 { -1.0 } else { 1.0 };
        self.playing = true;
    }

    pub fn seek(&mut self, time: f32) -> AnimationResult<()> {
        finite(time)?;
        self.time = match self.loop_mode {
            LoopMode::Once => time.clamp(0.0, self.clip.duration),
            LoopMode::Loop => time.rem_euclid(self.clip.duration),
            LoopMode::PingPong => {
                let duration = self.clip.duration;
                let phase = time.rem_euclid(duration * 2.0);
                if phase <= duration {
                    phase
                } else {
                    duration * 2.0 - phase
                }
            }
        };
        Ok(())
    }

    pub fn sample(&self) -> Pose {
        self.clip.sample(self.time)
    }

    pub fn sample_into(&self, pose: &mut Pose) {
        self.clip.sample_into(self.time, pose);
    }

    /// Advances by `delta_seconds` using current speed and loop mode.
    pub fn advance(&mut self, delta_seconds: f32) -> AnimationResult<Playback> {
        finite(delta_seconds)?;
        if delta_seconds < 0.0 {
            return Err(AnimationError::InvalidDuration);
        }
        if !self.playing || self.speed == 0.0 || delta_seconds == 0.0 {
            return Ok(Playback::default());
        }
        let mut playback = Playback::default();
        match self.loop_mode {
            LoopMode::Once => self.advance_once(delta_seconds, &mut playback),
            LoopMode::Loop => self.advance_loop(delta_seconds, &mut playback),
            LoopMode::PingPong => self.advance_pingpong(delta_seconds, &mut playback),
        }
        Ok(playback)
    }

    fn advance_once(&mut self, delta_seconds: f32, playback: &mut Playback) {
        let delta = delta_seconds * self.speed;
        if delta > 0.0 {
            let next = (self.time + delta).min(self.clip.duration);
            collect_forward(&self.clip, self.time, next, false, true, playback);
            self.time = next;
            if next >= self.clip.duration {
                self.playing = false;
            }
        } else {
            let next = (self.time + delta).max(0.0);
            collect_backward(&self.clip, next, self.time, true, false, playback);
            self.time = next;
            if next <= 0.0 {
                self.playing = false;
            }
        }
    }

    fn advance_loop(&mut self, delta_seconds: f32, playback: &mut Playback) {
        let delta = delta_seconds * self.speed;
        let duration = self.clip.duration;
        if delta > 0.0 {
            let total = f64::from(self.time) + f64::from(delta);
            let duration_f = f64::from(duration);
            if total < duration_f {
                collect_forward(&self.clip, self.time, total as f32, false, true, playback);
                self.time = total as f32;
            } else {
                collect_forward(&self.clip, self.time, duration, false, true, playback);
                let remaining = total - duration_f;
                let wraps_f = (remaining / duration_f).floor();
                let wraps = if wraps_f >= u64::MAX as f64 {
                    u64::MAX
                } else {
                    wraps_f as u64 + 1
                };
                let partial = (remaining % duration_f) as f32;
                collect_forward(&self.clip, 0.0, partial, true, true, playback);
                playback.wraps = wraps;
                playback.skipped_loops = wraps.saturating_sub(1);
                self.time = partial;
            }
        } else {
            let total = f64::from(self.time) + f64::from(delta);
            let duration_f = f64::from(duration);
            if total >= 0.0 {
                collect_backward(&self.clip, total as f32, self.time, true, false, playback);
                self.time = total as f32;
            } else {
                collect_backward(&self.clip, 0.0, self.time, true, false, playback);
                let remaining = -total;
                let wraps_f = (remaining / duration_f).floor();
                let wraps = if wraps_f >= u64::MAX as f64 {
                    u64::MAX
                } else {
                    wraps_f as u64 + 1
                };
                let traveled = (remaining % duration_f) as f32;
                let partial = (duration - traveled).max(0.0);
                collect_backward(&self.clip, partial, duration, true, true, playback);
                playback.wraps = wraps;
                playback.skipped_loops = wraps.saturating_sub(1);
                self.time = partial;
            }
        }
    }

    fn advance_pingpong(&mut self, delta_seconds: f32, playback: &mut Playback) {
        let mut remaining = delta_seconds * self.speed.abs();
        let duration = self.clip.duration;
        let mut segments = 0usize;
        while remaining > f32::EPSILON && segments < PINGPONG_SEGMENT_LIMIT {
            segments += 1;
            if self.time <= 0.0 && self.direction < 0.0 {
                self.direction = 1.0;
            }
            if self.time >= duration && self.direction > 0.0 {
                self.direction = -1.0;
            }
            let forward = self.direction > 0.0;
            if forward {
                let available = (duration - self.time).max(0.0);
                let step = remaining.min(available);
                collect_forward(
                    &self.clip,
                    self.time,
                    self.time + step,
                    false,
                    true,
                    playback,
                );
                self.time += step;
                remaining -= step;
                if self.time >= duration {
                    self.time = duration;
                    self.direction = -1.0;
                    playback.wraps += 1;
                }
            } else {
                let step = remaining.min(self.time);
                collect_backward(
                    &self.clip,
                    self.time - step,
                    self.time,
                    true,
                    false,
                    playback,
                );
                self.time -= step;
                remaining -= step;
                if self.time <= 0.0 {
                    self.time = 0.0;
                    self.direction = 1.0;
                    playback.wraps += 1;
                }
            }
        }
        if remaining > f32::EPSILON {
            playback.truncated = true;
        }
    }
}

fn collect_forward(
    clip: &Clip,
    from: f32,
    to: f32,
    include_from: bool,
    include_to: bool,
    playback: &mut Playback,
) {
    if to < from {
        return;
    }
    for marker in &clip.markers {
        let after_start = if include_from {
            marker.time >= from
        } else {
            marker.time > from
        };
        let before_end = if include_to {
            marker.time <= to
        } else {
            marker.time < to
        };
        if after_start && before_end {
            playback.events.push(PlaybackEvent {
                clip: clip.name.clone(),
                marker: marker.name.clone(),
                time: marker.time,
            });
        }
    }
}

fn collect_backward(
    clip: &Clip,
    from: f32,
    to: f32,
    include_from: bool,
    include_to: bool,
    playback: &mut Playback,
) {
    if from > to {
        return;
    }
    for marker in &clip.markers {
        let after_start = if include_from {
            marker.time >= from
        } else {
            marker.time > from
        };
        let before_end = if include_to {
            marker.time <= to
        } else {
            marker.time < to
        };
        if after_start && before_end {
            playback.events.push(PlaybackEvent {
                clip: clip.name.clone(),
                marker: marker.name.clone(),
                time: marker.time,
            });
        }
    }
}
