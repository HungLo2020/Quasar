# Native wgpu graphics migration

VulkanicGAL is retained as Quasar's central graphics import, with only
`pub use wgpu; pub use wgpu::*;`. The dependency is external crates.io wgpu
30.0.1, with no fork, vendor tree or changes to upstream source. Driver handling,
validation, shader translation, synchronization and resource ownership all come
from upstream. Quasar renderers contain only their rendering behavior.

| Previous API | Current API |
|---|---|
| `VulkanicGal`, `GalOptions`, `BackendChoice` | Native `Instance`/`Adapter`/`Device`/`Queue`; optional `quasar_frame::Gpu` |
| `Handle`/resource registry | Owned native `Texture`, `TextureView`, `Buffer`, bind groups |
| `TextureDesc`, `BufferDesc`, usage vectors | Native borrowed descriptors and usage bitflags |
| `CommandOp`, `CommandList`, explicit barriers | Native encoder/passes, single-use `CommandBuffer` |
| Submission receipts, manual retirement | `Queue::submit`, `SubmissionIndex`, `Device::poll`, native Drop |
| GLSL/Shaderc/SPIR-V rewrite | Checked-in WGSL, native shader modules and pipeline validation |
| Numeric render-target handle | `RenderTarget` with native attachment views and dimensions |
| `VULKANIC_BACKEND`, separate `wgpu` feature | `WGPU_BACKEND`, backend feature forwarding |
| GAL-private profiling/counters | Frame timings, native query sets, optional native allocator reports |

No unrelated crate should add a direct dependency on wgpu. Import upstream types
through `vulkanic_gal`, including `util::DeviceExt`. They have identical type
identity to upstream, allowing external libraries using the same wgpu version to
compose. Native backends are Cargo features, selected by upstream adapter logic;
features do not prove hardware availability on another operating system.

```rust,ignore
use quasar_frame::{Gpu, OffscreenTarget};
let gpu = Gpu::headless()?;
let target = OffscreenTarget::new(&gpu, 640, 400)?;
let world = three_d.prepare(&gpu, target.target(), camera, &draws, pass)?;
let hud = two_d.prepare(&gpu, &canvas, &textures, target.target(), options, load)?;
let raw = gpu.encoder("application native commands").finish();
gpu.queue.submit([world.commands, raw, hud.commands]);
```

Renderers are optional resources alongside an independently usable native device.
No ECS, scene, window, physics or application runner is required for composition.
A headless CPU game can omit graphics crates entirely. The native surface helper
retains an Arc window; low-level games can own a native surface and configure,
acquire, encode, submit and present directly.

`Material`, `LitMaterial`, `SurfaceMaterial` and `NormalMap` now own cloned native
views, so they implement Clone rather than Copy. Clone a material when reused.
Texture upload helpers return owners/views; sampler overrides accept native
samplers. Prepare methods take `&Gpu` and `&RenderTarget`. Commands are consumed
on submission, and repeated execution requires recording another buffer.

Reloading drops old owners after publishing replacements. Outstanding native
command buffers retain old resources and can still be submitted. Releasing
caches or resizing no longer fails because an artificial slot registry is full.
Do not explicitly destroy native textures/buffers still used by encoded work.
Dynamic 2D textures and the default shadow preparation use immutable generations
for arbitrary submission ordering; unchanged 2D generations upload zero bytes.
Renderer geometry/instance/light buffers grow and retain their native bindings.
Each preparation encodes immutable staging copies before its draws, preserving
its snapshot even if frames are submitted in reverse order. Native command
buffers retain staging resources and earlier buffers after growth.

For immediately submitted frames, `FramebufferPresenter` keeps a private texture
and encodes its upload before drawing. `ShadowRenderState3D::prepare_into` accepts
an explicit reusable `ShadowTarget`. Submit each shadow producer and all its
receivers together before the next frame overwrites that target; Nightwatch uses
this path. Applications requiring independently reordered producers/receivers use
ordinary `prepare`. These are renderer choices using native resources, without
another submission tracker. Outstanding prepared frames still retain memory.

Frame GPU timing is optional, frame-correlated and asynchronous under polling.
Enable it for benchmarks, poll/wait before final summaries, and preserve absent
measurements on unsupported adapters. The query interval covers submitted
command buffers, including explicitly encoded staging copies. Implicit
`Queue::write_*` transfers precede that interval and are excluded; this includes
some asset/dynamic-image uploads. Timing is not a measurement of all driver work. Native validation/allocator details that
upstream does not expose cannot be reconstructed as equivalent GAL counters.
Historical benchmark reports describe earlier releases and are not current API
contracts. See [the validation report](native-wgpu-validation.md) for measured behavior and platform limits.

All views, buffers, pipelines, queues and targets in a composed frame must belong
to the same native device. Multiple independent instances/devices can coexist;
resources are not interchangeable between them. Native error scopes report
validation, internal and allocation errors. Invalid cross-instance native usage
can still trigger upstream assertions; Quasar does not add a second ownership
registry around native objects.
