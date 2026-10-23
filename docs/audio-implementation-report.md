# `quasar-audio` implementation report

Date: 2026-10-23. Reference repositories were read-only; no MattMC, BlackOps,
MicroMages, Jackal or MissileCommand file was modified, and no Quasar commit or
push was made.

## 1. Audit and feature matrix

The repository-by-repository audit, evidence paths, revisions, feature matrix
and migration mapping are in [`audio-audit.md`](audio-audit.md). Audited
revisions:

| Repository | Revision |
|---|---|
| MattMC | `481ba61f9c254c16dd632505af6dc4253a5ee90c` default branch; Rust audio unchanged since `e6bc0e29835e431327cc8d7007ffce5fe53ce1b4`; local checkout `30c9b82f4433f9a9c2fe132515bcdddbc71e36d4` |
| BlackOps | `563bfc24adbd9ff0c4533909fb0b7dcbea3ada2b` |
| MicroMages | `f45c4b9bcf5c073e6407419c70e7daa7311dc50a` |
| Jackal | `b6d90a1c999dcb9d6f82cdc0e88ef876aac4ee77` |
| MissileCommand | `d078f9f21155dcf8bf926372a3c0e36c9a80986d` |

The audit distinguishes implemented behavior from planned or incomplete work.
Examples of the latter are BlackOps' unrecovered client spatial/bus/duck path,
MattMC's pending stream recovery and real-device reload work, and Jackal's
recovered audio timing without a full mixer.

## 2. Dependency selection

| Dependency | Role | Rationale |
|---|---|---|
| `cpal` 0.15.3 (optional, default) | Linux ALSA, Windows, macOS output | Direct device enumeration/selection and a callback we control. Rodio's `OutputStream`/`Sink` layer would hide device lifetime and make a custom deterministic mixer and bounded command path harder to keep authoritative. CPAL is used by both Rodio and the audited games. |
| `hound` 3.5 | WAV decoding/writing | Small, maintained RIFF/WAVE reader for PCM 8/16/24/32 and IEEE float; used for offline WAV output too. |
| `lewton` 0.10 | Ogg Vorbis decoding | Pure-Rust decoder already used by MattMC; avoids a C codec dependency and supports packet-by-packet streaming. |
| `crossbeam-channel` 0.5 | Bounded command/stream queues | Non-blocking `try_send`/`try_recv` with explicit capacity, used on both the control/callback boundary and the decoder/mixer boundary. |
| `serde` + `quasar-storage` (optional `settings`) | Schema-versioned Basalt-path settings | Reuses Quasar's atomic JSON persistence without making the audio crate depend on Basalt or hardcoding a game directory. |

Rodio was evaluated and is acceptable, but was not selected because Quasar must
own the public API, voice lifetime, callback allocation policy, device thread and
offline/native parity. No Rodio type appears in the public API.

## 3. Architecture

```text
game/control thread
  ├── AudioEngine (Send + Sync handle) ── bounded command queue ──┐
  ├── Clip decode / StreamSource decoder thread (bounded queue) ──┤
  └── OfflineRenderer / Mixer (deterministic, no device)          │
                                                                  ▼
device-owner thread: owns cpal::Stream, creates/destroys on one thread
  └── audio callback: drains commands, mixes frames, publishes atomics
```

Modules:

- `decode`: `Clip`, WAV/Ogg decoding, bounded decoded PCM, Ogg granule trim.
- `stream`: bounded Ogg decoder worker and `StreamReader`.
- `mixer`: voices, buses, groups, admission, scheduling, resampling, pan,
  diagnostics, PCM/WAV rendering.
- `spatial`: listener, distance models, stereo pan, custom `SpatialModel`.
- `native`: device enumeration/open, CPAL stream ownership, command bridge,
  device diagnostics and reopen.
- `offline`: deterministic renderer wrapper.
- `settings`: optional Basalt-path JSON adapter.
- `error`: stable backend-independent errors.

Design decisions that matter for correctness:

- `VoiceHandle` is an identity with atomic status/position, not a reused mixer
  slot. Terminal status remains observable.
- Streaming decoder threads are created on the caller (or offline mixer) thread.
  The native callback receives an already-prepared `PreparedSource`, so it never
  opens files, decodes or spawns threads.
- CPAL streams are `!Send` on some backends, so they never leave the device
  thread. `AudioEngine` is `Send + Sync`; dropping it signals and joins.
- Bus/voice-group tables are preallocated and bounded. The 64-voice-group cap and
  256 MiB per-clip decoded cap are explicit errors, not silent growth.
- `pitch` and `speed` both multiply the source step. Quasar does not claim a
  pitch-preserving time-stretch.
- Mixer frames freeze on global pause; scheduled starts and loop regions use the
  same frame clock. Device presentation latency is reported as configured buffer
  frames, not as measured input-to-photon latency.

## 4. What the crate provides

- WAV/Ogg file, memory and streaming sources; procedural sources.
- Simultaneous voices, 64-voice benchmark, configurable global/bus/group limits,
  priorities and reject/oldest/lowest-priority policies.
- Music/effects buses with master volume, mute, per-bus mute/volume/limit.
- Per-voice gain, pitch, speed, pan, spatial placement, pause/resume, stop,
  seek/restart (clip only), loop modes and sample-frame scheduling.
- Listener transforms, inverse/linear/exponential attenuation and custom
  spatial models.
- Playback status and cancellation through stable handles.
- Device enumeration/selection, graceful missing-device errors, device-loss
  diagnostics and explicit reopen.
- Offline PCM/WAV rendering and deterministic hashes; no device required.
- Diagnostics for active/scheduled voices, starts/finishes/stops/steals, late
  starts, underruns, decode errors, clipping, peak, queue occupancy/capacity,
  command errors and device failure.
- Optional settings persistence at caller-resolved Basalt paths.

Game-specific behavior stays out of the crate: no Minecraft category/event IDs,
BlackOps aliases/RNG/priorities, Jackal command IDs, MicroMages track tables or
MissileCommand oscillator tables.

## 5. Tests and verification

Run on the development host (Linux x86_64, Rust 1.94.0):

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo test -p quasar-audio --all-features -- --include-ignored --test-threads=1
cargo test -p quasar-audio --no-default-features --locked
cargo test -p quasar-audio --no-default-features --features settings
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked
cargo run --release -p audio-example -- --offline ... --benchmark ...
cargo run -p audio-example -- --devices
cargo run -p audio-example -- --play --frames 24000
```

Results:

- Formatting: pass.
- Workspace tests: pass (all non-ignored tests; GPU tests remain opt-in).
- `quasar-audio` all features: 3 unit + 30 offline + 4 native + 3 settings
  tests pass; 2 of the native tests are hardware tests `#[ignore]`d for ordinary
  CI and pass with `--include-ignored` on this host.
- `quasar-audio` without default features: 3 unit + 30 offline tests pass; no
  CPAL/ALSA build dependency.
- Settings-only, no native: 3 unit + 30 offline + 3 settings tests pass.
- Strict Clippy with warnings denied: pass.
- Rustdoc with `-D warnings`: pass.
- Native Linux ALSA: device enumeration, clip playback, prepared Ogg stream,
  spatial effect, and missing-device error path exercised. CI is configured to
  compile-check native Linux/Windows/macOS builds; those CI jobs were not run in
  this session because no push was made.
- Deterministic reference hashes: identical in debug and release.
- Independent consumer: a temporary crate outside the workspace depending on
  `quasar-audio` with `default-features = false` built and ran a clip/mixer
  program, proving the crate does not require the Quasar workspace or loop.
- Offline stream smoke: first audio ≈2–5 ms, queue bounded at 32 KiB, no decode
  errors.

The two ignored native tests are:

- `native_engine_plays_a_short_clip_and_reports_diagnostics`
- `missing_device_is_reported_without_fabricating_output`

They are ignored in ordinary CI because GitHub runners do not provide a reliable
audio device. The compile-time `AudioEngine: Send + Sync` assertion runs in CI.

## 6. Benchmarks

Latest benchmark results and host details:
[`examples/audio/benchmarks/2026-10-23-linux-x64/`](../examples/audio/benchmarks/2026-10-23-linux-x64/).

| Measurement | Result |
|---|---:|
| Four-second demo mix | 11.71 ms (61.0 ns/frame) |
| Demo render allocations | 1 (output buffer) |
| 64 voices, one second | 63.37 ms (20.6 ns per voice-frame) |
| 64-voice render allocations | 0 |
| Decoded clip memory | 2,025,600 bytes |
| Stream first audio | 2.06 ms |
| Stream startup underruns | 96 frames (2.0 ms) |
| Stream queue memory bound | 32,768 bytes |

Interpretation: a 64-voice mix uses about 6.3% of one core on this host in
release, and ordinary mixing allocates nothing after buffers are prepared.
Streaming decode allocates on its worker thread; the callback only drains
bounded chunks. These are single-host measurements with normal run-to-run variance, not
cross-platform targets.

## 7. Known limitations

- Only Linux ALSA/CPAL was hardware-validated. Windows/macOS and non-x86 targets
  compile where applicable but are unverified on hardware.
- Streaming is Ogg-only; WAV is loaded as a `Clip` because it is uncompressed.
- `StreamSource` exposes no pre-decode duration/frame count.
- Streaming loops restart the decoder and discard to the region start; the seam
  can interpolate by one output frame when resampling.
- Resampling is linear, not band-limited.
- Offline clip/procedural rendering is sample-frame exact; streamed decode is
  asynchronous and is not used for golden hashes.
- `pitch` and `speed` are both step multipliers; no independent time-stretch.
- Procedural sources run only at the mixer rate and cannot loop/seek/restart or
  be pitched in place.
- Reopening a lost device does not restore active voices; games replay from
  authoritative state.
- No HRTF, OpenAL EFX, WMA/XMA/Wwise, automatic device-loss recovery or
  sample-accurate speaker presentation claim.
- Settings persistence covers reusable master/bus/device state only, not game
  sound names or event mappings.
- Ordinary callback allocations are avoided, but a game-supplied custom
  `SpatialModel` or `ProceduralSource` can violate the documented real-time
  contract; Quasar cannot enforce it.

## 8. Migration mapping

See the per-repository mapping in [`audio-audit.md`](audio-audit.md#migration-mapping).
In summary:

- **MattMC:** keep `SoundInstance`, category/event resolution and Minecraft
  identifiers; replace the Rust OpenAL backend commands with `AudioEngine`/`Mixer`
  buses, `Clip`/`StreamSource`, listener transforms, and scheduled `play_at`.
- **BlackOps:** keep catalogue/hash/RNG/alias policy and recovered timing;
  replace `AudioOutput` with `AudioEngine`, map `starts_at` to mixer frames, and
  map entity/global limits to `VoiceGroup` plus `VoicePolicy`.
- **MicroMages:** keep track/SFX tables; use two buses, `LoopRegion`, group/bus
  limits, per-effect speed and pause.
- **Jackal:** keep command IDs and the asset manifest; use one music voice plus a
  16-voice effects bus with `StealOldest` and clip/region loops.
- **MissileCommand:** keep the synth as a `ProceduralSource` (or render to a
  `Clip` for deterministic tests), a 24-voice limit and mute; the native backend
  supplies non-Linux output without changing the game's sound tables.
