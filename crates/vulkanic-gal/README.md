# VulkanicGAL

Quasar's single dependency and import point for **unmodified upstream wgpu
30.0.1**, fetched from crates.io. Its entire implementation is:

```rust
pub use wgpu;
pub use wgpu::*;
```

All types, validation, resource ownership, command encoding, synchronization,
surface behavior and driver backends are upstream wgpu. There is no private
backend, handle registry, command translator, shader compiler or patched fork.
Use `vulkanic_gal::Device`, `Queue`, `TextureView`, `CommandEncoder`, WGSL and the
ordinary upstream descriptors. The re-exports preserve upstream type identity.
Other workspace crates depend on VulkanicGAL rather than importing wgpu directly;
architecture tests enforce this boundary.

The default features forward `vulkan`, `metal`, `dx12` and `gles`. Select only
needed backends with `default-features = false`. Additional upstream forwarding
features are `webgpu`, `webgl`, `spirv`, `glsl`, `naga-ir`, `counters` and `trace`.
Backend availability still depends on the OS, adapter and enabled features.
Native Quasar shaders use WGSL; Shaderc and a native C++ shader build are gone.

`WGPU_BACKEND=vulkan`, `metal`, `dx12` or `gl` and `WGPU_ADAPTER_NAME` work with
Quasar's optional startup helpers. Low-level callers can instead supply native
`InstanceDescriptor`, `RequestAdapterOptions` and `DeviceDescriptor` themselves.
Inspect `Adapter::get_info()`, `features()` and `limits()` for actual capabilities.
The old `VULKANIC_BACKEND`, `GalOptions`, `VulkanicGal`, numeric handles, barriers,
retirement receipts and separate `wgpu`/`opengl` Cargo features are removed.

[`quasar-frame`](../quasar-frame/README.md) provides optional setup, surface,
readback and timing helpers. Neither VulkanicGAL nor the renderers requires an
ECS, scene graph or application runner. See the
[migration guide](../../docs/native-wgpu-migration.md) for ownership and examples.

```sh
cargo check -p vulkanic-gal --no-default-features
cargo check -p vulkanic-gal --no-default-features --features vulkan
WGPU_BACKEND=vulkan cargo run --release -p vulkanic-gal --example submit_bench
```
