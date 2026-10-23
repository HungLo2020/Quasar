//! Deterministic mixer, voices, buses, scheduling and voice admission.
//!
//! The same mixer drives native output and deterministic offline rendering.
//! A voice owns immutable `Arc` PCM or a procedural generator; the mixing
//! thread never decodes files or allocates for normal playback.

use crate::decode::{Clip, MAX_CHANNELS, MAX_SAMPLE_RATE, MIN_SAMPLE_RATE};
use crate::error::{AudioError, AudioResult};
use crate::spatial::{self, Listener, SpatialConfig, SpatialModelKind};
use crate::stream::{StreamReader, StreamSource};
use hound::{WavSpec, WavWriter};
use std::{
    fmt,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
};

/// Absolute mixer frame counter. Frames advance only while the mixer is not
/// globally paused, so scheduled events remain attached to simulation time.
pub type Frame = u64;

/// Bus/category identity. `MASTER` is always present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BusId(pub u16);
impl BusId {
    /// The implicit master bus created by every mixer.
    pub const MASTER: Self = Self(0);
}

/// Optional voice-admission group, independent of mixing buses.
///
/// Games can use groups for per-entity or per-subsystem voice limits while
/// still sharing one bus, or for a limit that spans several buses. Groups have
/// no gain, mute or spatial state; use a [`BusId`] for mixing controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VoiceGroup(pub u32);

/// Maximum number of distinct voice groups a mixer can track without
/// allocating during admission.
pub const MAX_VOICE_GROUPS: usize = 64;

/// One independently controlled bus.
#[derive(Debug, Clone)]
pub struct BusConfig {
    /// Human-readable name (not used for matching).
    pub name: Arc<str>,
    /// Linear bus gain.
    pub volume: f32,
    /// Mutes this bus without changing its stored volume.
    pub muted: bool,
    /// Maximum simultaneous voices on this bus; `0` means the global limit.
    pub max_voices: usize,
}

/// Mixer format, capacity and diagnostic policy.
#[derive(Debug, Clone)]
pub struct MixerConfig {
    /// Output sample rate.
    pub sample_rate: u32,
    /// Output channel count (1 or 2).
    pub channels: u16,
    /// Maximum simultaneous voices across all buses.
    pub max_voices: usize,
    /// Reserved command capacity for the native backend.
    pub command_capacity: usize,
}
impl Default for MixerConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 2,
            max_voices: 64,
            command_capacity: 256,
        }
    }
}
impl MixerConfig {
    fn validate(&self) -> AudioResult<()> {
        if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&self.sample_rate) {
            return Err(AudioError::InvalidArgument(format!(
                "mixer sample rate {} is outside {MIN_SAMPLE_RATE}..={MAX_SAMPLE_RATE}",
                self.sample_rate
            )));
        }
        if !(1..=MAX_CHANNELS).contains(&self.channels) {
            return Err(AudioError::InvalidArgument(
                "mixer supports mono or stereo output".into(),
            ));
        }
        if self.max_voices == 0 || self.max_voices > 65_536 {
            return Err(AudioError::InvalidArgument(
                "max_voices must be 1..=65536".into(),
            ));
        }
        if self.command_capacity == 0 {
            return Err(AudioError::InvalidArgument(
                "command_capacity must be positive".into(),
            ));
        }
        Ok(())
    }
}

/// A sample-frame loop region, end-exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopRegion {
    /// First frame played after the loop point.
    pub start: Frame,
    /// End frame (exclusive).
    pub end: Frame,
}
impl LoopRegion {
    /// Creates a validated region.
    pub fn new(start: Frame, end: Frame) -> AudioResult<Self> {
        if start >= end {
            return Err(AudioError::InvalidArgument(
                "loop region must have start < end".into(),
            ));
        }
        Ok(Self { start, end })
    }
    fn span(self) -> Frame {
        self.end - self.start
    }
}

/// Loop behavior for a voice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoopMode {
    /// Stop at the end.
    #[default]
    None,
    /// Restart at frame zero.
    Whole,
    /// Play the intro once, then repeat a region exactly.
    Region(LoopRegion),
}

/// What to do when a voice limit is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VoicePolicy {
    /// Reject the new voice.
    Reject,
    /// Stop the oldest voice in the constrained scope.
    StealOldest,
    /// Stop the lowest-priority voice, but only if the new priority is higher.
    #[default]
    StealLowestPriority,
}

/// Per-play configuration. It is copied into the voice and can be changed
/// later through the handle/voice setters.
#[derive(Debug, Clone)]
pub struct PlaybackConfig {
    /// Destination bus.
    pub bus: BusId,
    /// Linear voice gain (0..=4).
    pub volume: f32,
    /// Playback pitch multiplier.
    pub pitch: f32,
    /// Additional playback-speed multiplier, useful for game-specific timing.
    pub speed: f32,
    /// Stereo pan in `[-1, 1]`.
    pub pan: f32,
    /// Loop policy.
    pub looping: LoopMode,
    /// Admission priority; higher wins when stealing.
    pub priority: i32,
    /// Optional admission group for a game-owned voice limit.
    pub group: Option<VoiceGroup>,
    /// Limit policy for this request.
    pub policy: VoicePolicy,
    /// Optional spatial emitter configuration.
    pub spatial: Option<SpatialConfig>,
}
impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            bus: BusId::MASTER,
            volume: 1.0,
            pitch: 1.0,
            speed: 1.0,
            pan: 0.0,
            looping: LoopMode::None,
            priority: 0,
            group: None,
            policy: VoicePolicy::StealLowestPriority,
            spatial: None,
        }
    }
}

/// Runtime voice status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VoiceStatus {
    /// Waiting for its scheduled frame.
    Scheduled = 0,
    /// Producing samples.
    Playing = 1,
    /// Paused by the caller or by global pause.
    Paused = 2,
    /// Explicitly stopped.
    Stopped = 3,
    /// Reached the end of its source.
    Finished = 4,
}
impl VoiceStatus {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Scheduled,
            1 => Self::Playing,
            2 => Self::Paused,
            3 => Self::Stopped,
            _ => Self::Finished,
        }
    }
    /// Whether the voice can no longer produce samples.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Stopped | Self::Finished)
    }
}

#[derive(Debug)]
struct VoiceState {
    status: AtomicU8,
    position: AtomicU64,
}
/// Stable playback handle. It remains valid after the voice stops so callers
/// can observe terminal status; it never aliases a later voice.
#[derive(Clone)]
pub struct VoiceHandle {
    id: u64,
    state: Arc<VoiceState>,
    token: Arc<()>,
}
impl fmt::Debug for VoiceHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VoiceHandle")
            .field("id", &self.id)
            .field("status", &self.status())
            .finish()
    }
}
impl VoiceHandle {
    pub(crate) fn new(id: u64, token: Arc<()>) -> Self {
        Self {
            id,
            state: Arc::new(VoiceState {
                status: AtomicU8::new(VoiceStatus::Scheduled as u8),
                position: AtomicU64::new(0),
            }),
            token,
        }
    }
    fn belongs_to(&self, token: &Arc<()>) -> bool {
        Arc::ptr_eq(&self.token, token)
    }
    /// Monotonic voice identity.
    pub fn id(&self) -> u64 {
        self.id
    }
    /// Current status.
    pub fn status(&self) -> VoiceStatus {
        VoiceStatus::from_u8(self.state.status.load(Ordering::Acquire))
    }
    /// Whether the voice is still scheduled, playing or paused.
    pub fn is_active(&self) -> bool {
        !self.status().is_terminal()
    }
    /// Source position in frames, or 0 while scheduled.
    pub fn position_frames(&self) -> Frame {
        self.state.position.load(Ordering::Acquire)
    }
    pub(crate) fn set_status(&self, status: VoiceStatus) {
        self.state.status.store(status as u8, Ordering::Release);
    }
    fn set_position(&self, position: Frame) {
        self.state.position.store(position, Ordering::Release);
    }
}

/// A source that generates one frame at a time on the mixing thread.
///
/// Implementations must be real-time safe: no I/O, filesystem access,
/// blocking locks or unbounded allocation in `next_frame`.
pub trait ProceduralSource: Send {
    /// Source sample rate. It must equal the mixer's sample rate; procedural
    /// voices are called once per output frame. Render to a [`Clip`] when
    /// resampling, pitch or speed adjustment is required.
    fn sample_rate(&self) -> u32;
    /// 1 or 2 channels.
    fn channels(&self) -> u16;
    /// Fills `output` with one frame and returns false when finished.
    fn next_frame(&mut self, output: &mut [f32]) -> bool;
}

/// A playable source.
pub enum SoundSource {
    /// Immutable decoded PCM, shared by Arc.
    Clip(Clip),
    /// A streaming Ogg source decoded on a background thread.
    Stream(StreamSource),
    /// A real-time generator.
    Procedural(Box<dyn ProceduralSource>),
}
impl From<Clip> for SoundSource {
    fn from(clip: Clip) -> Self {
        Self::Clip(clip)
    }
}
/// A source whose decoder thread (if any) has already been created on a
/// control thread. The native callback consumes this form so it never spawns
/// threads or opens files while rendering.
pub(crate) enum PreparedSource {
    Clip(Clip),
    Stream(StreamReader),
    Procedural(Box<dyn ProceduralSource>),
}
impl SoundSource {
    pub(crate) fn prepare(self, looping: LoopMode) -> AudioResult<PreparedSource> {
        Ok(match self {
            Self::Clip(clip) => PreparedSource::Clip(clip),
            Self::Stream(stream) => PreparedSource::Stream(stream.reader(looping)?),
            Self::Procedural(generator) => PreparedSource::Procedural(generator),
        })
    }
}
impl PreparedSource {
    #[cfg(feature = "native")]
    pub(crate) fn validate_prepared(
        &self,
        config: &PlaybackConfig,
        mixer_sample_rate: u32,
    ) -> AudioResult<()> {
        validate_playback_values(config)?;
        validate_source_values(config, self.kind(), mixer_sample_rate)
    }
    #[cfg(feature = "native")]
    fn kind(&self) -> SourceKind<'_> {
        match self {
            Self::Clip(clip) => SourceKind::Clip {
                frames: clip.frames(),
            },
            Self::Stream(_) => SourceKind::Stream,
            Self::Procedural(generator) => SourceKind::Procedural(generator.as_ref()),
        }
    }
}
enum SourceKind<'a> {
    Clip { frames: u64 },
    Stream,
    Procedural(&'a dyn ProceduralSource),
}
impl fmt::Debug for SoundSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Clip(clip) => f.debug_tuple("Clip").field(clip).finish(),
            Self::Stream(stream) => f.debug_tuple("Stream").field(&stream.label()).finish(),
            Self::Procedural(_) => f.write_str("Procedural(..)"),
        }
    }
}

enum VoiceSource {
    Clip(Clip),
    Stream(StreamReader),
    Procedural(Box<dyn ProceduralSource>),
}
struct Voice {
    handle: VoiceHandle,
    source: VoiceSource,
    config: PlaybackConfig,
    start_frame: Frame,
    position: f64,
    started: bool,
    paused: bool,
    finished: bool,
    sequence: u64,
}
struct Bus {
    id: BusId,
    config: BusConfig,
}

/// Snapshot diagnostics for tests, CLI tooling and status overlays.
#[derive(Debug, Clone, Copy, Default)]
pub struct MixerDiagnostics {
    /// Voices currently scheduled, playing or paused.
    pub active_voices: usize,
    /// Voices waiting for their start frame.
    pub scheduled_voices: usize,
    /// Cumulative voices started.
    pub voices_started: u64,
    /// Cumulative voices finished naturally.
    pub voices_finished: u64,
    /// Cumulative voices stopped by explicit control.
    pub voices_stopped: u64,
    /// Cumulative voices stolen by limits.
    pub voices_stolen: u64,
    /// Scheduled requests that arrived after their requested frame.
    pub late_starts: u64,
    /// Output frames where a streaming voice had no decoded data.
    pub underruns: u64,
    /// Background stream decode failures.
    pub decode_errors: u64,
    /// Frames whose mixed magnitude exceeded 1.0 and were clamped.
    pub clipped_frames: u64,
    /// Current mixer frame.
    pub frame: Frame,
    /// Peak absolute sample since the last peak reset.
    pub peak: f32,
}

/// Deterministic audio mixer. It has no device dependency and can render to
/// PCM/WAV for tests, headless replay and deterministic verification.
pub struct Mixer {
    config: MixerConfig,
    buses: Vec<Bus>,
    master_volume: f32,
    muted: bool,
    paused: bool,
    listener: Listener,
    frame: Frame,
    token: Arc<()>,
    voices: Vec<Voice>,
    group_limits: Vec<(VoiceGroup, usize)>,
    next_id: u64,
    next_bus: u16,
    diagnostics: MixerDiagnostics,
}
impl Mixer {
    /// Creates a mixer with a master bus.
    pub fn new(config: MixerConfig) -> AudioResult<Self> {
        Self::new_with_token(config, Arc::new(()))
    }

    /// Creates a mixer that shares voice-handle identity with a native engine.
    pub(crate) fn new_with_token(config: MixerConfig, token: Arc<()>) -> AudioResult<Self> {
        config.validate()?;
        let mut mixer = Self {
            config: config.clone(),
            buses: Vec::with_capacity(32),
            master_volume: 1.0,
            muted: false,
            paused: false,
            listener: Listener::default(),
            frame: 0,
            token,
            voices: Vec::with_capacity(config.max_voices),
            group_limits: Vec::with_capacity(MAX_VOICE_GROUPS),
            next_id: 1,
            next_bus: 1,
            diagnostics: MixerDiagnostics::default(),
        };
        mixer.buses.push(Bus {
            id: BusId::MASTER,
            config: BusConfig {
                name: Arc::from("master"),
                volume: 1.0,
                muted: false,
                // The global limit already applies; 0 avoids double-counting.
                max_voices: 0,
            },
        });
        Ok(mixer)
    }

    /// Creates a named bus. The returned ID is stable for this mixer.
    pub fn create_bus(
        &mut self,
        name: impl Into<Arc<str>>,
        volume: f32,
        max_voices: usize,
    ) -> AudioResult<BusId> {
        validate_gain(volume)?;
        if self.next_bus == u16::MAX {
            return Err(AudioError::Internal("bus identity exhausted".into()));
        }
        let id = BusId(self.next_bus);
        self.next_bus += 1;
        self.buses.push(Bus {
            id,
            config: BusConfig {
                name: name.into(),
                volume,
                muted: false,
                max_voices,
            },
        });
        Ok(id)
    }
    /// Returns a bus configuration.
    pub fn bus(&self, id: BusId) -> Option<&BusConfig> {
        self.buses
            .iter()
            .find(|bus| bus.id == id)
            .map(|bus| &bus.config)
    }
    /// Returns every bus with its identity, including the master bus.
    pub fn buses(&self) -> impl Iterator<Item = (BusId, &BusConfig)> {
        self.buses.iter().map(|bus| (bus.id, &bus.config))
    }
    /// Finds a bus by its caller-assigned name. Names are not required to be
    /// unique; the first match is returned.
    pub fn bus_id(&self, name: &str) -> Option<BusId> {
        self.buses
            .iter()
            .find(|bus| bus.config.name.as_ref() == name)
            .map(|bus| bus.id)
    }
    /// Changes bus volume.
    pub fn set_bus_volume(&mut self, id: BusId, volume: f32) -> AudioResult<()> {
        validate_gain(volume)?;
        self.bus_mut(id)?.volume = volume;
        Ok(())
    }
    /// Mutes/unmutes a bus.
    pub fn set_bus_muted(&mut self, id: BusId, muted: bool) -> AudioResult<()> {
        self.bus_mut(id)?.muted = muted;
        Ok(())
    }
    /// Changes the per-bus voice limit. `0` means the global mixer limit.
    pub fn set_bus_max_voices(&mut self, id: BusId, max_voices: usize) -> AudioResult<()> {
        self.bus_mut(id)?.max_voices = max_voices;
        Ok(())
    }
    /// Sets a voice limit for an admission group. `0` removes the limit.
    ///
    /// A mixer tracks at most [`MAX_VOICE_GROUPS`] distinct groups so ordinary
    /// admission never grows this table. Configuring more returns an explicit
    /// error rather than allocating without bound.
    pub fn set_group_limit(&mut self, group: VoiceGroup, max_voices: usize) -> AudioResult<()> {
        if let Some(entry) = self
            .group_limits
            .iter_mut()
            .find(|(candidate, _)| *candidate == group)
        {
            entry.1 = max_voices;
            return Ok(());
        }
        if self.group_limits.len() >= MAX_VOICE_GROUPS {
            return Err(AudioError::Unsupported(format!(
                "mixer tracks at most {MAX_VOICE_GROUPS} voice groups"
            )));
        }
        self.group_limits.push((group, max_voices));
        Ok(())
    }
    /// Returns a group's configured limit, or `None` when it is untracked.
    pub fn group_limit(&self, group: VoiceGroup) -> Option<usize> {
        self.group_limits
            .iter()
            .find(|(candidate, _)| *candidate == group)
            .map(|(_, limit)| *limit)
    }
    /// Removes a non-master bus and stops its voices.
    pub fn remove_bus(&mut self, id: BusId) -> AudioResult<()> {
        if id == BusId::MASTER {
            return Err(AudioError::InvalidArgument(
                "the master bus cannot be removed".into(),
            ));
        }
        let index = self
            .buses
            .iter()
            .position(|bus| bus.id == id)
            .ok_or(AudioError::NotFound)?;
        self.buses.remove(index);
        for voice in &mut self.voices {
            if voice.config.bus == id {
                voice.handle.set_status(VoiceStatus::Stopped);
                voice.finished = true;
                self.diagnostics.voices_stopped = self.diagnostics.voices_stopped.saturating_add(1);
            }
        }
        self.voices.retain(|voice| !voice.finished);
        Ok(())
    }
    /// Adds a bus with a caller-assigned identity (used by the native command bridge).
    #[cfg(feature = "native")]
    pub(crate) fn add_bus(
        &mut self,
        id: BusId,
        name: impl Into<Arc<str>>,
        volume: f32,
        max_voices: usize,
    ) -> AudioResult<()> {
        validate_gain(volume)?;
        if id == BusId::MASTER || self.buses.iter().any(|bus| bus.id == id) {
            return Err(AudioError::InvalidArgument(
                "bus identity already exists".into(),
            ));
        }
        self.buses.push(Bus {
            id,
            config: BusConfig {
                name: name.into(),
                volume,
                muted: false,
                max_voices,
            },
        });
        Ok(())
    }
    fn bus_mut(&mut self, id: BusId) -> AudioResult<&mut BusConfig> {
        self.buses
            .iter_mut()
            .find(|bus| bus.id == id)
            .map(|bus| &mut bus.config)
            .ok_or(AudioError::NotFound)
    }
    /// Master gain.
    pub fn master_volume(&self) -> f32 {
        self.master_volume
    }
    /// Sets master gain (0..=4).
    pub fn set_master_volume(&mut self, volume: f32) -> AudioResult<()> {
        validate_gain(volume)?;
        self.master_volume = volume;
        Ok(())
    }
    /// Global mute.
    pub fn muted(&self) -> bool {
        self.muted
    }
    /// Sets global mute.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }
    /// Global pause freezes voice positions and scheduled starts but the
    /// native device continues consuming silence.
    pub fn paused(&self) -> bool {
        self.paused
    }
    /// Sets global pause.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        if paused {
            for voice in &mut self.voices {
                voice.handle.set_status(VoiceStatus::Paused);
            }
        } else {
            for voice in &mut self.voices {
                if !voice.paused {
                    voice.handle.set_status(if voice.started {
                        VoiceStatus::Playing
                    } else {
                        VoiceStatus::Scheduled
                    });
                }
            }
        }
    }
    /// Listener transform used by spatial voices.
    pub fn listener(&self) -> Listener {
        self.listener
    }
    /// Updates the listener transform.
    pub fn set_listener(&mut self, listener: Listener) -> AudioResult<()> {
        validate_listener(&listener)?;
        self.listener = listener;
        Ok(())
    }
    /// Mixer configuration.
    pub fn config(&self) -> &MixerConfig {
        &self.config
    }
    /// Output sample rate in Hz.
    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate
    }
    /// Output channel count (1 or 2).
    pub fn channels(&self) -> u16 {
        self.config.channels
    }
    /// Current mixer frame.
    pub fn frame(&self) -> Frame {
        self.frame
    }
    /// Number of live voices.
    pub fn active_voices(&self) -> usize {
        self.voices.len()
    }
    /// Current diagnostics snapshot.
    pub fn diagnostics(&self) -> MixerDiagnostics {
        MixerDiagnostics {
            active_voices: self.voices.len(),
            scheduled_voices: self.voices.iter().filter(|voice| !voice.started).count(),
            frame: self.frame,
            ..self.diagnostics
        }
    }
    /// Clears the peak diagnostic.
    pub fn reset_peak(&mut self) {
        self.diagnostics.peak = 0.0;
    }

    /// Plays immediately at the next rendered frame.
    pub fn play(
        &mut self,
        source: impl Into<SoundSource>,
        config: PlaybackConfig,
    ) -> AudioResult<VoiceHandle> {
        let start = self.frame;
        self.play_at(source, config, start)
    }

    /// Starts a streaming Ogg source immediately.
    pub fn play_stream(
        &mut self,
        stream: StreamSource,
        config: PlaybackConfig,
    ) -> AudioResult<VoiceHandle> {
        self.play_at(SoundSource::Stream(stream), config, self.frame)
    }

    /// Schedules a voice at an absolute mixer frame. A frame in the past starts
    /// at the next rendered frame and increments the late-start diagnostic.
    pub fn play_at(
        &mut self,
        source: impl Into<SoundSource>,
        config: PlaybackConfig,
        start_frame: Frame,
    ) -> AudioResult<VoiceHandle> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| AudioError::Internal("voice identity exhausted".into()))?;
        let handle = VoiceHandle::new(id, self.token.clone());
        self.play_at_with_handle(source, config, start_frame, handle.clone())?;
        Ok(handle)
    }

    pub(crate) fn play_at_with_handle(
        &mut self,
        source: impl Into<SoundSource>,
        config: PlaybackConfig,
        start_frame: Frame,
        handle: VoiceHandle,
    ) -> AudioResult<()> {
        let source = source.into();
        self.validate_playback(&config, &source)?;
        let prepared = source.prepare(config.looping)?;
        self.play_prepared_validated(prepared, config, start_frame, handle)
    }

    /// Plays a source prepared on the control thread. The native command
    /// callback uses this path so streaming decoder threads are never spawned
    /// from the audio callback.
    #[cfg(feature = "native")]
    pub(crate) fn play_prepared_at(
        &mut self,
        source: PreparedSource,
        config: PlaybackConfig,
        start_frame: Frame,
        handle: VoiceHandle,
    ) -> AudioResult<()> {
        self.validate_config(&config)?;
        self.validate_source(&config, source.kind())?;
        self.play_prepared_validated(source, config, start_frame, handle)
    }

    fn play_prepared_validated(
        &mut self,
        source: PreparedSource,
        config: PlaybackConfig,
        start_frame: Frame,
        handle: VoiceHandle,
    ) -> AudioResult<()> {
        let actual_start = start_frame.max(self.frame);
        if start_frame < self.frame {
            self.diagnostics.late_starts = self.diagnostics.late_starts.saturating_add(1);
        }
        self.admit(config.bus, config.group, config.policy, config.priority)?;
        let id = handle.id;
        let started = actual_start <= self.frame;
        handle.set_status(if started {
            VoiceStatus::Playing
        } else {
            VoiceStatus::Scheduled
        });
        if started {
            self.diagnostics.voices_started = self.diagnostics.voices_started.saturating_add(1);
        }
        let voice_source = match source {
            PreparedSource::Clip(clip) => VoiceSource::Clip(clip),
            PreparedSource::Stream(reader) => VoiceSource::Stream(reader),
            PreparedSource::Procedural(generator) => VoiceSource::Procedural(generator),
        };
        self.voices.push(Voice {
            handle,
            source: voice_source,
            config,
            start_frame: actual_start,
            position: 0.0,
            started,
            paused: false,
            finished: false,
            sequence: id,
        });
        Ok(())
    }

    fn validate_playback(&self, config: &PlaybackConfig, source: &SoundSource) -> AudioResult<()> {
        let kind = match source {
            SoundSource::Clip(clip) => SourceKind::Clip {
                frames: clip.frames(),
            },
            SoundSource::Stream(_) => SourceKind::Stream,
            SoundSource::Procedural(generator) => SourceKind::Procedural(generator.as_ref()),
        };
        self.validate_config(config)?;
        self.validate_source(config, kind)
    }

    fn validate_config(&self, config: &PlaybackConfig) -> AudioResult<()> {
        self.bus(config.bus).ok_or(AudioError::NotFound)?;
        validate_playback_values(config)
    }

    fn validate_source(&self, config: &PlaybackConfig, source: SourceKind<'_>) -> AudioResult<()> {
        validate_source_values(config, source, self.config.sample_rate)
    }

    fn admit(
        &mut self,
        bus: BusId,
        group: Option<VoiceGroup>,
        policy: VoicePolicy,
        priority: i32,
    ) -> AudioResult<()> {
        if self.voices.len() >= self.config.max_voices {
            self.steal(policy, priority, None, None)?;
        }
        let bus_limit = self
            .buses
            .iter()
            .find(|candidate| candidate.id == bus)
            .map(|candidate| candidate.config.max_voices)
            .unwrap_or(usize::MAX);
        let bus_voices = self
            .voices
            .iter()
            .filter(|voice| voice.config.bus == bus)
            .count();
        if bus_limit != 0 && bus_voices >= bus_limit {
            self.steal(policy, priority, Some(bus), None)?;
        }
        if let Some(group) = group {
            let group_limit = self
                .group_limits
                .iter()
                .find(|(candidate, _)| *candidate == group)
                .map(|(_, limit)| *limit)
                .unwrap_or(0);
            let group_voices = self
                .voices
                .iter()
                .filter(|voice| voice.config.group == Some(group))
                .count();
            if group_limit != 0 && group_voices >= group_limit {
                self.steal(policy, priority, None, Some(group))?;
            }
        }
        Ok(())
    }
    fn steal(
        &mut self,
        policy: VoicePolicy,
        priority: i32,
        bus_scope: Option<BusId>,
        group_scope: Option<VoiceGroup>,
    ) -> AudioResult<()> {
        if policy == VoicePolicy::Reject {
            return Err(AudioError::VoiceLimit);
        }
        let mut victim: Option<(usize, i32, u64)> = None;
        for (index, voice) in self.voices.iter().enumerate() {
            if let Some(bus) = bus_scope
                && voice.config.bus != bus
            {
                continue;
            }
            if let Some(group) = group_scope
                && voice.config.group != Some(group)
            {
                continue;
            }
            let candidate = (index, voice.config.priority, voice.sequence);
            let replace = match victim {
                None => true,
                Some((_, old_priority, old_sequence)) => match policy {
                    VoicePolicy::StealOldest => candidate.2 < old_sequence,
                    _ => {
                        candidate.1 < old_priority
                            || (candidate.1 == old_priority && candidate.2 < old_sequence)
                    }
                },
            };
            if replace {
                victim = Some(candidate);
            }
        }
        let Some((index, victim_priority, _)) = victim else {
            return Err(AudioError::VoiceLimit);
        };
        if policy == VoicePolicy::StealLowestPriority && priority <= victim_priority {
            return Err(AudioError::VoiceLimit);
        }
        let voice = self.voices.remove(index);
        voice.handle.set_status(VoiceStatus::Stopped);
        self.diagnostics.voices_stolen = self.diagnostics.voices_stolen.saturating_add(1);
        Ok(())
    }

    /// Stops a voice. Returns false for an unknown or already terminal voice.
    pub fn stop(&mut self, handle: &VoiceHandle) -> bool {
        if !handle.belongs_to(&self.token) {
            return false;
        }
        let Some(index) = self
            .voices
            .iter()
            .position(|voice| voice.handle.id == handle.id)
        else {
            return false;
        };
        let voice = self.voices.remove(index);
        voice.handle.set_status(VoiceStatus::Stopped);
        self.diagnostics.voices_stopped = self.diagnostics.voices_stopped.saturating_add(1);
        true
    }
    /// Stops every voice.
    pub fn stop_all(&mut self) {
        for voice in &self.voices {
            voice.handle.set_status(VoiceStatus::Stopped);
            self.diagnostics.voices_stopped = self.diagnostics.voices_stopped.saturating_add(1);
        }
        self.voices.clear();
    }
    /// Stops every voice on a bus.
    pub fn stop_bus(&mut self, bus: BusId) {
        for voice in &self.voices {
            if voice.config.bus == bus {
                voice.handle.set_status(VoiceStatus::Stopped);
                self.diagnostics.voices_stopped = self.diagnostics.voices_stopped.saturating_add(1);
            }
        }
        self.voices.retain(|voice| voice.config.bus != bus);
    }
    /// Pauses/resumes one voice.
    pub fn set_voice_paused(&mut self, handle: &VoiceHandle, paused: bool) -> bool {
        if !handle.belongs_to(&self.token) {
            return false;
        }
        let Some(voice) = self
            .voices
            .iter_mut()
            .find(|voice| voice.handle.id == handle.id)
        else {
            return false;
        };
        voice.paused = paused;
        voice.handle.set_status(if paused {
            VoiceStatus::Paused
        } else if voice.started {
            VoiceStatus::Playing
        } else {
            VoiceStatus::Scheduled
        });
        true
    }
    /// Changes voice gain.
    pub fn set_voice_volume(&mut self, handle: &VoiceHandle, volume: f32) -> AudioResult<()> {
        if !handle.belongs_to(&self.token) {
            return Err(AudioError::NotFound);
        }
        validate_gain(volume)?;
        let voice = self
            .voices
            .iter_mut()
            .find(|voice| voice.handle.id == handle.id)
            .ok_or(AudioError::NotFound)?;
        voice.config.volume = volume;
        Ok(())
    }
    /// Changes voice pitch.
    pub fn set_voice_pitch(&mut self, handle: &VoiceHandle, pitch: f32) -> AudioResult<()> {
        if !handle.belongs_to(&self.token) {
            return Err(AudioError::NotFound);
        }
        validate_multiplier(pitch)?;
        let voice = self
            .voices
            .iter_mut()
            .find(|voice| voice.handle.id == handle.id)
            .ok_or(AudioError::NotFound)?;
        voice.config.pitch = pitch;
        Ok(())
    }
    /// Changes voice playback speed.
    pub fn set_voice_speed(&mut self, handle: &VoiceHandle, speed: f32) -> AudioResult<()> {
        if !handle.belongs_to(&self.token) {
            return Err(AudioError::NotFound);
        }
        validate_multiplier(speed)?;
        let voice = self
            .voices
            .iter_mut()
            .find(|voice| voice.handle.id == handle.id)
            .ok_or(AudioError::NotFound)?;
        voice.config.speed = speed;
        Ok(())
    }
    /// Changes voice pan.
    pub fn set_voice_pan(&mut self, handle: &VoiceHandle, pan: f32) -> AudioResult<()> {
        if !handle.belongs_to(&self.token) {
            return Err(AudioError::NotFound);
        }
        validate_pan(pan)?;
        let voice = self
            .voices
            .iter_mut()
            .find(|voice| voice.handle.id == handle.id)
            .ok_or(AudioError::NotFound)?;
        voice.config.pan = pan;
        Ok(())
    }
    /// Changes spatial placement.
    pub fn set_voice_spatial(
        &mut self,
        handle: &VoiceHandle,
        spatial: Option<SpatialConfig>,
    ) -> AudioResult<()> {
        if !handle.belongs_to(&self.token) {
            return Err(AudioError::NotFound);
        }
        if let Some(spatial) = &spatial {
            validate_spatial(spatial)?;
        }
        let voice = self
            .voices
            .iter_mut()
            .find(|voice| voice.handle.id == handle.id)
            .ok_or(AudioError::NotFound)?;
        voice.config.spatial = spatial;
        Ok(())
    }
    /// Seeks a clip voice to a source frame.
    pub fn seek(&mut self, handle: &VoiceHandle, frame: Frame) -> AudioResult<()> {
        if !handle.belongs_to(&self.token) {
            return Err(AudioError::NotFound);
        }
        let voice = self
            .voices
            .iter_mut()
            .find(|voice| voice.handle.id == handle.id)
            .ok_or(AudioError::NotFound)?;
        let frames = match &voice.source {
            VoiceSource::Clip(clip) => clip.frames(),
            VoiceSource::Stream(_) => {
                return Err(AudioError::Unsupported(
                    "streaming voices cannot seek; recreate the stream".into(),
                ));
            }
            VoiceSource::Procedural(_) => {
                return Err(AudioError::Unsupported(
                    "procedural voices cannot seek".into(),
                ));
            }
        };
        if frame >= frames {
            return Err(AudioError::InvalidArgument(
                "seek frame is outside the source".into(),
            ));
        }
        voice.position = frame as f64;
        voice.handle.set_position(frame);
        voice.handle.set_status(if voice.paused {
            VoiceStatus::Paused
        } else if voice.started {
            VoiceStatus::Playing
        } else {
            VoiceStatus::Scheduled
        });
        Ok(())
    }
    /// Restarts a clip voice from frame zero. Streaming and procedural voices
    /// cannot be rewound in place; create a new voice for those sources.
    pub fn restart(&mut self, handle: &VoiceHandle) -> AudioResult<()> {
        if !handle.belongs_to(&self.token) {
            return Err(AudioError::NotFound);
        }
        let voice = self
            .voices
            .iter_mut()
            .find(|voice| voice.handle.id == handle.id)
            .ok_or(AudioError::NotFound)?;
        match &voice.source {
            VoiceSource::Clip(_) => {}
            VoiceSource::Stream(_) => {
                return Err(AudioError::Unsupported(
                    "streaming voices cannot restart; create a new StreamSource voice".into(),
                ));
            }
            VoiceSource::Procedural(_) => {
                return Err(AudioError::Unsupported(
                    "procedural voices cannot restart; create a new generator voice".into(),
                ));
            }
        }
        voice.position = 0.0;
        voice.handle.set_position(0);
        voice.handle.set_status(if voice.paused {
            VoiceStatus::Paused
        } else if voice.started {
            VoiceStatus::Playing
        } else {
            VoiceStatus::Scheduled
        });
        Ok(())
    }

    /// Renders one output frame into `output` (length = channels).
    pub fn next_frame(&mut self, output: &mut [f32]) -> AudioResult<()> {
        let channels = self.config.channels as usize;
        if output.len() != channels {
            return Err(AudioError::InvalidArgument(
                "output frame length must equal mixer channels".into(),
            ));
        }
        output.fill(0.0);
        if self.paused {
            return Ok(());
        }
        let mut frame = [0.0f32; 2];
        for voice in &mut self.voices {
            if !voice.started {
                if voice.start_frame > self.frame {
                    continue;
                }
                voice.started = true;
                voice.handle.set_status(if voice.paused {
                    VoiceStatus::Paused
                } else {
                    VoiceStatus::Playing
                });
                self.diagnostics.voices_started = self.diagnostics.voices_started.saturating_add(1);
            }
            if voice.paused {
                continue;
            }
            let mut local = [0.0f32; 2];
            let (sample_channels, done, underrun) =
                Self::mix_voice(voice, self.config.sample_rate, &mut local);
            if underrun {
                self.diagnostics.underruns = self.diagnostics.underruns.saturating_add(1);
            }
            if done {
                voice.finished = true;
            } else {
                let bus = self.buses.iter().find(|bus| bus.id == voice.config.bus);
                let (bus_gain, bus_muted) = bus
                    .map(|bus| (bus.config.volume, bus.config.muted))
                    .unwrap_or((1.0, false));
                if bus_muted || self.muted {
                    continue;
                }
                let mut gain = voice.config.volume * bus_gain * self.master_volume;
                let mut pan = voice.config.pan;
                if let Some(spatial) = &voice.config.spatial {
                    let evaluated = spatial::evaluate(
                        &self.listener,
                        spatial.position,
                        &spatial.model,
                        spatial.pan_strength,
                    );
                    gain *= evaluated.gain;
                    pan = (pan + evaluated.pan).clamp(-1.0, 1.0);
                }
                gain = gain.clamp(0.0, 4.0);
                // Mono output cannot represent stereo position; keep the
                // voice's loudness instead of halving it through pan gains.
                let (left_pan, right_pan) = if channels == 2 {
                    pan_gains(pan)
                } else {
                    (1.0, 1.0)
                };
                if sample_channels == 1 {
                    frame[0] += local[0] * gain * left_pan;
                    frame[1] += local[0] * gain * right_pan;
                } else {
                    frame[0] += local[0] * gain * left_pan;
                    frame[1] += local[1] * gain * right_pan;
                }
            }
        }
        self.voices.retain(|voice| {
            if voice.finished {
                voice.handle.set_status(VoiceStatus::Finished);
                self.diagnostics.voices_finished =
                    self.diagnostics.voices_finished.saturating_add(1);
                if matches!(&voice.source, VoiceSource::Stream(reader) if reader.error().is_some())
                {
                    self.diagnostics.decode_errors =
                        self.diagnostics.decode_errors.saturating_add(1);
                }
                false
            } else {
                true
            }
        });
        let clipped = frame[0].abs() > 1.0 || frame[1].abs() > 1.0;
        output[0] = frame[0].clamp(-1.0, 1.0);
        if channels == 2 {
            output[1] = frame[1].clamp(-1.0, 1.0);
        } else {
            output[0] = (frame[0] + frame[1]).clamp(-1.0, 1.0) * 0.5;
        }
        let peak = output
            .iter()
            .fold(0.0f32, |peak, value| peak.max(value.abs()));
        self.diagnostics.peak = self.diagnostics.peak.max(peak);
        if clipped {
            self.diagnostics.clipped_frames = self.diagnostics.clipped_frames.saturating_add(1);
        }
        self.frame += 1;
        Ok(())
    }

    fn mix_voice(voice: &mut Voice, sample_rate: u32, local: &mut [f32; 2]) -> (usize, bool, bool) {
        match &mut voice.source {
            VoiceSource::Clip(clip) => {
                let frames = clip.frames();
                if frames == 0 {
                    return (1, true, false);
                }
                match voice.config.looping {
                    LoopMode::Region(region) if voice.position >= region.end as f64 => {
                        let span = region.span() as f64;
                        voice.position = region.start as f64
                            + (voice.position - region.end as f64).rem_euclid(span);
                    }
                    LoopMode::Whole if voice.position >= frames as f64 => {
                        voice.position %= frames as f64;
                    }
                    LoopMode::None if voice.position >= frames as f64 => {
                        return (1, true, false);
                    }
                    _ => {}
                }
                let index = voice.position.floor() as u64;
                let fraction = voice.position - index as f64;
                let next = match voice.config.looping {
                    LoopMode::None => (index + 1).min(frames.saturating_sub(1)),
                    LoopMode::Whole => (index + 1) % frames,
                    LoopMode::Region(region) => {
                        if index + 1 >= region.end {
                            region.start
                        } else {
                            index + 1
                        }
                    }
                };
                let left = lerp(clip.sample_at(index, 0), clip.sample_at(next, 0), fraction);
                let right = if clip.channels() == 2 {
                    lerp(clip.sample_at(index, 1), clip.sample_at(next, 1), fraction)
                } else {
                    left
                };
                local[0] = left;
                local[1] = right;
                let step = f64::from(clip.sample_rate()) / f64::from(sample_rate)
                    * f64::from(voice.config.pitch)
                    * f64::from(voice.config.speed);
                voice.position += step;
                voice.handle.set_position(voice.position.max(0.0) as u64);
                if matches!(voice.config.looping, LoopMode::None) && voice.position >= frames as f64
                {
                    voice.finished = true;
                }
                (clip.channels() as usize, false, false)
            }
            VoiceSource::Stream(reader) => {
                let step = f64::from(reader.sample_rate()) / f64::from(sample_rate)
                    * f64::from(voice.config.pitch)
                    * f64::from(voice.config.speed);
                let (frame, underrun) = reader.sample(voice.position);
                if let Some(frame) = frame {
                    local[0] = finite_or_zero(frame[0]);
                    local[1] = finite_or_zero(frame[1]);
                    voice.position += step;
                    voice.handle.set_position(voice.position.max(0.0) as u64);
                    (2, false, underrun)
                } else if reader.error().is_some() {
                    (2, true, underrun)
                } else if underrun {
                    (2, false, true)
                } else {
                    (2, true, false)
                }
            }
            VoiceSource::Procedural(generator) => {
                let channels = generator.channels() as usize;
                let mut buffer = [0.0f32; 2];
                if !generator.next_frame(&mut buffer[..channels]) {
                    return (channels, true, false);
                }
                local[0] = finite_or_zero(buffer[0]).clamp(-1.0, 1.0);
                local[1] = if channels == 2 {
                    finite_or_zero(buffer[1]).clamp(-1.0, 1.0)
                } else {
                    local[0]
                };
                voice.handle.set_position(voice.position as u64);
                voice.position += 1.0;
                (channels, false, false)
            }
        }
    }

    /// Renders interleaved frames into `output`.
    pub fn render_interleaved(&mut self, frames: usize, output: &mut [f32]) -> AudioResult<()> {
        let channels = self.config.channels as usize;
        let expected = frames
            .checked_mul(channels)
            .ok_or_else(|| AudioError::InvalidArgument("frame count overflow".into()))?;
        if output.len() != expected {
            return Err(AudioError::InvalidArgument(
                "output length must be frames * channels".into(),
            ));
        }
        for frame in output.chunks_exact_mut(channels) {
            self.next_frame(frame)?;
        }
        Ok(())
    }
    /// Convenience allocation for tests and offline tools.
    pub fn render(&mut self, frames: usize) -> AudioResult<Vec<f32>> {
        let samples = frames
            .checked_mul(self.config.channels as usize)
            .ok_or_else(|| AudioError::InvalidArgument("frame count overflow".into()))?;
        let mut output = vec![0.0; samples];
        self.render_interleaved(frames, &mut output)?;
        Ok(output)
    }
    /// Renders PCM16 WAV output in bounded chunks.
    pub fn render_wav(&mut self, path: impl AsRef<Path>, frames: usize) -> AudioResult<()> {
        let spec = WavSpec {
            channels: self.config.channels,
            sample_rate: self.config.sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = WavWriter::create(path, spec)?;
        let channels = self.config.channels as usize;
        let chunk_frames = 4096usize.min(frames.max(1));
        let mut buffer = vec![0.0f32; chunk_frames * channels];
        let mut remaining = frames;
        while remaining > 0 {
            let count = remaining.min(chunk_frames);
            let slice = &mut buffer[..count * channels];
            self.render_interleaved(count, slice)?;
            for sample in slice.iter() {
                writer.write_sample((sample.clamp(-1.0, 1.0) * 32767.0).round() as i16)?;
            }
            remaining -= count;
        }
        writer.finalize()?;
        Ok(())
    }
}

pub(crate) fn validate_playback_values(config: &PlaybackConfig) -> AudioResult<()> {
    validate_gain(config.volume)?;
    validate_multiplier(config.pitch)?;
    validate_multiplier(config.speed)?;
    validate_pan(config.pan)?;
    if let Some(spatial) = &config.spatial {
        validate_spatial(spatial)?;
    }
    Ok(())
}

fn validate_source_values(
    config: &PlaybackConfig,
    source: SourceKind<'_>,
    mixer_sample_rate: u32,
) -> AudioResult<()> {
    match source {
        SourceKind::Stream => {
            if let LoopMode::Region(region) = config.looping
                && region.start >= region.end
            {
                return Err(AudioError::InvalidArgument(
                    "stream loop region must have start < end".into(),
                ));
            }
        }
        SourceKind::Clip { frames } => match config.looping {
            LoopMode::None | LoopMode::Whole => {}
            LoopMode::Region(region) => {
                if region.end > frames || region.start >= region.end {
                    return Err(AudioError::InvalidArgument(
                        "loop region is outside the clip".into(),
                    ));
                }
            }
        },
        SourceKind::Procedural(generator) => {
            if !matches!(config.looping, LoopMode::None) {
                return Err(AudioError::Unsupported(
                    "procedural voices do not support looping; use a Clip".into(),
                ));
            }
            if config.pitch != 1.0 || config.speed != 1.0 {
                return Err(AudioError::Unsupported(
                    "procedural voices run at the mixer rate; use a Clip for pitch/speed".into(),
                ));
            }
            if !(1..=MAX_CHANNELS).contains(&generator.channels()) {
                return Err(AudioError::Unsupported(
                    "procedural source must be mono or stereo".into(),
                ));
            }
            if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&generator.sample_rate()) {
                return Err(AudioError::Unsupported(
                    "procedural source sample rate is unsupported".into(),
                ));
            }
            if generator.sample_rate() != mixer_sample_rate {
                return Err(AudioError::Unsupported(format!(
                    "procedural source runs at {} Hz but the mixer runs at {} Hz; render to a Clip for resampling",
                    generator.sample_rate(),
                    mixer_sample_rate
                )));
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_gain(value: f32) -> AudioResult<()> {
    if !value.is_finite() || !(0.0..=4.0).contains(&value) {
        return Err(AudioError::InvalidArgument(
            "gain must be finite in [0, 4]".into(),
        ));
    }
    Ok(())
}
pub(crate) fn validate_multiplier(value: f32) -> AudioResult<()> {
    if !value.is_finite() || !(0.01..=16.0).contains(&value) {
        return Err(AudioError::InvalidArgument(
            "pitch/speed must be finite in [0.01, 16]".into(),
        ));
    }
    Ok(())
}
pub(crate) fn validate_pan(value: f32) -> AudioResult<()> {
    if !value.is_finite() || !(-1.0..=1.0).contains(&value) {
        return Err(AudioError::InvalidArgument(
            "pan must be finite in [-1, 1]".into(),
        ));
    }
    Ok(())
}
pub(crate) fn validate_spatial(spatial: &SpatialConfig) -> AudioResult<()> {
    if !spatial.position.iter().all(|value| value.is_finite()) {
        return Err(AudioError::InvalidArgument(
            "spatial position must be finite".into(),
        ));
    }
    if !spatial.pan_strength.is_finite() || !(-1.0..=1.0).contains(&spatial.pan_strength) {
        return Err(AudioError::InvalidArgument(
            "spatial pan strength must be finite in [-1, 1]".into(),
        ));
    }
    let finite_nonnegative = |value: f32| value.is_finite() && value >= 0.0;
    match &spatial.model {
        SpatialModelKind::Linear {
            min_distance,
            max_distance,
        } => {
            if !finite_nonnegative(*min_distance)
                || !finite_nonnegative(*max_distance)
                || *max_distance < *min_distance
            {
                return Err(AudioError::InvalidArgument(
                    "linear distance range must be finite with 0 <= min <= max".into(),
                ));
            }
        }
        SpatialModelKind::Inverse {
            min_distance,
            max_distance,
            rolloff,
        }
        | SpatialModelKind::Exponential {
            min_distance,
            max_distance,
            rolloff,
        } => {
            if !finite_nonnegative(*min_distance)
                || *min_distance <= 0.0
                || !finite_nonnegative(*max_distance)
                || *max_distance < *min_distance
                || !finite_nonnegative(*rolloff)
            {
                return Err(AudioError::InvalidArgument(
                    "inverse/exponential range requires 0 < min <= max and rolloff >= 0".into(),
                ));
            }
        }
        SpatialModelKind::None | SpatialModelKind::Custom(_) => {}
    }
    Ok(())
}
pub(crate) fn validate_listener(listener: &Listener) -> AudioResult<()> {
    if !listener.position.iter().all(|value| value.is_finite())
        || !listener.forward.iter().all(|value| value.is_finite())
        || !listener.up.iter().all(|value| value.is_finite())
    {
        return Err(AudioError::InvalidArgument(
            "listener position/orientation must be finite".into(),
        ));
    }
    let forward = spatial::length(listener.forward);
    let up = spatial::length(listener.up);
    if forward <= f32::EPSILON || up <= f32::EPSILON {
        return Err(AudioError::InvalidArgument(
            "listener forward and up vectors must be non-zero".into(),
        ));
    }
    Ok(())
}
fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}
fn lerp(a: f32, b: f32, fraction: f64) -> f32 {
    a + (b - a) * fraction as f32
}
fn pan_gains(pan: f32) -> (f32, f32) {
    let pan = pan.clamp(-1.0, 1.0);
    if pan > 0.0 {
        (1.0 - pan, 1.0)
    } else if pan < 0.0 {
        (1.0, 1.0 + pan)
    } else {
        (1.0, 1.0)
    }
}
