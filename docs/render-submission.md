# Instancing, visibility and reusable benchmarks

Native graphics APIs now use unmodified wgpu through VulkanicGAL. See
[native migration](native-wgpu-migration.md) for native views/command buffers, WGSL, Drop ownership,
backend selection and replacement of old slot/receipt APIs. Historical benchmark
figures describe their recorded release.

Render submission remains an optional layer over a caller-owned GAL. Games
supply borrowed mesh/material/model values; no object identity, scene, ECS,
window, runner or frame owner is introduced. A future adapter can extract the
same values from components or nodes.

## Opt-in rendering policy

```rust
let submission = SubmissionConfig {
    instancing: true,
    frustum_culling: true,
    max_instances: 4096,
};
let mut surfaces = SurfaceRenderState3D::with_submission(&mut gal, 16, 2, submission)?;
let mut shadows = ShadowRenderState3D::with_submission(&mut gal, shadow_config, submission)?;
// prepare uses the existing SurfaceDraw3D / ShadowView slices and GAL commands.
```

The legacy, lit and surface renderers all provide `with_submission`; their old
constructors retain scalar rendering and the existing custom-fragment ABI.
`SubmissionConfig::default()` disables both optimizations. Nightwatch opts into
both. The `visibility`, `config` and `diagnostics` features independently enable
spatial culling, configuration adapters and diagnostics hooks. They are absent
from the renderer's default dependency set. Requesting culling without the
visibility feature returns `UnsupportedFeature` at construction.

Instancing groups compatible **consecutive** opaque draws with the same mesh,
texture bindings and pipeline. It preserves order and never sorts a scene.
Transparent draws retain individual draws and caller ordering. Each instance
retains its own model, tint, shading, specular, emission, cutoff, normal-map
parameters and shadow-reception flag. Both receiver and depth-only shadow passes
use this path. Different materials/views, pipelines and meshes split batches.

Built-in WGSL shaders use read-only draw storage at binding 7. Native
`first_instance` selects records and a flat vertex output at location 7 selects
per-instance fragment data. Records are 160 legacy, 208 lit/surface and 144 shadow
bytes. Scalar and instanced modes share this native shader layout for pixel parity.
Custom shaders use the WGSL interface documented in [lighting](lighting-3d.md).

Native destination buffers and bindings are cached and grow as needed. Each
preparation owns immutable staging data and encodes its copies before its draws;
its native command buffer retains all resources.
Arbitrary pending/reordered preparations stay isolated; there is no custom
three-slot receipt registry. The configured per-preparation ceiling remains
1..=4096 instances; shadow inputs also respect total caster budgets. Only visible
records are uploaded. Native allocation and temporary CPU vectors mean this API
is not allocation-free. Applications bound outstanding frames to control aggregate
memory, and native wgpu manages completion and lifetime.

`work_stats()` returns the most recent preparation's candidates, visible and
culled instances, physical draw count and instance storage upload bytes.
`Prepared3D.stats.draws` and `ShadowStats.draws` count physical draws; existing
uniform upload counters describe uniforms, with instance storage separate in
work stats. Upstream does not expose equivalent private host-write counters.
Capture work stats after each successful prepare when accumulating a frame. Counts refer to pass instances, so a caster
rendered into two lights is counted twice.

## Conservative visibility

Meshes retain local bounds computed at upload. The optional integration reuses
`quasar-spatial::Frustum::from_view_projection` and
`Aabb::transformed_affine`, which accept ordinary column-major f64 matrices and
are independently usable without rendering. Frustum extraction supports both
zero-to-one and negative-one-to-one clip depth, including infinite far planes.
Affine bounds handle rotation, shear, non-uniform and mirrored scale.

Receiver culling uses the camera's view/projection. Each shadow view separately
culls its original caster slice against that light's view/projection. Camera
rejection never removes an off-camera shadow caster from the light's inputs.
Plane/AABB tests are conservative; intersecting or enclosing boxes remain.
Bounds receive a relative floating-point margin. Unsupported/projective or
numerically invalid culling inputs fail open, leaving rendering validation to
the original path. There is no occlusion culling, spatial index requirement,
implicit world storage or automatic light-volume fitting.

## Existing configuration and diagnostics

With the `config` feature, renderer policy provides `register_defaults` and
`from_config` for a caller-selected key prefix. These use `quasar-config` getters
and precedence; they read no environment or files themselves. Games select their
own policy defaults, file formats, env mappings and overrides.

With `diagnostics`, `set_diagnostics` accepts an explicit handle. Preparation
spans use that handle's span level; debug `submission_work` events contain the
work counts. Disabled handles do not emit records or call span clocks. No global
logger or per-object event stream is installed.

`quasar-profile::BenchmarkSession` reuses `FrameProfiler` and
`ProfileThresholds`: bounded retention, configurable warmup, actual p99 checks,
and caller-supplied bounded metadata. Optional `config` and `diagnostics`
features provide the same prefix-based config adapters and a structured final
summary/failure. GPU limits require valid timestamps for every measured frame;
empty samples or unavailable GPU timing cannot pass a configured budget.

Renderer preparation reuses FrameReport's existing command-generation timing,
so every profiling consumer gets the same measurement without adding an
application-specific timer. Additional sample/summary fields distinguish that
preparation, GAL
validation, hazard analysis, backend encoding/submission, visible/culled
instances and instance uploads. Old JSON fields remain at the top level and new
fields deserialize with defaults. Session artifacts add a `benchmark` object
with config, context metadata and observed-frame count. No implicit device,
host or user-information collection occurs in the profiling library.

## Nightwatch configuration

Nightwatch resolves defaults < TOML/JSON file < explicitly mapped environment
< command-line flags < repeatable `--set` overrides. It never rewrites a file.
Unknown file keys are preserved by Config; unknown explicit overrides fail.
Simulation-only builds retain configuration/benchmark modeling without pulling
in renderers, a window, audio or the application runner.

```toml
[render]
instancing = true
frustum_culling = true
max_instances = 4096
validation = "auto" # GAL build/env default; true/false explicitly override it
shadows = true
normal_maps = true
shadow_resolution = 1024

[benchmark]
warmup_frames = 8
sample_capacity = 600
max_draws_p99 = 100
max_resource_creates_p99 = 4
max_host_write_bytes_p99 = 200000
# Time limits are nanoseconds. Zero means unbounded for every max_* setting.
max_cpu_frame_p99 = 0
max_gpu_frame_p99 = 0
max_acquire_p99 = 0
max_present_p99 = 0
```

```sh
cargo run --release -p nightwatch -- --autoplay --no-audio --frames 180 \
  --benchmark-file /tmp/nightwatch.json --config nightwatch.toml
cargo run --release -p nightwatch -- --autoplay --no-audio --frames 180 \
  --benchmark-file /tmp/scalar.json --no-instancing --no-culling --validation
```

Common options retain their `common.*` config keys; simulation options use
`game.*`. Flags include `--no-instancing`, `--no-culling`, `--validation` and
`--no-validation`. Renderer env mappings are `NIGHTWATCH_INSTANCING`,
`NIGHTWATCH_FRUSTUM_CULLING`, `NIGHTWATCH_VALIDATION`; common settings use the
corresponding `NIGHTWATCH_*` names. `--log-level` accepts trace/debug/info/warn/
error or off. Nightwatch sets span level to debug, so normal info-level logs
contain setup/final benchmark reports rather than per-frame spans.

Native validation and driver debug behavior are upstream InstanceDescriptor
options. Hold `WGPU_VALIDATION`/`WGPU_DEBUG` and adapter selection fixed across
matched runs. Compare CPU preparation/submission and GPU work separately from
presentation. No cross-device thresholds are implied.

## Repeated command runner

`tools/benchmark_runs.py` is application-independent: it runs argument lists
without a shell, substitutes an artifact path, repeats cases and collects the
raw profile JSON/logs plus a comparison file. It does not build code, create a
window server, change validation or infer game flags. Supply an existing DISPLAY
or wrap it in the environment used for native tests.

```json
{"cases":[{"name":"optimized","command":["target/release/nightwatch",
"--autoplay","--no-audio","--frames","180","--benchmark-file","{output}"],
"env":{"WGPU_VALIDATION":"1"}}]}
```

```sh
python3 tools/benchmark_runs.py --spec cases.json --output /tmp/render-runs --repeats 3
```

The runner rejects empty measurements, failed processes and existing output
files. Raw runs remain available for inspecting variance; it does not silently
average away outliers or impose timing limits across machines.

## Imported model generations

Optional [model assets](model-assets-3d.md) use this same draw/instance path with
per-view sampler policies. Clearing binding caches resets those associations;
rebind live model samplers after cache invalidation. Surface draw `options.z`
is now a small flag mask: bit 0 receives shadows, bit 1 flips backface normals
for double-sided materials, and bit 2 forces opaque alpha. The new flags are
selected only by explicit mesh face/alpha policy; legacy meshes retain prior
behavior. Scalar and instanced modes share the native WGSL layout.
