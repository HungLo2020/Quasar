# 3D lighting and materials

This delivers the 3D lighting foundation of issues #13/#47, with the ownership
boundaries required by #52. It does not complete their broader rendering or
scene/ECS roadmaps. Quasar's renderer remains usable directly with VulkanicGAL,
alongside `quasar-render-2d`, from any ordinary Cargo application.

## Ownership and composition

`LitRenderState3D` owns native shader/pipeline/binding caches and growable
instance/light buffers on the device supplied by its caller. It does not create a device,
window, surface, frame coordinator, scene graph or ECS, and preparation does not
submit rendering. `Mesh::upload` creates initialized native vertex/index buffers
on that same device.

Inputs are `LitCamera3D`, a slice of `LitDraw3D`, and `Lighting` with a borrowed
light slice. Games own transforms, material parameters, light selection, draw
ordering and update cadence. A future node/component/ECS integration can extract
these values without adding dependencies to the renderer. Instances, blocks,
actors and draw calls acquire no implicit engine identity.

`prepare` returns ordinary `Prepared3D` GAL commands and statistics. Applications
order those commands with 2D/HUD/UI and custom GAL passes, then submit/present
through either their own orchestration or optional `quasar-frame`. All resources
must belong to the same native device. Existing legacy `RenderState3D` consumers
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
| Resource lifetimes | Native resource retention and Drop |
| Backends | Native wgpu API and WGSL; verification notes below distinguish actual runs from compile checks |

Normal maps and directional/spotlight shadow maps are available through the
additive [surface/shadow path](shadows-3d.md), leaving this lit ABI unchanged.
Authored tangents, metallic/roughness PBR, lightmaps/probes, HDR exposure/tonemapping,
clustered assignment and postprocessing remain future stages. This milestone does not add dynamic 2D lighting. The optional fragment
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

## Native ordering, ownership and custom shaders

PassOptions selects color/depth load or clear behavior. The weapon pass clears
existing depth with `clear_color: None, clear_depth: true`; wgpu derives the
required dependencies without a `depth_before` field or explicit barrier.
Order world, weapon and HUD native command buffers on the same queue.

Native buffers hold per-preparation draw and light data. Reordered or abandoned
command buffers cannot overwrite another preparation. Native command buffers
retain their resources; there is no fixed-slot exhaustion or custom submission
receipt. Caches can be cleared during asset replacement and owner Drop releases
references. Temporary CPU vectors and native buffers allocate during preparation.

`with_fragment_shader` accepts WGSL with entry point `main`. Start from
`DEFAULT_LIT_FRAGMENT_SHADER`; native pipeline validation checks stage interfaces.
Lit and surface shading share the same WGSL binding layout:

| Group 0 binding | Native resource |
|---|---|
| 0 / 16 | Albedo texture / sampler |
| 1 | Read-only vertex storage, three vec4s per vertex |
| 2 | Base-instance uniform (zero; first_instance selects records) |
| 3 | Ambient/eye/info header plus four-vec4 light records |
| 4 / 20 | Normal texture / sampler |
| 5 / 21 | Shadow depth array / comparison sampler |
| 6 | Shadow header and six-vec4 records |
| 7 | Draw storage, 208 bytes per lit/surface instance |

Vertex outputs are UV (location 0), world position (1), normal (2), and flat
instance index (7). Fragment output is location 0. The checked-in WGSL is the
authoritative resource/record interface. There is no runtime GLSL rewriting or
Shaderc compilation. [Migration](native-wgpu-migration.md) describes native APIs.

## Validation

- CPU tests cover invalid light/cone data and affine normal-transform requirements.
- GPU pixel tests cover colored/mixed lights, point falloff, spotlight rejection,
  specular response, unlit/emissive/cutout behavior, explicit color encoding,
  non-uniform-scale normals and custom fragment shaders.
- Composition tests mix legacy/lit 3D, native application passes and 2D; lifetime tests
  cover native resource retention, reversed submission order, reuse
  cache reuse and native ownership teardown.
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
