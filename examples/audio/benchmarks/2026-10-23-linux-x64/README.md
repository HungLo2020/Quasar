# Offline audio benchmark — 2026-10-23

Host: Intel Core i7-10750H (12 logical CPUs), Linux 7.0.0-38-generic x86_64,
Rust 1.94.0, release profile. Output: [`offline-mix.json`](offline-mix.json).

Command:

```sh
cargo run --release -p audio-example -- \
  --offline /tmp/opencode/audio-final.wav \
  --benchmark examples/audio/benchmarks/2026-10-23-linux-x64/offline-mix.json
```

| Measurement | Result |
|---|---:|
| Demo render length | 192,000 frames / 4.0 s |
| Demo mixing cost | 11.71 ms (61.0 ns/frame) |
| Allocation during demo mix | 1 (the output `Vec`) |
| Decoded clip memory | 2,025,600 bytes |
| Peak | 0.4946 |
| Clipped frames | 0 |
| Underruns (offline) | 0 |
| 64-voice polyphony mix | 63.37 ms for 1.0 s (1,320.1 ns/frame, 20.6 ns per voice-frame) |
| Allocation during 64-voice mix | 0 |
| Stream first audio | 2.06 ms (single run) |
| Stream startup underruns | 96 frames (2.0 ms) |
| Stream bounded queue memory | 32,768 bytes |
| Stream decode errors | 0 |

The demo mixes a looping stereo music bed, overlapping effects, WAV/Ogg clips,
a procedural voice and a spatial emitter across two buses and a voice group.
The polyphony run starts 64 shared clips and renders one second with a
preallocated output buffer; its zero-allocation result covers rendering, not
file decoding or thread setup.

Streaming figures are a single real-time-paced run and include decoder-thread
setup. Decoder allocations are off the audio callback and bounded by the queue;
they are not counted against the callback's zero-allocation mixing result.
Timings are host-specific and vary with CPU frequency/thermal state; compare them on the same machine and toolchain.
