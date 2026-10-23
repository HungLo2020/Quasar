# Audio audit and feature matrix

This document records the read-only source audit that drove `quasar-audio`. It
is evidence-based: each game was inspected at the commit below through the
GitHub tree/contents APIs and, for MattMC's Rust audio tree, the local checkout
(HEAD `30c9b82f4433f9a9c2fe132515bcdddbc71e36d4`; no audio changes since the
reviewed `e6bc0e29` policy commit). No reference repository was modified.

| Repository | Revision inspected | Main implementation |
|---|---|---|
| `HungLo2020/MattMC` | `481ba61f9c254c16dd632505af6dc4253a5ee90c` (default branch); Rust audio policy at `e6bc0e29` | Java `SoundEngine` + Rust OpenAL policy/backend |
| `HungLo2020/BlackOps` | `563bfc24adbd9ff0c4533909fb0b7dcbea3ada2b` | Rust recovered alias policy + Rodio output |
| `HungLo2020/MicroMages` | `f45c4b9bcf5c073e6407419c70e7daa7311dc50a` | Rodio Ogg music/SFX |
| `HungLo2020/Jackal` | `b6d90a1c999dcb9d6f82cdc0e88ef876aac4ee77` | Rodio WAV music/SFX |
| `HungLo2020/MissileCommand` | `d078f9f21155dcf8bf926372a3c0e36c9a80986d` | Direct ALSA procedural synth |

## MattMC

Evidence: `docs/development/audio/SOUND-ENGINE.md`, `CLIENT-SOUNDS.md`,
`index.md`; `src/main/rust/audio/{backend,device,source,stream_decoder,decoder,
asset,listener,policy/*}`; `net/blaze3d/audio/*.java`;
`client/sounds/{SoundEngine,MusicManager,NativeSoundClient,NativeSoundPolicy}.java`.

Implemented:

- OpenAL via `alto`; one device-owner thread and a global backend lock.
- Two source pools split from 30 channels: static and streaming (`sqrt(total)`
  clamped to 2..8 streaming, remainder static). Pool exhaustion is reported.
- Static Ogg decode via `lewton`; 256 MiB decoded cap; Java-supplied bytes copied
  into Rust-owned assets; Ogg granule truncation.
- Streaming queue: 16,384-frame chunks, depth 4, decoder thread, underrun/EOF
  distinction, reusable buffers.
- Listener position/orientation/gain; per-source position, pitch, gain, looping,
  relative flag, disable-attenuation, linear attenuation distance; default
  inverse-clamped distance model; device enumeration, preferred/default/implicit
  fallback, disconnect/default-change detection and reopen.
- 11 vanilla sound categories; category volume × instance volume; music bus;
  pause/resume; manual loops; delayed sounds; 20-tick minimum source lifetime;
  replacement voices remain until they end.
- `sounds.json` parsing, weighted event resolution with caller-owned RNG draws,
  nested events (max depth 32, 64 retained draws), preload/stream flags,
  subtitles, music delays (`STARTING_DELAY=100`, frequency max delays
  20/10/0 minutes), ambience/biome loops/fades, mood, volume preview.
- Explicitly not complete: real-device reload with pending IO, stream recovery,
  long-session resource behavior, and complete audio-hardware verification.

Migration implications for Quasar:

- Quasar needs categories/buses, listener transforms, inverse/linear
  attenuation, relative/disable-attenuation controls, static clips, streaming
  Ogg, pause/resume, looping, queued/delayed playback, voice lifetime rules and
  device reopen.
- Weighted `sounds.json` resolution and Minecraft category names stay in MattMC;
  Quasar provides seeded scheduling, buses, source selection primitives and
  streaming.
- MattMC's OpenAL HRTF/limiter and the exact device thread are backend-specific;
  CPAL does not claim HRTF parity.

## BlackOps

Evidence: `src/audio.rs`, `audio_policy.rs`, `sound_host.rs`, `sound_catalog.rs`,
`loop_sound.rs`, `position_sound_events.rs`, `runtime_audio.rs`,
`analysis/audio-policy.md`, `audio-runtime.md`, `loop-sound.md`,
`position-sound-events.md`, `playsound-position.md`, and the audio tests.

Implemented and measured:

- 2,639 authored alias lists, 8,717 variants, 1,801 available PCM bindings;
  portable catalogue with lazy PCM. T5 PCM16 and Microsoft ADPCM decoders are
  independently implemented; 115 WMA files remain explicitly unavailable.
- Recovered alias hash, CRT `rand()` transition, uniform variant selection over
  the first 64 eligible variants, context filtering, repeat avoidance for >2
  variants, separate probability gate, separate gain/pitch random draws, exact
  packed scalar scales, pitch clamp 0.01..1.9, start-delay milliseconds,
  secondary alias recursion/validation, global/entity limit modes (oldest,
  reject; priority-steal mode 3 is rejected until computed priority is known).
- Real output: Rodio/CPAL, 64 voices, 128 MiB resident budget, one-shot and loop,
  gain/pitch, voice handles, stop/stop-all, completion polling.
- Server loop-sound state stores alias hash and signed fade milliseconds exactly;
  client spatial/per-frame loop refresh is not implemented and is rejected.
- Positional event store: 300 ms expiry, allocation-order consume-once dispatch,
  20 Hz frame boundary, exact event fields; listener resolves via SoundHost and
  the shared output/policy. Spatial, buses, pan, duck, snapshots, occlusion,
  reverb and fades are **not** implemented.

Migration implications:

- Quasar must offer per-voice volume/pitch/looping/priority/limits, delayed
  sample-frame scheduling, voice handles/status, output completion diagnostics,
  and a way for BlackOps to keep alias selection/RNG/context logic in game code.
- BlackOps can use Quasar's `Mixer`/`AudioEngine` as the output sink while
  retaining its catalogue, hash, variant selection and exact random stream.
- Quasar must expose a custom spatial model and per-voice pan for the still-
  unreconstructed BlackOps spatial/bus path; it must not pretend those rules are
  recovered.

## MicroMages

Evidence: `src/audio.rs`, `assets/music/*.ogg`, `assets/sfx/*.ogg`,
`scripts/verify_audio.py`, `Cargo.toml`.

Implemented:

- Rodio 0.20; nine Ogg music tracks and 37 Ogg SFX assets (34 indexed in code).
- Preloads all decoded SFX into `Arc<[f32]>`; music is decoded per start.
- Music: one sink; five tracks loop whole, four use intro-then-region loops
  (`dreamy` 1.596..39.934, `intro` 15.174..24.759); one track has a non-unity
  speed (`dreamy` bits `0x3f7f943f`); music volume 0.85 × user volume; 256-step
  fade.
- SFX: three voices with per-effect priority; duplicate effect stops the existing
  voice; when no slot is free, replace if new priority >= old; pitch uses
  `speed = max(1 + pitch*0.05, 0.4)`; 0.4..; loop disabled for SFX.
- Music/effects volume 0..100; pause preserves position; unpause resumes; host
  pause pauses all sinks.
- Assets and Ogg decode are tested; host ALSA verification uses an ALSA file/null
  plugin through a FIFO (`verify_audio.py`).

Migration implications:

- Quasar needs exact loop regions, music/SFX buses, 3-voice limits with
  priorities, same-effect replacement, pause/resume, per-sound speed/pitch,
  volume fades (games can update volume over time), and looped music.
- MicroMages-specific track IDs, loop metadata and priority tables stay in the
  game.

## Jackal

Evidence: `src/sound.rs`, `assets/audio.json`, `tools/reference/AUDIO_EXPORT.md`,
`export_audio.py`, `Cargo.toml`.

Implemented:

- Rodio; 43 checked-in PCM16 mono WAV assets at 44.1 kHz, all exported from the
  recovered driver/synthesizer; loop start frames are recorded for looping
  tracks (e.g. boss, briefing, stage tracks, helicopter, ending).
- Music one at a time; one sink; looping is intro-play-then-loop-from-start
  (no loop end; repeats to file end).
- Effects: 16-sink limit, oldest stopped when full; no priority table; pause
  pauses music and effects; stop/restart on command; music/effects volume
  constants (0.75/0.8) and no per-sound pitch/pan.
- Command IDs 0/0x59 stop music + effects; music command IDs select tracks;
  everything else is an effect. Scene timelines retain original cue IDs.
- Decode/loop bounds are tested; no spatial audio.

Migration implications:

- Quasar needs file WAV decode, whole/region looping, 16-voice limit with
  oldest-steal, music exclusivity, pause/resume, stop-all and stable handles.
- Jackal's command-ID mapping and asset metadata remain game-owned.

## MissileCommand

Evidence: `src/audio.rs`, `Cargo.toml`, `README.md`.

Implemented:

- Direct Linux ALSA (`alsa` crate), 48 kHz stereo, 5 ms period / 40 ms buffer
  target; worker thread; 512-frame i16 buffer.
- Procedural synth with 24 voices, LFSR noise, square-wave oscillator, per-effect
  frequency/gain/envelope/duration; effects: launch, explosion, city lost, bonus,
  empty, game over; hard clamp ±0.85; mute clears voices; graceful fallback when
  ALSA/device is unavailable.
- No files, no looping, no spatial, no per-sound pitch, no buses, no non-Linux
  backend. Tests check bounded voices and decay.

Migration implications:

- Quasar needs procedural generators, bounded voice count, mute, low-latency
  native output and device failure handling.
- MissileCommand's synth tables and effect semantics stay in the game; Quasar
  does not hardcode arcade sound definitions.

## Final feature matrix

Legend: **Y** implemented; **P** partial/limited; **N** not implemented;
**G** provided as a primitive for game code to build on.

| Capability | MattMC | BlackOps | MicroMages | Jackal | MissileCommand | Quasar |
|---|---|---|---|---|---|---|
| WAV decode | N (Ogg) | Y (T5 PCM/ADPCM) | N (Ogg) | Y | N | Y |
| Ogg Vorbis decode | Y | N (WMA gaps) | Y | N | N | Y |
| Memory/file-backed clips | Y | Y | Y | Y | N | Y |
| Streaming music | Y | Y (catalogue) | N | N | N | P (Ogg) |
| Music playback/looping | Y | Y (base) | Y (regions) | Y (intro+start) | N | Y |
| Loop points | G | P | Y | Y | N | Y |
| Simultaneous SFX | Y (pools) | Y (64) | Y (3) | Y (16) | Y (24) | Y |
| Categories/buses | Y (11) | N | P (music/SFX) | P | N | Y |
| Master mute/volume | Y | Y | Y | P | Y | Y |
| Per-sound volume/pitch | Y | Y | Y | P | N | Y |
| Playback speed | Y (pitch) | Y (pitch) | Y | N | N | Y |
| Stereo pan | Y (OpenAL) | N | N | N | N | Y |
| Voice priorities/limits | Y (pools) | Y (modes) | Y | Y (oldest) | Y (24) | Y |
| Per-entity voice groups | P | Y | N | N | N | Y |
| Settings persistence | Y (game config) | P | P | P | P | Y (optional adapter) |
| Scheduled playback | Y (delays) | Y (ms) | G | G | N | Y |
| Listener/attenuation | Y | N | N | N | N | Y |
| Custom spatial policy | G | G | N | N | N | Y |
| Procedural PCM | N | N | N | N | Y | Y |
| Device enumeration/selection | Y | Y | P | P | P (ALSA name) | Y |
| Device loss/reopen | Y | P | P | P | P | P (reopen loses voices) |
| Pause/resume | Y | P | Y | Y | Y | Y |
| Offline deterministic render | N | N | N | N | N | Y |
| Diagnostics | Y | Y | P | P | P | Y |

“P” is deliberate: for example, Quasar streams Ogg but not WAV, and native
device reopen reconstructs the engine while game-owned voices must be replayed.

## What stays in the games

`quasar-audio` contains no Minecraft sound event names, no BlackOps alias/RNG
tables, no Jackal command IDs, no MicroMages track metadata and no MissileCommand
oscillator tables. Those are game assets/algorithms. Quasar owns decoding,
mixing, buses, scheduling, spatial math, voice admission, resource lifetime,
device output and diagnostics; games own selection, semantic event naming,
replay/random policy and their own sound definitions.

## Migration mapping

- **MattMC:** keep Java `SoundInstance`/world policy. Route its Rust backend's
  device/voice commands to `AudioEngine` or implement the same backend contract
  on top of `Mixer`; map categories to buses, `SoundConfig` to `PlaybackConfig`,
  listener transforms to `Listener`, and use `StreamSource` for streamed Ogg.
  Keep weighted resolution and Minecraft identifiers in MattMC.
- **BlackOps:** keep `SoundCatalog`, `audio_policy`, `alias_id`, `AudioRandom`,
  variant selection and limits in the game. Replace `AudioOutput` with
  `AudioEngine`/`Mixer`; map `PlaybackOptions` to `PlaybackConfig`, preserve
  `starts_at` with `play_at` in mixer frames (convert milliseconds using the
  game clock), map entity/global admission modes to `VoiceGroup` plus
  `VoicePolicy`, and implement unrecovered spatial/bus/duck rules as custom
  `SpatialModel`/game policy rather than claiming Quasar parity.
- **MicroMages:** keep track/SFX tables and loop metadata. Replace Rodio sinks
  with two buses, `LoopMode::Region` for the four intro/region tracks, priority
  voice limits, and per-effect speed; volume changes/fade remain game updates.
- **Jackal:** keep command-ID timeline mapping and the 43-asset manifest.
  Replace Rodio sinks with one music voice plus a 16-voice SFX bus using
  `StealOldest`; loop metadata uses `LoopRegion` or `Whole`.
- **MissileCommand:** keep the synth as a `ProceduralSource` (or render to a
  `Clip` for offline determinism); use a 24-voice limit and mute. Replace direct
  ALSA with the native backend so Windows/macOS become possible; preserve the
  documented Linux-only original behavior as a game policy, not a Quasar limit.

## Known limits of the Quasar replacement

- Native output uses CPAL. Linux ALSA was exercised on this host; CI is
  configured to compile-check native audio on Linux, Windows and macOS. None of
  Windows, macOS, arm64 or riscv64 was hardware-validated.
- Streaming is Ogg-only. Streaming WAV is rejected explicitly; use `Clip::load`.
- `StreamSource` has no pre-decode duration/frame count; use a `Clip` when the
  game must know the total length before playback.
- Streaming loop regions restart the decoder and discard to the start frame.
  This is bounded but can cost CPU for long intros; it is not a gapless seek
  implementation. When the output rate differs from the stream rate, or pitch/
  speed resampling is active, linear interpolation can blend across the loop
  seam by one output frame. The same one-frame seam interpolation applies to
  clip loop regions at non-unity pitch/speed or after rate conversion; at unity
  rate the clip seam is sample-frame exact. That limitation is documented
  rather than hidden.
- Procedural sources must generate at the mixer rate. They cannot loop, seek,
  restart or be pitched in place; render to a `Clip` for those operations.
- Streaming and procedural voices cannot be rewound in place; create a new voice.
- `pitch` and `speed` are both resampling multipliers. There is no independent
  pitch-preserving time-stretch.
- The settings adapter persists reusable master/bus/device settings, not
  game-specific sound IDs, alias tables or event mappings.
- Resampling uses linear interpolation. It is deterministic and suitable for
  effects/music, but it is not a band-limited resampler.
- Offline clip/procedural rendering is sample-frame exact. Stream decoding is
  asynchronous, so streamed offline renders are smoke/latency tests, not golden
  hash references.
- Native presentation is buffered and not input-to-photon calibrated. Mixer
  frames are exact; device presentation is later by an unreported OS/driver
  amount plus the optional `buffer_frames` request.
- Reopening a lost device reconstructs the engine; active voices are not
  restored. Games must replay from authoritative state.
- HRTF, OpenAL EFX, WMA/Wwise/XMA, and the full BlackOps/MattMC mixer remain
  unsupported. No compatibility stub claims them.
