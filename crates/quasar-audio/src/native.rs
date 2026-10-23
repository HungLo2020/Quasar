//! Optional real-time backend using CPAL.
//!
//! The audio callback owns the mixer, drains a bounded lock-free command queue,
//! renders exact frames and updates atomics for the control thread. It never
//! opens files, decodes, allocates command storage or waits on a mutex.

use crate::error::{AudioError, AudioResult};
use crate::mixer::{
    BusConfig, BusId, Frame, Mixer, MixerConfig, MixerDiagnostics, PlaybackConfig, PreparedSource,
    SoundSource, VoiceGroup, VoiceHandle, VoiceStatus, validate_gain, validate_listener,
    validate_multiplier, validate_pan, validate_spatial,
};
use crate::spatial::{Listener, SpatialConfig};
use crate::stream::StreamSource;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender, TrySendError};
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
};

/// Native backend configuration. `None` sample rate/channels select the device
/// default; requests that cannot be matched fail rather than silently changing
/// the requested format.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Requested output sample rate.
    pub sample_rate: Option<u32>,
    /// Requested output channels (1 or 2).
    pub channels: Option<u16>,
    /// Maximum simultaneous voices.
    pub max_voices: usize,
    /// Bounded control queue capacity.
    pub command_capacity: usize,
    /// Requested device buffer frames; `None` uses the backend default.
    pub buffer_frames: Option<u32>,
}
impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            sample_rate: None,
            channels: None,
            max_voices: 64,
            command_capacity: 256,
            buffer_frames: None,
        }
    }
}

/// A selectable output device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Device name as reported by the platform.
    pub name: String,
    /// Whether this is the host default output.
    pub is_default: bool,
}

/// Native diagnostics. `mixer` is the same data available offline.
#[derive(Debug, Clone)]
pub struct EngineDiagnostics {
    /// Mixer state and counters.
    pub mixer: MixerDiagnostics,
    /// Pending control commands.
    pub queue_occupancy: usize,
    /// Commands rejected by the callback (invalid arguments or limits).
    pub command_errors: u64,
    /// Set when the backend reports a stream/device failure.
    pub device_lost: bool,
    /// Last backend error text, if any.
    pub error: Option<String>,
    /// Negotiated output sample rate.
    pub sample_rate: u32,
    /// Negotiated output channels.
    pub channels: u16,
    /// Configured device buffer frames, when reported.
    pub buffer_frames: Option<u32>,
    /// Capacity of the bounded control queue.
    pub queue_capacity: usize,
}
impl EngineDiagnostics {
    /// Approximate device-side buffering in frames. Real presentation latency
    /// also includes the OS, driver and compositor; this is not input-to-photon
    /// accuracy.
    pub fn buffered_frames(&self) -> u32 {
        self.buffer_frames.unwrap_or(0)
    }
    /// Approximate device buffering in seconds. `None` when the backend did not
    /// report a buffer size; it is not a measured round-trip latency.
    pub fn buffered_seconds(&self) -> Option<f64> {
        if self.sample_rate == 0 {
            return None;
        }
        self.buffer_frames
            .map(|frames| f64::from(frames) / f64::from(self.sample_rate))
    }
}

struct Shared {
    frame: AtomicU64,
    active: AtomicU64,
    scheduled: AtomicU64,
    started: AtomicU64,
    finished: AtomicU64,
    stopped: AtomicU64,
    stolen: AtomicU64,
    late: AtomicU64,
    underruns: AtomicU64,
    decode_errors: AtomicU64,
    clipped: AtomicU64,
    peak_bits: AtomicU32,
    command_errors: AtomicU64,
    device_lost: AtomicBool,
    error: Mutex<Option<String>>,
    sample_rate: u32,
    channels: u16,
    buffer_frames: Option<u32>,
}
impl Shared {
    fn publish(&self, mixer: &Mixer) {
        let d = mixer.diagnostics();
        self.frame.store(d.frame, Ordering::Relaxed);
        self.active.store(d.active_voices as u64, Ordering::Relaxed);
        self.scheduled
            .store(d.scheduled_voices as u64, Ordering::Relaxed);
        self.started.store(d.voices_started, Ordering::Relaxed);
        self.finished.store(d.voices_finished, Ordering::Relaxed);
        self.stopped.store(d.voices_stopped, Ordering::Relaxed);
        self.stolen.store(d.voices_stolen, Ordering::Relaxed);
        self.late.store(d.late_starts, Ordering::Relaxed);
        self.underruns.store(d.underruns, Ordering::Relaxed);
        self.decode_errors.store(d.decode_errors, Ordering::Relaxed);
        self.clipped.store(d.clipped_frames, Ordering::Relaxed);
        self.peak_bits.store(d.peak.to_bits(), Ordering::Relaxed);
    }
    fn diagnostics(&self, queue_occupancy: usize, queue_capacity: usize) -> EngineDiagnostics {
        EngineDiagnostics {
            mixer: MixerDiagnostics {
                active_voices: self.active.load(Ordering::Relaxed) as usize,
                scheduled_voices: self.scheduled.load(Ordering::Relaxed) as usize,
                voices_started: self.started.load(Ordering::Relaxed),
                voices_finished: self.finished.load(Ordering::Relaxed),
                voices_stopped: self.stopped.load(Ordering::Relaxed),
                voices_stolen: self.stolen.load(Ordering::Relaxed),
                late_starts: self.late.load(Ordering::Relaxed),
                underruns: self.underruns.load(Ordering::Relaxed),
                decode_errors: self.decode_errors.load(Ordering::Relaxed),
                clipped_frames: self.clipped.load(Ordering::Relaxed),
                frame: self.frame.load(Ordering::Relaxed),
                peak: f32::from_bits(self.peak_bits.load(Ordering::Relaxed)),
            },
            queue_occupancy,
            command_errors: self.command_errors.load(Ordering::Relaxed),
            device_lost: self.device_lost.load(Ordering::Relaxed),
            error: self
                .error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
            sample_rate: self.sample_rate,
            channels: self.channels,
            buffer_frames: self.buffer_frames,
            queue_capacity,
        }
    }
}

enum Command {
    Play {
        handle: VoiceHandle,
        source: PreparedSource,
        config: PlaybackConfig,
        start: Frame,
    },
    Stop(VoiceHandle),
    Pause(VoiceHandle, bool),
    Volume(VoiceHandle, f32),
    Pitch(VoiceHandle, f32),
    Speed(VoiceHandle, f32),
    Pan(VoiceHandle, f32),
    Spatial(VoiceHandle, Option<SpatialConfig>),
    Seek(VoiceHandle, Frame),
    Restart(VoiceHandle),
    StopAll,
    StopBus(BusId),
    AddBus(BusId, BusConfig),
    BusVolume(BusId, f32),
    BusMuted(BusId, bool),
    BusLimit(BusId, usize),
    GroupLimit(VoiceGroup, usize),
    RemoveBus(BusId),
    Master(f32),
    Muted(bool),
    Paused(bool),
    Listener(Listener),
    Shutdown,
}

/// Real-time audio engine. It owns a bounded command queue and a dedicated
/// device-owner thread; ordinary control calls are non-blocking (`try_send`).
///
/// CPAL streams are deliberately created and destroyed on one thread because
/// some backends mark them `!Send`. The handle itself is `Send + Sync`, so a
/// game can share it or send it to a control thread. Dropping the handle joins
/// the device thread after the stream has stopped.
pub struct AudioEngine {
    commands: Sender<Command>,
    shared: Arc<Shared>,
    device_name: Option<String>,
    config: EngineConfig,
    next_bus: AtomicU32,
    next_voice: AtomicU64,
    token: Arc<()>,
    buses: Mutex<BTreeSet<BusId>>,
    shutdown: Option<Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl AudioEngine {
    /// Lists output devices for the default host.
    pub fn devices() -> AudioResult<Vec<DeviceInfo>> {
        let host = cpal::default_host();
        let default = host
            .default_output_device()
            .and_then(|device| device.name().ok());
        let devices = host
            .output_devices()
            .map_err(|error| AudioError::Device(error.to_string()))?;
        let mut output = Vec::new();
        for device in devices {
            let name = device
                .name()
                .map_err(|error| AudioError::Device(error.to_string()))?;
            output.push(DeviceInfo {
                is_default: default.as_deref() == Some(name.as_str()),
                name,
            });
        }
        Ok(output)
    }
    /// Opens the default output or a named device.
    ///
    /// All CPAL objects are created on a dedicated device thread. If opening
    /// fails, the error is returned from this call and no engine is produced.
    pub fn open(device_name: Option<&str>, config: EngineConfig) -> AudioResult<Self> {
        validate_engine_config(&config)?;
        let (sender, receiver) = crossbeam_channel::bounded(config.command_capacity);
        let (startup_sender, startup_receiver) = crossbeam_channel::bounded(1);
        let (shutdown_sender, shutdown_receiver) = crossbeam_channel::bounded(1);
        let thread_config = config.clone();
        let token = Arc::new(());
        let thread_token = token.clone();
        let requested_name = device_name.map(str::to_owned);
        let thread = std::thread::Builder::new()
            .name("quasar-audio-device".into())
            .spawn(move || {
                match open_device(
                    requested_name.as_deref(),
                    &thread_config,
                    receiver,
                    thread_token,
                ) {
                    Ok((stream, shared, name)) => {
                        if startup_sender.send(Ok((shared, name))).is_err() {
                            return;
                        }
                        // Keep the stream and its callback alive on this
                        // thread until the handle is dropped.
                        let _ = shutdown_receiver.recv();
                        drop(stream);
                    }
                    Err(error) => {
                        let _ = startup_sender.send(Err(error));
                    }
                }
            })
            .map_err(|error| AudioError::Device(format!("spawn audio device thread: {error}")))?;
        let (shared, name) = match startup_receiver.recv() {
            Ok(Ok(startup)) => startup,
            Ok(Err(error)) => {
                let _ = thread.join();
                return Err(error);
            }
            Err(_) => {
                let _ = thread.join();
                return Err(AudioError::Device(
                    "audio device thread exited during initialization".into(),
                ));
            }
        };
        Ok(Self {
            commands: sender,
            shared,
            device_name: name,
            config,
            next_bus: AtomicU32::new(1),
            next_voice: AtomicU64::new(1),
            token,
            buses: Mutex::new(BTreeSet::from([BusId::MASTER])),
            shutdown: Some(shutdown_sender),
            thread: Some(thread),
        })
    }
    /// Opens the default device, returning `None` when no device is available.
    pub fn try_open(config: EngineConfig) -> Option<Self> {
        Self::open(None, config).ok()
    }
    /// Selected device name, if the backend exposed one.
    pub fn device_name(&self) -> Option<&str> {
        self.device_name.as_deref()
    }
    /// Plays immediately.
    pub fn play(
        &self,
        source: impl Into<SoundSource>,
        config: PlaybackConfig,
    ) -> AudioResult<VoiceHandle> {
        let frame = self.shared.frame.load(Ordering::Acquire);
        self.play_at(source, config, frame)
    }
    /// Schedules at an absolute mixer frame.
    pub fn play_at(
        &self,
        source: impl Into<SoundSource>,
        config: PlaybackConfig,
        start: Frame,
    ) -> AudioResult<VoiceHandle> {
        if !self.bus_exists(config.bus) {
            return Err(AudioError::NotFound);
        }
        let handle = VoiceHandle::new(
            self.next_voice.fetch_add(1, Ordering::Relaxed),
            self.token.clone(),
        );
        let source = source.into();
        // Create any decoder thread on the caller, not in the audio callback.
        let source = source.prepare(config.looping)?;
        // Reject invalid gain/pan/spatial/loop/source settings synchronously.
        source.validate_prepared(&config, self.shared.sample_rate)?;
        let command = Command::Play {
            handle: handle.clone(),
            source,
            config,
            start,
        };
        if let Err(TrySendError::Full(_)) = self.commands.try_send(command) {
            handle.set_status(VoiceStatus::Stopped);
            return Err(AudioError::QueueFull);
        }
        Ok(handle)
    }
    /// Starts a streaming Ogg source immediately.
    pub fn play_stream(
        &self,
        stream: StreamSource,
        config: PlaybackConfig,
    ) -> AudioResult<VoiceHandle> {
        self.play(SoundSource::Stream(stream), config)
    }
    /// Stops a voice.
    pub fn stop(&self, handle: &VoiceHandle) -> AudioResult<()> {
        self.send(Command::Stop(handle.clone()))
    }
    /// Pauses or resumes a voice.
    pub fn set_voice_paused(&self, handle: &VoiceHandle, paused: bool) -> AudioResult<()> {
        self.send(Command::Pause(handle.clone(), paused))
    }
    /// Sets voice gain.
    pub fn set_voice_volume(&self, handle: &VoiceHandle, volume: f32) -> AudioResult<()> {
        validate_gain(volume)?;
        self.send(Command::Volume(handle.clone(), volume))
    }
    /// Sets voice pitch.
    pub fn set_voice_pitch(&self, handle: &VoiceHandle, pitch: f32) -> AudioResult<()> {
        validate_multiplier(pitch)?;
        self.send(Command::Pitch(handle.clone(), pitch))
    }
    /// Sets voice speed.
    pub fn set_voice_speed(&self, handle: &VoiceHandle, speed: f32) -> AudioResult<()> {
        validate_multiplier(speed)?;
        self.send(Command::Speed(handle.clone(), speed))
    }
    /// Sets voice pan.
    pub fn set_voice_pan(&self, handle: &VoiceHandle, pan: f32) -> AudioResult<()> {
        validate_pan(pan)?;
        self.send(Command::Pan(handle.clone(), pan))
    }
    /// Updates voice spatial placement.
    pub fn set_voice_spatial(
        &self,
        handle: &VoiceHandle,
        spatial: Option<SpatialConfig>,
    ) -> AudioResult<()> {
        if let Some(spatial) = &spatial {
            validate_spatial(spatial)?;
        }
        self.send(Command::Spatial(handle.clone(), spatial))
    }
    /// Seeks a clip voice.
    pub fn seek(&self, handle: &VoiceHandle, frame: Frame) -> AudioResult<()> {
        self.send(Command::Seek(handle.clone(), frame))
    }
    /// Restarts a voice.
    pub fn restart(&self, handle: &VoiceHandle) -> AudioResult<()> {
        self.send(Command::Restart(handle.clone()))
    }
    /// Stops all voices.
    pub fn stop_all(&self) -> AudioResult<()> {
        self.send(Command::StopAll)
    }
    /// Stops a bus.
    pub fn stop_bus(&self, bus: BusId) -> AudioResult<()> {
        self.send(Command::StopBus(bus))
    }
    /// Creates a bus with a stable producer-assigned identity.
    pub fn create_bus(
        &self,
        name: impl Into<Arc<str>>,
        volume: f32,
        max_voices: usize,
    ) -> AudioResult<BusId> {
        validate_gain(volume)?;
        let id = self.next_bus.fetch_add(1, Ordering::Relaxed);
        let id = u16::try_from(id)
            .map(BusId)
            .map_err(|_| AudioError::Internal("bus identity exhausted".into()))?;
        self.send(Command::AddBus(
            id,
            BusConfig {
                name: name.into(),
                volume,
                muted: false,
                max_voices,
            },
        ))?;
        let mut buses = self
            .buses
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        buses.insert(id);
        Ok(id)
    }
    /// Changes bus volume.
    pub fn set_bus_volume(&self, bus: BusId, volume: f32) -> AudioResult<()> {
        validate_gain(volume)?;
        if !self.bus_exists(bus) {
            return Err(AudioError::NotFound);
        }
        self.send(Command::BusVolume(bus, volume))
    }
    /// Mutes a bus.
    pub fn set_bus_muted(&self, bus: BusId, muted: bool) -> AudioResult<()> {
        if !self.bus_exists(bus) {
            return Err(AudioError::NotFound);
        }
        self.send(Command::BusMuted(bus, muted))
    }
    /// Changes a per-bus voice limit. `0` uses the global limit.
    pub fn set_bus_max_voices(&self, bus: BusId, max_voices: usize) -> AudioResult<()> {
        if !self.bus_exists(bus) {
            return Err(AudioError::NotFound);
        }
        self.send(Command::BusLimit(bus, max_voices))
    }
    /// Sets or replaces a game-owned voice-group limit. `0` removes the limit.
    pub fn set_group_limit(&self, group: VoiceGroup, max_voices: usize) -> AudioResult<()> {
        self.send(Command::GroupLimit(group, max_voices))
    }
    /// Removes a non-master bus. Subsequent play requests targeting it fail
    /// synchronously.
    pub fn remove_bus(&self, bus: BusId) -> AudioResult<()> {
        if bus == BusId::MASTER {
            return Err(AudioError::InvalidArgument(
                "the master bus cannot be removed".into(),
            ));
        }
        if !self.bus_exists(bus) {
            return Err(AudioError::NotFound);
        }
        self.send(Command::RemoveBus(bus))?;
        let mut buses = self
            .buses
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        buses.remove(&bus);
        Ok(())
    }
    /// Whether this engine has created a bus identity. The device thread also
    /// validates the identifier when the command is executed.
    pub fn bus_exists(&self, bus: BusId) -> bool {
        self.buses
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&bus)
    }
    /// Sets master volume.
    pub fn set_master_volume(&self, volume: f32) -> AudioResult<()> {
        validate_gain(volume)?;
        self.send(Command::Master(volume))
    }
    /// Mutes/unmutes the mixer.
    pub fn set_muted(&self, muted: bool) -> AudioResult<()> {
        self.send(Command::Muted(muted))
    }
    /// Pauses/resumes the whole mixer.
    pub fn set_paused(&self, paused: bool) -> AudioResult<()> {
        self.send(Command::Paused(paused))
    }
    /// Updates the listener.
    pub fn set_listener(&self, listener: Listener) -> AudioResult<()> {
        validate_listener(&listener)?;
        self.send(Command::Listener(listener))
    }
    /// Current diagnostics.
    pub fn diagnostics(&self) -> EngineDiagnostics {
        self.shared
            .diagnostics(self.commands.len(), self.commands.capacity().unwrap_or(0))
    }
    /// Whether the device stream reported a loss/error.
    pub fn device_lost(&self) -> bool {
        self.shared.device_lost.load(Ordering::Acquire)
    }
    /// Reopens the same device and configuration. Active voices, their handles,
    /// named buses and group limits are not restored; the game must recreate
    /// them and replay from its own authoritative state. If reopening fails,
    /// this engine has already been consumed.
    pub fn reopen(self) -> AudioResult<Self> {
        let name = self.device_name.clone();
        let config = self.config.clone();
        drop(self);
        Self::open(name.as_deref(), config)
    }
    fn send(&self, command: Command) -> AudioResult<()> {
        match self.commands.try_send(command) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(AudioError::QueueFull),
            Err(TrySendError::Disconnected(_)) => {
                Err(AudioError::DeviceLost("audio callback has stopped".into()))
            }
        }
    }
}
impl Drop for AudioEngine {
    fn drop(&mut self) {
        let _ = self.commands.try_send(Command::Shutdown);
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn validate_engine_config(config: &EngineConfig) -> AudioResult<()> {
    if config.command_capacity == 0 {
        return Err(AudioError::InvalidArgument(
            "command_capacity must be positive".into(),
        ));
    }
    if config.max_voices == 0 || config.max_voices > 65_536 {
        return Err(AudioError::InvalidArgument(
            "max_voices must be 1..=65536".into(),
        ));
    }
    if let Some(channels) = config.channels
        && !(1..=2).contains(&channels)
    {
        return Err(AudioError::Unsupported(
            "native mixer supports mono or stereo output only".into(),
        ));
    }
    if let Some(rate) = config.sample_rate
        && !(crate::MIN_SAMPLE_RATE..=crate::MAX_SAMPLE_RATE).contains(&rate)
    {
        return Err(AudioError::InvalidArgument(format!(
            "sample rate {rate} Hz is outside {}..={}",
            crate::MIN_SAMPLE_RATE,
            crate::MAX_SAMPLE_RATE
        )));
    }
    if config.buffer_frames == Some(0) {
        return Err(AudioError::InvalidArgument(
            "buffer_frames must be positive when specified".into(),
        ));
    }
    Ok(())
}

fn open_device(
    device_name: Option<&str>,
    config: &EngineConfig,
    receiver: Receiver<Command>,
    token: Arc<()>,
) -> AudioResult<(cpal::Stream, Arc<Shared>, Option<String>)> {
    let host = cpal::default_host();
    let device = match device_name {
        Some(name) => host
            .output_devices()
            .map_err(|error| AudioError::Device(error.to_string()))?
            .find(|device| device.name().ok().as_deref() == Some(name))
            .ok_or(AudioError::NotFound)?,
        None => host.default_output_device().ok_or(AudioError::NoDevice)?,
    };
    let (stream_config, sample_format) = select_config(&device, config)?;
    let sample_rate = stream_config.sample_rate.0;
    let channels = stream_config.channels;
    if !(1..=2).contains(&channels) {
        return Err(AudioError::Unsupported(
            "native mixer supports mono or stereo output only".into(),
        ));
    }
    let shared = Arc::new(Shared {
        frame: AtomicU64::new(0),
        active: AtomicU64::new(0),
        scheduled: AtomicU64::new(0),
        started: AtomicU64::new(0),
        finished: AtomicU64::new(0),
        stopped: AtomicU64::new(0),
        stolen: AtomicU64::new(0),
        late: AtomicU64::new(0),
        underruns: AtomicU64::new(0),
        decode_errors: AtomicU64::new(0),
        clipped: AtomicU64::new(0),
        peak_bits: AtomicU32::new(0),
        command_errors: AtomicU64::new(0),
        device_lost: AtomicBool::new(false),
        error: Mutex::new(None),
        sample_rate,
        channels,
        buffer_frames: config.buffer_frames.or(match stream_config.buffer_size {
            cpal::BufferSize::Fixed(frames) => Some(frames),
            cpal::BufferSize::Default => None,
        }),
    });
    let mixer = Mixer::new_with_token(
        MixerConfig {
            sample_rate,
            channels,
            max_voices: config.max_voices,
            command_capacity: config.command_capacity,
        },
        token,
    )?;
    let error_shared = shared.clone();
    let error_callback = move |error: cpal::StreamError| {
        error_shared.device_lost.store(true, Ordering::Release);
        let mut slot = error_shared
            .error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *slot = Some(error.to_string());
    };
    let stream = match sample_format {
        cpal::SampleFormat::F32 => build_stream::<f32>(
            &device,
            &stream_config,
            mixer,
            receiver,
            shared.clone(),
            error_callback,
        )?,
        cpal::SampleFormat::I16 => build_stream::<i16>(
            &device,
            &stream_config,
            mixer,
            receiver,
            shared.clone(),
            error_callback,
        )?,
        cpal::SampleFormat::U16 => build_stream::<u16>(
            &device,
            &stream_config,
            mixer,
            receiver,
            shared.clone(),
            error_callback,
        )?,
        cpal::SampleFormat::I32 => build_stream::<i32>(
            &device,
            &stream_config,
            mixer,
            receiver,
            shared.clone(),
            error_callback,
        )?,
        cpal::SampleFormat::U32 => build_stream::<u32>(
            &device,
            &stream_config,
            mixer,
            receiver,
            shared.clone(),
            error_callback,
        )?,
        cpal::SampleFormat::I8 => build_stream::<i8>(
            &device,
            &stream_config,
            mixer,
            receiver,
            shared.clone(),
            error_callback,
        )?,
        cpal::SampleFormat::U8 => build_stream::<u8>(
            &device,
            &stream_config,
            mixer,
            receiver,
            shared.clone(),
            error_callback,
        )?,
        cpal::SampleFormat::F64 => build_stream::<f64>(
            &device,
            &stream_config,
            mixer,
            receiver,
            shared.clone(),
            error_callback,
        )?,
        other => {
            return Err(AudioError::Unsupported(format!(
                "CPAL sample format {other:?}"
            )));
        }
    };
    stream
        .play()
        .map_err(|error| AudioError::Device(error.to_string()))?;
    let name = device.name().ok();
    Ok((stream, shared, name))
}

fn select_config(
    device: &cpal::Device,
    config: &EngineConfig,
) -> AudioResult<(cpal::StreamConfig, cpal::SampleFormat)> {
    let default = device
        .default_output_config()
        .map_err(|error| AudioError::Device(error.to_string()))?;
    let channels = config.channels.unwrap_or_else(|| default.channels().min(2));
    if !(1..=2).contains(&channels) {
        return Err(AudioError::Unsupported(
            "native mixer supports mono or stereo output only".into(),
        ));
    }
    let rate = config.sample_rate.unwrap_or(default.sample_rate().0);
    let supported = if default.channels() == channels
        && default.sample_rate().0 == rate
        && is_supported_format(default.sample_format())
    {
        default
    } else {
        let ranges = device
            .supported_output_configs()
            .map_err(|error| AudioError::Device(error.to_string()))?;
        let mut found = None;
        for range in ranges {
            if range.channels() == channels
                && range.min_sample_rate().0 <= rate
                && rate <= range.max_sample_rate().0
                && is_supported_format(range.sample_format())
            {
                found = Some(range.with_sample_rate(cpal::SampleRate(rate)));
                break;
            }
        }
        found.ok_or_else(|| {
            AudioError::Unsupported(format!(
                "no output configuration for {channels} channels at {rate} Hz"
            ))
        })?
    };
    let sample_format = supported.sample_format();
    let mut stream_config: cpal::StreamConfig = supported.config();
    if let Some(frames) = config.buffer_frames {
        stream_config.buffer_size = cpal::BufferSize::Fixed(frames);
    }
    Ok((stream_config, sample_format))
}

fn is_supported_format(format: cpal::SampleFormat) -> bool {
    matches!(
        format,
        cpal::SampleFormat::F32
            | cpal::SampleFormat::I16
            | cpal::SampleFormat::U16
            | cpal::SampleFormat::I32
            | cpal::SampleFormat::U32
            | cpal::SampleFormat::I8
            | cpal::SampleFormat::U8
            | cpal::SampleFormat::F64
    )
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut mixer: Mixer,
    commands: Receiver<Command>,
    shared: Arc<Shared>,
    error_callback: impl FnMut(cpal::StreamError) + Send + 'static,
) -> AudioResult<cpal::Stream>
where
    T: cpal::Sample + cpal::SizedSample + cpal::FromSample<f32> + 'static,
{
    let channels = config.channels as usize;
    let callback_shared = shared.clone();
    device
        .build_output_stream::<T, _, _>(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                while let Ok(command) = commands.try_recv() {
                    if apply_command(&mut mixer, command).is_err() {
                        callback_shared
                            .command_errors
                            .fetch_add(1, Ordering::Relaxed);
                    }
                }
                if callback_shared.device_lost.load(Ordering::Acquire) {
                    data.fill(T::from_sample(0.0));
                    return;
                }
                let mut frame = [0.0f32; 2];
                let mut chunks = data.chunks_exact_mut(channels);
                for output in chunks.by_ref() {
                    let _ = mixer.next_frame(&mut frame[..channels]);
                    for (sample, target) in frame[..channels].iter().zip(output.iter_mut()) {
                        *target = T::from_sample(*sample);
                    }
                }
                chunks.into_remainder().fill(T::from_sample(0.0));
                callback_shared.publish(&mixer);
            },
            error_callback,
            None,
        )
        .map_err(|error| AudioError::Device(error.to_string()))
}

fn apply_command(mixer: &mut Mixer, command: Command) -> AudioResult<()> {
    match command {
        Command::Play {
            handle,
            source,
            config,
            start,
        } => {
            let result = mixer.play_prepared_at(source, config, start, handle.clone());
            if result.is_err() {
                handle.set_status(VoiceStatus::Stopped);
            }
            result
        }
        Command::Stop(handle) => {
            mixer.stop(&handle);
            Ok(())
        }
        Command::Pause(handle, paused) => {
            if mixer.set_voice_paused(&handle, paused) {
                Ok(())
            } else {
                Err(AudioError::NotFound)
            }
        }
        Command::Volume(handle, value) => mixer.set_voice_volume(&handle, value),
        Command::Pitch(handle, value) => mixer.set_voice_pitch(&handle, value),
        Command::Speed(handle, value) => mixer.set_voice_speed(&handle, value),
        Command::Pan(handle, value) => mixer.set_voice_pan(&handle, value),
        Command::Spatial(handle, value) => mixer.set_voice_spatial(&handle, value),
        Command::Seek(handle, frame) => mixer.seek(&handle, frame),
        Command::Restart(handle) => mixer.restart(&handle),
        Command::StopAll => {
            mixer.stop_all();
            Ok(())
        }
        Command::StopBus(bus) => {
            mixer.stop_bus(bus);
            Ok(())
        }
        Command::AddBus(id, config) => {
            mixer.add_bus(id, config.name, config.volume, config.max_voices)
        }
        Command::BusVolume(id, volume) => mixer.set_bus_volume(id, volume),
        Command::BusMuted(id, muted) => mixer.set_bus_muted(id, muted),
        Command::BusLimit(id, limit) => mixer.set_bus_max_voices(id, limit),
        Command::GroupLimit(group, limit) => mixer.set_group_limit(group, limit),
        Command::RemoveBus(id) => mixer.remove_bus(id),
        Command::Master(volume) => mixer.set_master_volume(volume),
        Command::Muted(muted) => {
            mixer.set_muted(muted);
            Ok(())
        }
        Command::Paused(paused) => {
            mixer.set_paused(paused);
            Ok(())
        }
        Command::Listener(listener) => mixer.set_listener(listener),
        Command::Shutdown => {
            mixer.stop_all();
            Ok(())
        }
    }
}
