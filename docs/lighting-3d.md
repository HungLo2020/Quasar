# 3D lighting and materials

This delivers the 3D lighting foundation of issues #13/#47, with the ownership
boundaries required by #52. It does not complete their broader rendering or
scene/ECS roadmaps. Quasar's renderer remains usable directly with VulkanicGAL,
alongside `quasar-render-2d`, from any ordinary Cargo application.

## Ownership and composition

`LitRenderState3D` owns shader/pipeline/descriptor caches and three bounded
upload slots on the GAL supplied by its caller. It does not create a device,
window, surface, frame coordinator, scene graph or ECS, and preparation does not
submit rendering. `Mesh::upload` retains its existing explicit upload submission
on that same GAL queue.

Inputs are `LitCamera3D`, a slice of `LitDraw3D`, and `Lighting` with a borrowed
light slice. Games own transforms, material parameters, light selection, draw
ordering and update cadence. A future node/component/ECS integration can extract
these values without adding dependencies to the renderer. Instances, blocks,
actors and draw calls acquire no implicit engine identity.

`prepare` returns ordinary `Prepared3D` GAL commands and statistics. Applications
order those commands with 2D/HUD/UI and custom GAL passes, then submit/present
through either their own orchestration or optional `quasar-frame`. All resources
must belong to the construction GAL. Existing legacy `RenderState3D` consumers
retain their shader behavior, 160-byte draw payload and no light-buffer cost.

## Implemented capabilities

| Capability | Status and contract |
|---|---|
| Ambient | Caller-supplied linear RGB, independent of camera |
| Directional lights | Multiple colored lights with intensity; direction follows emitted rays |
| Point lights | Position, color, intensity, finite range and selected attenuation |
| Spotlights | Point parameters plus direction and inner/outer cone half-angles in radians |
| Diffuse/specular | Per-fragment Lambert and optional Blinn–Phong response |
| Unlit | Ignores light lists and ambient; retains texture, tint, emission and alpha |
| Materials | Linear tint/specular/emissive, shininess, explicit texture encoding, alpha cutoff and existing blend/depth-write policy |
| Normals | Inverse-transpose model transform, normalized after interpolation; non-uniform/mirrored scales supported |
| Shader extension | Caller-supplied fragment shader with the documented lit ABI; default source exported as `DEFAULT_LIT_FRAGMENT_SHADER` |
| Mixed passes | Lit/legacy 3D, direct GAL and 2D on shared targets/device/frame |
| Resource lifetimes | Existing submission receipts and explicit release, including light upload slots |
| Backends | Backend-neutral GAL API and GLSL; verification notes below distinguish actual runs from compile checks |

Normal maps/tangents, metallic/roughness PBR, lightmaps/probes, shadow maps,
HDR exposure/tonemapping, clustered assignment and postprocessing remain future
stages. This milestone does not add dynamic 2D lighting. The optional fragment
shader extension allows custom shading now; custom resource layouts or additional
passes use direct GAL composition instead of being silently forced into the
built-in material layout.

## Light budget and attenuation

Construction accepts a budget of 0..=4096 lights, with an explicit error if a
pass exceeds it. The default forward shader loops over every supplied light per
fragment. The ceiling is a storage/validation bound, not a recommended real-time
budget; callers choose a practical limit and perform selection/culling as needed.
No light is silently dropped or assigned by an implicit world manager.

Directions are normalized during encoding. Non-finite values, zero-length
directions, negative colors/intensities, non-positive ranges and invalid spot
cones are rejected before recording. Spot half-angles require
`0 <= inner < outer < pi/2`. Ambient, light, tint, emission and specular RGB are
linear; alpha remains linear coverage. Lit models must be finite, affine and
invertible so the normal transform is defined.

For distance `d` and range `r`, the smooth attenuation window is
`max(1 - d*d/(r*r), 0)^2`. `InverseSquare` multiplies it by
`1 / max(d*d, 0.01)`. Spot attenuation additionally interpolates between the
cosines of the outer and inner cone angles. Point lights do not implicitly cast
shadows; emissive surfaces do not implicitly create lights.

## Color space and blending

Current GAL textures/targets used here are UNORM or floating point, without an
automatic sRGB transfer. `LitMaterial::texture_color_space` selects explicit
sRGB decoding for encoded color textures; numeric/linear textures stay linear.
Lighting and emission are evaluated in linear space. Alpha is never decoded.

`Lighting::output_color_space` selects linear output or explicit sRGB encoding.
Use linear output for HDR targets, transparent materials and lighting/effect
composition, followed by a caller-owned final encoding/tonemapping pass. The
renderer rejects transparent blending with encoded sRGB output, since that
would blend lighting in nonlinear space. Clear colors are already in the target's
chosen encoding. The sample uses opaque encoded world/weapon passes followed by
a display-space 2D HUD. No HDR tone mapping is implied by that sample.

## Depth and resource transitions

`PassOptions` converts to `LitPassOptions`: a first clear assumes undefined depth,
while loading assumes `DepthStencilAttachment`. For a later pass that clears
previously written depth, pass an explicit policy:

```rust
LitPassOptions {
    pass: PassOptions { clear_color: None, clear_depth: true },
    depth_before: TextureUsageState::DepthStencilAttachment,
}
```

This is the world-to-first-person-weapon case: clear the depth values while
preserving the dependency on earlier writes. Other prior depth uses require an
explicit GAL transition into an accepted state before this pass. Color-target
transitions and ordering between custom passes remain the application's
responsibility, as with the legacy renderer. Nightwatch inserts direct GAL
attachment barriers between offscreen passes; native frame-target attachment
transitions remain GAL-managed.

Each slot holds draw uniforms and one light storage buffer. Light storage is
allocated once at `48 + max(1, budget) * 64` bytes per slot. Each prepare uploads
only the 48-byte header and 64 bytes per actual light, plus the existing aligned
per-draw payloads (192 bytes of data per lit draw). No light data is duplicated
per mesh. CPU preparation creates temporary encoded input/command vectors; this
is not an allocation-free API.

The three-slot policy follows legacy rendering: pending commands reserve their
slots; all-pending pressure returns `InFlight`. Submitted pressure may wait for
the oldest eligible submission. Completed slots can be reused without resource
creation. Cloned commands retain reservations. Release and cache invalidation
reject outstanding unsubmitted uses. Submitted resources retire through GAL;
no unconditional device-idle wait is added.

`Prepared3D.stats.uniform_upload_bytes` describes aligned draw-uniform uploads.
Light-storage writes are additionally visible in GAL's host-write counters and
`quasar-profile` frame samples.

## Fragment shader extension ABI

`LitRenderState3D::with_fragment_shader` uses the normal lit vertex stage and a
caller-provided GLSL fragment stage. Pipeline creation reports shader/interface
errors through GAL. Start from `DEFAULT_LIT_FRAGMENT_SHADER` when retaining the
built-in lighting and replacing a shading/style term.

| Binding/location | Layout |
|---|---|
| Set 0, binding 0 | Combined 2D texture/sampler |
| Set 0, binding 1 | Read-only storage geometry; three vec4s per vertex: position, normal, UV |
| Set 0, binding 2 | Dynamic std140 draw uniform: mat4 viewProjection, mat4 model, vec4 tint, vec4 specularShininess, vec4 emissiveCutoff, vec4 options |
| Set 0, binding 3 | Read-only std430 light storage: vec4 ambient, vec4 eye, vec4 info, followed by four-vec4 light records |
| Fragment input 0 | vec2 UV |
| Fragment input 1 | vec3 world position |
| Fragment input 2 | vec3 world normal |
| Fragment output 0 | vec4 target color |

Draw `options.x` is unlit/diffuse/Blinn–Phong (0/1/2); `options.y` is sRGB texture
decode (0/1). The light header's `info.x` is the light count and `info.y` is the
output sRGB flag, both stored as floats. Light records contain position/type
(0 directional, 1 point, 2 spot), normalized direction/range, color/intensity,
and inner cosine/outer cosine/attenuation (0 smooth, 1 inverse-square)/reserved.
Reserved fields are zero. The shader sources are the authoritative ABI example.

## Validation

- CPU tests cover invalid light/cone data and affine normal-transform requirements.
- GPU pixel tests cover colored/mixed lights, point falloff, spotlight rejection,
  specular response, unlit/emissive/cutout behavior, explicit color encoding,
  non-uniform-scale normals and custom fragment shaders.
- Composition tests mix legacy/lit 3D, direct GAL barriers and 2D; lifetime tests
  cover pending receipts, three-slot pressure, reversed submission order, reuse
  without resource churn and balanced teardown.
- Nightwatch tests cover game rules, headless repeatability, collision/navigation,
  weapons/reloads, enemy damage, waves/restart, visual changes and resize/teardown.

```sh
cargo test -p quasar-render-3d
cargo test -p quasar-render-3d --test lighting -- --ignored --test-threads=1
cargo test -p nightwatch --no-default-features
cargo test -p nightwatch --test render -- --ignored --test-threads=1
```

GPU tests require a working backend/device and are explicitly ignored during
ordinary CPU CI. Linux Vulkan is exercised locally. Backend-specific and native
verification results are recorded in the Nightwatch example README; compilation
alone does not establish Windows/macOS or hardware parity.
