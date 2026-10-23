# quasar-audio

Independent Quasar audio: WAV/Ogg decoding, deterministic mixing, buses and
voice groups, voice limits, scheduled playback, spatialization, procedural
sources, streaming Ogg, offline PCM/WAV rendering and an optional CPAL native
backend.

The crate depends only on `crossbeam-channel`, `hound`, `lewton`, and optionally
`cpal` (and `serde`/`quasar-storage` for the `settings` adapter). It has no
dependency on VulkanicGAL, the renderers, `quasar-window`, Qt/KDE, an ECS, a
scene graph, Basalt, or a Quasar application loop.

```toml
[dependencies]
quasar-audio = { path = "../Quasar/crates/quasar-audio" }

# Headless/offline only:
# quasar-audio = { path = "../Quasar/crates/quasar-audio", default-features = false }

# Optional schema-versioned Basalt-path settings adapter:
# quasar-audio = { path = "...", features = ["settings"] }
```

`native` is on by default and adds CPAL. Offline rendering works with
`default-features = false`. The `settings` feature is independent of `native`.

## Quick start

```rust
use quasar_audio::{Clip, LoopMode, Mixer, MixerConfig, PlaybackConfig};

let clip = Clip::load("music.ogg")?;
let mut mixer = Mixer::new(MixerConfig::default())?;
let music = mixer.create_bus("music", 0.7, 4)?;
let handle = mixer.play(
    clip,
    PlaybackConfig {
        bus: music,
        looping: LoopMode::Whole,
        ..Default::default()
    },
)?;
let pcm = mixer.render(48_000)?; // interleaved f32, one second
assert!(handle.is_active());
# Ok::<(), Box<dyn std::error::Error>>(())
```

For a device:

```rust
use quasar_audio::{AudioEngine, EngineConfig, PlaybackConfig};

let engine = AudioEngine::open(None, EngineConfig::default())?;
let sfx = engine.create_bus("sfx", 0.8, 32)?;
let handle = engine.play(clip, PlaybackConfig { bus: sfx, ..Default::default() })?;
// engine.stop(&handle)?;
// engine.set_voice_paused(&handle, true)?;
```

`AudioEngine::devices()` lists outputs. `AudioEngine::open(Some(name), ...)`
selects one by the name reported by the platform. Without a device, use
`Mixer`/`OfflineRenderer`; `AudioEngine::open` returns `AudioError::NoDevice`
rather than fabricating playback.

## Sources

- `Clip::load` / `Clip::from_bytes` decode WAV (PCM 8/16/24/32 and IEEE float
  32) and Ogg Vorbis. Decoded assets are immutable `Arc` buffers shared by
  voices; a voice start does not copy PCM.
- `StreamSource` decodes Ogg on a bounded background thread. Container
  detection uses bytes, not the filename extension. Streaming WAV is explicitly
  unsupported; load WAV as a `Clip` because it is uncompressed. A stream does
  not expose a total frame count without decoding; use a `Clip` when the game
  needs the duration before playback.
- `ProceduralSource` generates one frame at a time on the mixing thread. It must
  be real-time safe and must generate at the mixer's sample rate; use a `Clip`
  for pitch, speed or looping.
- `SoundSource` is `Clip`, `Stream`, or `Procedural`; game code never names a
  Rodio/CPAL type.

## Buses, groups and voice admission

A `Mixer` always has bus 0 (`BusId::MASTER`). `create_bus` adds named
categories such as music, effects, UI, ambient or voice. Each bus has volume,
mute and an optional voice limit.

`PlaybackConfig` carries a bus, gain, pitch, speed, pan, loop mode, priority,
admission policy, optional spatial config and an optional `VoiceGroup`.
Groups provide per-entity or per-subsystem limits that are independent of
mixing buses, so a game does not need to create a bus for every entity.
`Mixer::set_group_limit` tracks at most `MAX_VOICE_GROUPS` groups and returns an
error rather than growing without bound.

`VoicePolicy::Reject`, `StealOldest` and `StealLowestPriority` are applied to
the global, bus and group limits in that order. A new request is rejected if its
priority is not higher than the chosen victim under `StealLowestPriority`.

`VoiceHandle` is a stable identity, not a slot. It remains queryable after the
voice finishes or is stopped and never aliases a later voice. It exposes status,
source position, active/terminal state, stop, pause/resume, gain, pitch, speed,
pan, spatial updates, seek and restart.

## Scheduling and clocks

Four clocks are intentionally distinct:

1. **Simulation time** — owned by the game; fixed-step ticks, pauses and replays
   follow the game's rules.
2. **Mixer frame time** — an exact 64-bit sample-frame count. It advances only
   while the mixer is not globally paused.
3. **Requested playback time** — the absolute mixer frame passed to `play_at`.
   A past frame starts on the next rendered frame and increments
   `MixerDiagnostics::late_starts`; it is never silently backdated.
4. **Device presentation time** — later than mixer time by the device buffer,
   driver and OS. `EngineDiagnostics` reports the negotiated format, requested
   buffer frames, queue occupancy and counters, but does not claim
   input-to-photon accuracy.

Offline rendering of decoded clips and procedural sources is sample-frame exact.
Streamed decode is asynchronous, so streamed offline output is useful for smoke
and latency tests but is not a golden-hash source. Native presentation is not
sample-accurate at the speaker. A fixed-step game can convert its own clock to
mixer frames (`tick * mixer_rate / tick_rate`) and call `play_at`; audio never
requires an ECS, window, renderer or application loop.

`LoopMode::Region { start, end }` is end-exclusive and sample-frame exact for
clips at unity rate: the intro plays once, then the region repeats until stop.
When pitch/speed or rate conversion makes the read position fractional, linear
interpolation can blend one output frame across the seam. For streaming Ogg, the
decoder restarts and discards to the region start; this is bounded and explicit,
but not a gapless seek implementation, with the same one-frame seam caveat.

## Spatial audio

Set a `Listener` position/forward/up and a `SpatialConfig` per voice. Built-in
`None`, `Linear`, `Inverse` (default) and `Exponential` models produce gain and
pan. `SpatialModelKind::Custom` accepts a `SpatialModel` implementation for a
game's exact attenuation/positioning rules. Custom models run on the audio
thread and must not block, allocate or perform I/O; non-finite results are
rejected to zero rather than poisoning the mix.

## Real-time behavior and limits

Native CPAL streams are created, played and destroyed on one dedicated device
thread because some backends mark them `!Send`. `AudioEngine` itself is
`Send + Sync`; dropping it signals and joins the device thread after the stream
has stopped.

The audio callback owns the mixer and drains a bounded `crossbeam-channel`
command queue with `try_recv`. Ordinary control calls use `try_send` and return
`AudioError::QueueFull` instead of blocking the game thread. Validation that
needs mixer state (bus existence, loop bounds, source limits and numeric
configuration) happens before queueing where possible; live updates and
metadata commands are revalidated by the callback, and failures appear in
`EngineDiagnostics::command_errors` and the affected handle's status. Streaming
`StreamSource` decoders are started on the caller thread and send decoded chunks
through a bounded queue; the callback does not open files, decode, spawn
threads, or grow ordinary voice/stream buffers. Decoded assets are capped at
256 MiB each, voice counts and group tables are bounded, and commands that would
exceed a table return an explicit error.

Native device loss is reported through `EngineDiagnostics::device_lost` and
`error`; the callback emits silence after a backend failure. `reopen` rebuilds
the engine and device. Active voices are not restored — the game must replay
from its authoritative simulation state.

## Diagnostics

`MixerDiagnostics` reports active/scheduled voices, starts, finishes, explicit
stops, steals, late starts, streaming underruns, decode errors, clipped frames,
the current mixer frame and peak. `EngineDiagnostics` adds queue occupancy and
capacity, command errors, device-loss state, the last backend error, negotiated
rate/channels and configured buffer frames (`buffered_frames`,
`buffered_seconds`).

## Settings adapter

With `features = ["settings"]`, `AudioSettings` persists schema-versioned JSON
for master volume, mute, preferred device name and named bus settings. The
caller supplies the path (typically from `quasar-storage::GamePaths`), so no game
directory is hardcoded. Missing files return `None`; corrupt, unsupported or
out-of-range files are errors and are not overwritten. `from_mixer` captures the
mixer's current settings; `apply_mixer` creates missing named buses and returns
their stable identities.

## Platform support and verified scope

- Linux ALSA through CPAL was exercised on the development host, including
  device enumeration, clip playback, streaming playback and a device-missing
  error path.
- CI is configured to compile-check Windows and macOS native builds; neither was
  hardware-validated in this change. arm64 and riscv64 remain unverified.
  Offline rendering is platform-independent.
- HRTF, OpenAL EFX, WMA/XMA/Wwise, automatic device-loss recovery, band-limited
  resampling and independent pitch-preserving time-stretch are not implemented.
  `pitch` and `speed` both change the resampling step; there is no separate
  time-stretch algorithm. Procedural sources run only at the mixer rate.

## Verification

```sh
cargo test -p quasar-audio --all-features
cargo test -p quasar-audio --no-default-features
cargo test -p quasar-audio --no-default-features --features settings
cargo clippy -p quasar-audio --all-targets --all-features -- -D warnings
cargo run -p audio-example -- --offline /tmp/audio.wav
cargo run -p audio-example -- --play
cargo run -p audio-example -- --devices
```

`tests/offline.rs` covers decode, mixing, resampling, buses, groups, mute,
pause, exact loop regions, scheduling, pitch/speed, pan/spatial/custom models,
procedural audio, limits, streaming, invalid inputs, reference timing and golden
mix hashes. `tests/native.rs` contains opt-in device tests and a compile-time
`AudioEngine: Send + Sync` assertion. `tests/settings.rs` covers settings
round-trips and corrupt-file rejection.

See [the audit and feature matrix](../../docs/audio-audit.md) and
[the implementation report](../../docs/audio-implementation-report.md).
