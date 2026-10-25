# Framebuffer

A software-rendered 640x400 RGBA8 framebuffer presented through
`quasar-render-2d`'s `FramebufferPresenter`.

This example intentionally owns its raw `winit` loop instead of using
`quasar-app`: it exercises the presenter, allocation accounting and resize
behaviour below the runner, and serves as the documented lower-level escape
hatch for applications that need their own event loop. It exercises persistent dynamic
texture updates, scaling/filtering, letterboxing, resize, zero-size deferral and
recovery, headless screenshots, and native presentation benchmarks.

```sh
cargo run -p framebuffer -- --frames 120
cargo run -p framebuffer -- --frames 30 --exercise-window
cargo run -p framebuffer -- --headless --screenshot /tmp/framebuffer.png
cargo run -p framebuffer -- --frames 364 --benchmark /tmp/framebuffer-bench.json
cargo run -p framebuffer -- --frames 364 --poll --benchmark /tmp/framebuffer-poll.json
```

Escape quits. `--poll` enables completion polling; the default uses synchronous
retirement, matching the previous presenter. `--exercise-window` resizes,
toggles fullscreen, defers a zero-sized presentation, and verifies recovery.
Benchmarks exclude 64 warmup frames and record CPU/GPU frame distributions,
upload bytes, blocking/idle waits, resource churn, Rust allocations, and
observed completion latency.

## API

The convenience path owns a renderer, a persistent texture, and a one-quad
canvas:

```rust
let mut presenter =
    FramebufferPresenter::new(window, window_size, (width, height), options)?;
presenter.present(&rgba)?;
presenter.resize(new_width, new_height);
```

For shared-device composition, `RenderState2D` exposes the same persistent
texture path without owning a window:

```rust
let dynamic = state.create_dynamic_texture(&mut gal, &mut textures, width, height)?;
state.update_dynamic_texture(dynamic, &rgba)?;
let commands = state.prepare(&mut gal, &canvas, &textures, target, options, LoadOp::Load)?;
```

The texture, view, samplers, bindings, and up to three upload slots are reused.
Each prepared command list owns its framebuffer snapshot, so submissions can be
ordered or reordered without a later upload overwriting an in-flight frame.
Dynamic textures must be updated before each `prepare` that draws them and are
released with `destroy_dynamic_texture`.

## Benchmarks

[The 2026-10-23 RTX 2070 comparison](benchmarks/2026-10-23-rtx2070/README.md)
records the replacement against the removed legacy presenter. Steady-state
GAL resource churn dropped from 1,196 creates/destroys to zero, Rust allocations
dropped about 14%, and CPU/GPU frame times stayed within run variance.
