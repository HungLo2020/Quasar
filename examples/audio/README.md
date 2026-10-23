# Audio example

An independent `quasar-audio` example that builds a music bed, overlapping
positional/non-positional effects, multiple buses, a procedural voice, a
voice-group limit, WAV and Ogg clips, and deterministic offline output.

It does not create a window or require VulkanicGAL. Offline mode is suitable for
headless CI; `--play` uses the optional CPAL backend.

## Commands

```sh
# Render four seconds to WAV and print deterministic/benchmark data.
cargo run -p audio-example -- --offline /tmp/audio-example.wav

# Same, with a JSON benchmark record.
cargo run -p audio-example -- --offline /tmp/audio-example.wav \
  --benchmark /tmp/audio-benchmark.json

# List native outputs (does not open one).
cargo run -p audio-example -- --devices

# Play the demo on the default output.
cargo run -p audio-example -- --play --frames 48000

# Offline-only build (no CPAL/ALSA compile-time dependency).
cargo run -p audio-example --no-default-features -- --offline /tmp/audio.wav
```

`--frames N` overrides the offline render length and the approximate native
playback duration. `--play` and `--offline` are mutually exclusive.

## What the demo schedules

- A four-second stereo music clip on a `music` bus, looping whole-file.
- Three overlapping decaying effects scheduled at exact mixer frames, panned
  left/centre/right, with priorities and a `VoiceGroup` limit.
- WAV and Ogg references decoded through `Clip::from_bytes`.
- A procedural 1.2 kHz generator on the mixer thread.
- A spatial effect with an inverse distance model.
- The same music/stream/spatial/bus controls through `AudioEngine` in `--play`.

The example's global allocator counts allocations around the offline mix and
the 64-voice polyphony run. Output hashes are taken over quantized 16-bit PCM so
debug and release builds agree despite sub-LSB floating-point contraction. The
checked-in WAV/Ogg tones are generated sine references, not game assets.

## Benchmark output

The JSON includes:

- `mix_ms`, `mix_ns_per_frame`, `allocations`, `allocated_bytes`,
  `decoded_asset_bytes`, `peak`, clipping and voice counters for the demo.
- `polyphony`: 64 simultaneous clips, mixing time, per-voice cost, allocations
  during rendering and completion count.
- `streaming`: first-audio latency, rendered frames, underruns, decode errors and
  the bounded stream-queue memory budget.

Stored results live in [`benchmarks/`](benchmarks/). Timings are host-specific;
use them for regression comparisons on the same machine, not as cross-machine
targets.
