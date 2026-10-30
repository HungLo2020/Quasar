# Shadow mapping and normal-mapped surfaces

`SurfaceRenderState3D` extends the classic forward lighting path with optional
normal maps and explicit directional/spotlight shadow maps. The legacy and lit
renderers, their public material structs and shader layouts remain compatible.
This adds no scene, object, ECS, asset loader, window or frame-runner dependency.
All resources and commands use the caller's VulkanicGAL.

## Inputs and pass composition

Applications supply `SurfaceDraw3D` values containing a mesh, model matrix and
`SurfaceMaterial`. The latter wraps an existing `LitMaterial`, an optional
`NormalMap` and a `receives_shadows` flag. To use normal maps alone, construct a
surface renderer with a shadow budget of zero and pass `None` for shadows.
An internal eight-byte depth array satisfies the fixed shader interface; no
shadow renderer, shadow passes or map-sized allocations are needed.

A `ShadowRenderState3D` consumes borrowed `ShadowView` slices. Each view supplies
an explicit light index, light view/projection matrix, filtering/bias settings
and caster slice. `SurfaceDraw3D::shadow_caster()` extracts mesh, transform,
albedo view, alpha cutoff and opacity from a surface draw. Applications decide
which meshes cast and receive shadows independently. World storage and visibility
selection remain game-owned; future ECS/node adapters can extract these same
plain values without changing the renderer.

```rust
let mut surfaces = SurfaceRenderState3D::new(&mut gal, 16, 2)?;
let mut shadows = ShadowRenderState3D::new(&mut gal, ShadowConfig::default())?;
let mut material = SurfaceMaterial::from(LitMaterial::textured(&albedo_view));
material.normal_map = Some(NormalMap::new(&normal_view));
let draws = [SurfaceDraw3D { mesh: &mesh, model, material }];
let casters: Vec<_> = draws.iter().map(SurfaceDraw3D::shadow_caster).collect();
let maps = shadows.prepare(&mut gal, &[ShadowView {
    light_index: 0, // Index in this receiver's Lighting::lights slice.
    view_projection: light_view_projection,
    settings: ShadowSettings::default(),
    casters: &casters,
}])?;
let receiver = surfaces.prepare(
    &mut gal, target, camera, lighting, &draws,
    Some(maps.binding()), PassOptions::default(),
)?;
// Submit maps.commands BEFORE receiver.commands on this GAL.
// Direct GAL passes and quasar-render-2d can share this submission and target.
```

Preparation records commands without submitting, acquiring or presenting a
frame. Preserve normal color/depth transitions when composing custom or 2D
passes; see [the lighting contract](lighting-3d.md). Shadow depth attachments
are separate from world/weapon depth. No framebuffer copies are involved.

## Shadow contract

Directional shadows use caller-selected orthographic bounds, supported by
`Mat4::orthographic`. Spotlight shadows use a perspective projection with a
field of view covering twice the light's outer cone half-angle. Matrices use
right-handed coordinates and [0,1] clip depth, with the same GAL raster convention
as receiver passes. Keep each matrix aligned with its referenced light.

Each view renders a cleared Depth32Float array layer through a depth-only pass.
Alpha-tested casters sample albedo alpha multiplied by opacity and discard
below their cutoff. Blended transmission is not modeled; omit glass or choose
an explicit cutoff. Custom vertex deformation/skinning must supply matching
caster geometry; these passes currently use rigid mesh/model transforms.

Receiver shaders perform perspective division, reject points outside the map
or behind a spot camera, and compare projected depth. Outside coverage is fully
lit. `ShadowFilter` selects one comparison (`Hard`), a 3×3 PCF kernel or a 5×5
kernel. Taps outside the map are lit. Visibility multiplies only the associated
light's direct diffuse/specular response; ambient and emission survive.

Bias controls are explicit: pipeline raster bias, normalized-depth receiver
`bias`, world-unit `normal_bias`, and a 0..=1 shadow `strength`. Normal bias uses
the geometric normal rather than normal-map bumps. Tune these to the scene's
scale and projection range; excessive bias detaches shadows and insufficient
bias causes self-shadow artifacts. There is no automatic fitting, cascade
selection, light discovery or hidden scene ownership. Optional per-light
frustum culling and instanced submission use the
[submission policy](render-submission.md).

Current scope is one map per directional/spot light. Point-light cube shadows,
cascades, PCSS/contact hardening, temporal filtering, baked lighting remain future work. Conservative shadow caster culling is
available through the optional visibility feature. The fixed directional volume can lose detail
in a large world; callers control coverage and resolution now.

## Normal-map contract

`NormalMap` accepts sampled single-layer RGBA8 UNORM views. RGB stores a
linear tangent-space normal in [0,1]; alpha is ignored. Do not sRGB-decode it.
Sampling is linear/repeating. `uv_scale` may include negative components for
mirroring; `flip_y` handles authored opposite-Y conventions. Strength zero
exactly retains the geometric normal; the validated strength range is 0..=8.

The shader derives a tangent frame from world-position and UV derivatives,
orthogonalizes it against the interpolated inverse-transpose normal, and
preserves UV handedness. Non-uniform/mirrored model transforms work without
changing the existing vertex layout. Degenerate UV frames fall back to the
geometric normal. Authored tangent continuity at seams and exact MikkTSpace
matching are not provided by this derivative method. There is no height/parallax
mapping or metallic/roughness PBR in this path.

## Budgets, ownership and lifetime

Surface construction bounds lights to 0..=4096 and shadows to 0..=16. Shadow
construction bounds maps to 1..=16, resolution to 32..=4096 and total caster
draws across all maps to 1..=4096. Actual device capabilities can impose lower
limits. Overflow, non-finite inputs, invalid texture types, duplicate shadow
light indices and missing/point-light receiver references return errors.

A preparation creates an immutable native depth-array generation. The per-generation
budget `max_depth_bytes` validates `max(2, max_maps) * resolution² * 4` bytes.
The default two 1024² layers use 8 MiB per independent atlas. Applications must bound
the number of outstanding frames/bindings to bound aggregate memory; wgpu owns
completion and lifetime. There is no artificial three-frame registry.
Per-caster storage uses 144 bytes and receiver records use 96 bytes plus a
16-byte header; surface draw data uses 208 bytes per instance.

Producer and consumer command buffers retain native resources. Order producers
before receivers. A retained ShadowBinding keeps its view and metadata alive;
custom native consumers bind `texture_view()` normally, with no receipt command.
Reordered preparations use isolated arrays. Resizing or releasing caches cannot
invalidate already encoded work. Drop old owners when replacing generations;
native commands retain needed references. Total memory grows with caller-held
outstanding generations, and preparing a new atlas incurs native allocation cost.

Applications submitting whole frames in queue order can create a `ShadowTarget`
with `create_target` and use `prepare_into(&gpu, views, &target)` to reuse the
native depth array and layer views. Submit a producer and all its receivers before
another frame writes this target. Reordering whole producer/receiver frame batches
is supported; interleaving producers ahead of another frame's receivers is not.
Use ordinary `prepare` for independent generation isolation. Nightwatch opts into
this reusable target, recreating it transactionally when shadow resolution changes.

Instance buffers and bindings are cached and grow as needed. Every preparation
encodes its immutable staging copies before its shadow draws, so pending frames
retain their own caster transforms even when a target/buffer is reused.

## Surface shader extension

Custom fragment stages use WGSL and entry point `main`, preserving the shared
[lighting binding layout](lighting-3d.md#native-ordering-ownership-and-custom-shaders).
Lit and surface paths share 208-byte instance records. Native pipeline validation
reports incompatible stage/resource declarations; no GLSL or SPIR-V rewriting is
performed. Normal/shadow behavior remains explicit in material flags and bindings.

## Validation

GPU tests cover directional and spotlight occlusion, projection orientation,
independent light/array layers, hard/PCF filtering, strength, moving casters,
albedo/opacity cutouts, emission, receiver opt-out, normal-map-only operation,
mirrored UVs, authored Y convention and non-uniform scale. Lifetime tests cover
reversed frame order, retained producer/consumer/binding reservations, bounded
generation isolation, resize, cache reuse and native ownership teardown.
Nightwatch pixel tests exercise runtime toggles, reduced map resolution,
world/weapon/HUD composition, flashlight changes and target resizing.

```sh
cargo test --locked -p quasar-render-3d -p nightwatch
cargo test --locked -p quasar-render-3d --test surfaces -- --ignored --test-threads=1
WGPU_BACKEND=vulkan cargo test --locked -p quasar-render-3d --features vulkan \
  --test surfaces -- --ignored --test-threads=1
cargo test --locked -p nightwatch --test render -- --ignored --test-threads=1
```

GPU tests are ignored by default and require an available device. Vulkan and
wgpu runs on Linux establish local behavior, not Windows/macOS hardware parity.
GLES compilation is separate from runtime support: the upstream-selected GLSL
3.3 target on this host cannot compile the required storage buffers.
