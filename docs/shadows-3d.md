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
let mut material = SurfaceMaterial::from(LitMaterial::textured(albedo_view));
material.normal_map = Some(NormalMap::new(normal_view));
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
selection, light discovery or hidden scene ownership.

Current scope is one map per directional/spot light. Point-light cube shadows,
cascades, PCSS/contact hardening, temporal filtering, baked lighting and shadow
caster culling remain future work. The fixed directional volume can lose detail
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

A shadow renderer has at most three completion-tracked atlas/upload slots.
Depth allocation is bounded up front by `ShadowConfig::max_depth_bytes`:
`3 * max(2, max_maps) * resolution² * 4` bytes. The minimum two layers preserve
the array-view type even for a single-map renderer. The default two 1024² maps
reserve a worst-case 24 MiB depth budget; slots allocate lazily. Per-caster
144-byte uniforms use the device's aligned dynamic stride. Metadata costs a
16-byte header plus 96 bytes per map per receiver slot. Receiver draw payloads
are 208 bytes before dynamic-offset padding. Statistics distinguish shadow
caster work from receiver work; GAL counters also include metadata/light writes.

Producer commands, bindings and receiver commands retain a shared
`SubmissionUsage` reservation. Retaining a binding protects its atlas while
preparing consumers even after the producer has been submitted. Multiple
outstanding frames use separate arrays and may be submitted in a different
order without overwriting each other's maps. All-pending pressure returns
`InFlight`; submitted pressure may wait for the oldest eligible completion.
No device-idle wait is introduced. Custom GAL consumers of
`ShadowBinding::texture_view()` must carry its `submission_usage()` in
`CommandOp::TrackSubmission` and follow producer-before-consumer ordering.

Submit or abandon prepared commands and drop retained bindings before clearing
or releasing their owners. Clear receiver descriptor caches with
`SurfaceRenderState3D::clear_bindings` before resizing shadow maps. Clear shadow
caster bindings before retiring their albedo/mesh generations. Release surface
receivers before the shadow renderer, then meshes/textures. Submitted resources
retire through GAL. Target resizing only invalidates receiver target caches;
map resolution changes use the separate explicit `ShadowRenderState3D::resize`.

## Surface shader extension

`with_fragment_shader` and `DEFAULT_SURFACE_FRAGMENT_SHADER` expose the advanced
fragment path. Bindings 0–3 retain the lit interface. Binding 4 is a combined
normal sampler; binding 5 is a comparison `sampler2DArrayShadow`; binding 6 is
read-only std430 shadow metadata: vec4 info, then records containing mat4
viewProjection, vec4 settings, vec4 sampling. Settings hold light index, layer,
depth bias and normal bias; sampling holds inverse resolution, PCF radius,
strength and reserved. `info.x` stores the map count as a float.

The std140 draw uniform appends vec4 normalMap (UV scales, strength, Y sign).
`options.zw` select shadow reception and normal-map use. The existing lit ABI is
unchanged; use the surface source for this extended interface. Different layouts
or passes can still be built directly with GAL on the same device.

## Validation

GPU tests cover directional and spotlight occlusion, projection orientation,
independent light/array layers, hard/PCF filtering, strength, moving casters,
albedo/opacity cutouts, emission, receiver opt-out, normal-map-only operation,
mirrored UVs, authored Y convention and non-uniform scale. Lifetime tests cover
reversed frame order, retained producer/consumer/binding reservations, bounded
slot pressure, resize, reuse without resource churn and balanced teardown.
Nightwatch pixel tests exercise runtime toggles, reduced map resolution,
world/weapon/HUD composition, flashlight changes and target resizing.

```sh
cargo test --locked -p quasar-render-3d -p nightwatch
cargo test --locked -p quasar-render-3d --test surfaces -- --ignored --test-threads=1
VULKANIC_BACKEND=wgpu cargo test --locked -p quasar-render-3d --features wgpu \
  --test surfaces -- --ignored --test-threads=1
cargo test --locked -p nightwatch --test render -- --ignored --test-threads=1
```

GPU tests are ignored by default and require an available device. Vulkan and
wgpu runs on Linux establish local behavior, not Windows/macOS hardware parity.
OpenGL compilation is separate from runtime support: its current shader
conventions are rejected explicitly by the 3D renderers.
