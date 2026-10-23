//! Independent Quasar audio: decoding, deterministic mixing, buses, scheduling,
//! spatialization, procedural sources, offline PCM/WAV output and an optional
//! CPAL native backend.
//!
//! The crate has no dependency on the renderers, windowing, VulkanicGAL, an ECS,
//! a scene graph, Basalt, or an application loop. `Mixer` is deterministic and
//! device-free; `AudioEngine` is the optional real-time CPAL frontend.
//!
//! ## Time model
//!
//! * **Simulation time** belongs to the game. It advances only on its fixed tick.
//! * **Mixer frame time** is an exact 64-bit frame count. It advances only while
//!   the mixer is not globally paused. `play_at` schedules at this clock.
//! * **Requested playback time** is the absolute mixer frame supplied by the
//!   caller. If it is in the past, playback starts at the next rendered frame and
//!   `MixerDiagnostics::late_starts` increases.
//! * **Device presentation time** is later than the mixer by the device buffer
//!   and backend queue. The native backend reports configured/sample latency,
//!   queue occupancy and underruns; it does not claim input-to-photon accuracy.
//!
//! No API promises sub-frame accuracy for native presentation. Offline rendering
//! of clips and procedural sources is sample-frame exact; streamed decode is
//! asynchronous and is intended for smoke/latency checks rather than golden
//! hashes.

#![warn(missing_docs)]

mod decode;
mod error;
mod mixer;
#[cfg(feature = "native")]
mod native;
mod offline;
#[cfg(feature = "settings")]
mod settings;
mod spatial;
mod stream;

pub use decode::{
    AudioFormat, Clip, MAX_CHANNELS, MAX_DECODED_BYTES, MAX_SAMPLE_RATE, MIN_SAMPLE_RATE,
};
pub use error::{AudioError, AudioResult};
pub use mixer::{
    BusConfig, BusId, Frame, LoopMode, LoopRegion, MAX_VOICE_GROUPS, Mixer, MixerConfig,
    MixerDiagnostics, PlaybackConfig, ProceduralSource, SoundSource, VoiceGroup, VoiceHandle,
    VoicePolicy, VoiceStatus,
};
#[cfg(feature = "native")]
pub use native::{AudioEngine, DeviceInfo, EngineConfig, EngineDiagnostics};
pub use offline::OfflineRenderer;
#[cfg(feature = "settings")]
pub use settings::{AudioSettings, BusSettings, SCHEMA_VERSION};
pub use spatial::{
    Listener, SpatialConfig, SpatialGain, SpatialModel, SpatialModelKind,
    evaluate as evaluate_spatial,
};
pub use stream::StreamSource;
