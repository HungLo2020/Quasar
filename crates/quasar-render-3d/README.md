# quasar-render-3d

An optional mesh renderer consuming VulkanicGAL's public API. No window,
application loop, scene graph, ECS, asset decoder, or frame coordinator is
required. This is an initial mesh/material layer, not a full game world renderer.
These APIs were introduced in Git tag `2.1`. The current release is `v0.2.17`,
with Cargo package versions set to `0.2.17`. The crate defaults to the `vulkan`
feature; enable `opengl` with `default-features = false` when using an
OpenGL-backed GAL.

- `Mesh::upload` accepts indexed triangles with positions, normals and UVs.
- `Mat4` supplies column-major transforms and right-handed perspective with
  Vulkan [0,1] clip depth. Pass explicit model and view/projection matrices.
- `Material` samples a caller-owned same-GAL texture view with tint, optional
  alpha blending, and explicit depth-write policy.
- `Camera3D` supplies the view/projection matrix and a directional light. Shaders
  apply directional diffuse light plus ambient light and transform normals.
- `RenderState3D::prepare` records into an existing color/depth target and returns
  normal GAL commands. Color/depth load or clear policy is explicit.

Draws preserve caller order. Sort opaque/transparent objects as appropriate;
there is no automatic scene sorting, animation or PBR. Optional frustum
culling is available through the submission policy described below. Optional
directional/spotlight shadow mapping is described below.
Meshes are uploaded once into host-visible GAL buffers. Per-frame uniforms use
three bounded completion-tracked slots; geometry is not uploaded each frame.
Renderers never acquire a surface or present it. First-use mesh uploads submit
on the same GAL queue without waiting; caller-recorded draws follow that queue.

Use the same GAL for construction, meshes, materials, preparation and release.
The current shaders require explicit-binding GLSL and [0,1] clip depth. Unsupported
conventions return an error. Linux Vulkan has been exercised; other platforms
and OpenGL shader support are not established by these tests.

Prepared commands retain `SubmissionUsage` receipts for both uniform slots and
meshes. Keep the receipts intact. Command clones reserve resources too. Pending
commands prevent `release`, `release_targets`, `clear_bindings` and mesh release.
Submitted uses retire through GAL; buffers are reused only after completion.
When all three slots have pending commands, preparation returns `InFlight`.
When slots contain submitted work, bounded backpressure can wait for the oldest
eligible submission. It does not wait for device idle.

Invalidate pass caches with `release_targets` around target ownership changes.
When replacing meshes/material generations, use `clear_bindings` so descriptor
caches stop retaining the old geometry/views. Release renderer resources and
meshes explicitly with `release(&mut gal)`; these methods are idempotent. Dropping
an owner without release leaves resources with GAL until GAL teardown.

See [the runnable cube/HUD example](../../examples/composition/README.md) for
mesh creation, shared textures, a 2D overlay, native surfaces, readback, cleanup,
GPU tests and benchmark commands.

## Opt-in per-pixel lighting

`LitRenderState3D` adds a forward lighting/material path. `RenderState3D`,
`Camera3D`, `Material`, `Draw3D` and the legacy shaders retain their existing API
and output. Existing consumers do not allocate light buffers or adopt the new
path unless they construct a lit renderer.

```rust
use quasar_render_3d::{
    Attenuation, ColorSpace, Light, Lighting, LitCamera3D, LitDraw3D,
    LitMaterial, LitRenderState3D, Mat4, PassOptions,
};

// `gal`, `target`, `mesh` and `texture_view` belong to the application.
let mut renderer = LitRenderState3D::new(&mut gal, 64)?;
let lights = [Light::Point {
    position: [2.0, 3.0, 1.0], color: [1.0, 0.6, 0.2],
    intensity: 2.0, range: 12.0, attenuation: Attenuation::Smooth,
}];
let prepared = renderer.prepare(
    &mut gal, target,
    LitCamera3D { view_projection, position: camera_position },
    Lighting {
        ambient: [0.03, 0.04, 0.06], lights: &lights,
        output_color_space: ColorSpace::Linear,
    },
    &[LitDraw3D {
        mesh: &mesh, model: Mat4::IDENTITY,
        material: LitMaterial::textured(texture_view),
    }],
    PassOptions::default(),
)?;
// Order `prepared.commands` alongside 2D and custom GAL passes, then submit.
```

The input structs are plain Rust data and borrowed slices. They contain no
entity/node IDs, scene storage, scheduler or application loop. A future ECS or
scene adapter can extract these values from its own storage; an ordinary game
can construct exactly the same values directly. The renderer owns its caches
and upload slots, while applications own meshes, sampled views, targets and GAL.

See [3D lighting architecture](../../docs/lighting-3d.md) for shader extension,
color space, bounded resources, depth transitions, validation and the staged
capability matrix. [Nightwatch](../../examples/nightwatch/README.md) demonstrates
lit world/weapon passes, direct GAL barriers and a 2D HUD on one device/frame.

## Optional shadows and normal maps

`SurfaceMaterial` wraps `LitMaterial` with a normal map and shadow-reception flag;
`SurfaceRenderState3D` records the extended shading path. Pass `None` for shadows
to use normal maps independently. Existing meshes and legacy/lit APIs are unchanged.

`ShadowRenderState3D` accepts explicit borrowed caster slices and light matrices,
then returns depth-map commands and a completion-tracked binding for receivers.
It supports directional and spotlight maps, alpha cutouts, raster/depth/normal
bias, hard comparison and 3×3/5×5 PCF. Maps use bounded per-frame array slots;
there is no required scene registry, ECS or frame runner. Applications retain
control of map coverage, caster selection, draw order and GAL submission.

See [shadows and normal-mapped surfaces](../../docs/shadows-3d.md) for usage,
resource budgets, ordering/lifetime requirements, shader ABI and limitations.
[Nightwatch](../../examples/nightwatch/README.md) demonstrates sun/flashlight
shadows, moving rigid-part casters, procedural concrete/metal normal maps,
a first-person weapon and a 2D HUD in one GAL frame.

## Instancing and visibility

`with_submission` constructors accept an optional `SubmissionConfig` for the
legacy/lit/surface and shadow renderers. Compatible consecutive opaque draws
share instanced draws with per-instance materials; ordering is preserved.
The optional `visibility` feature reuses quasar-spatial for independent camera
and light-frustum tests against uploaded mesh bounds. The old constructors and
custom-shader ABI retain their existing behavior.

`work_stats()` separates candidate/visible/culled instances from physical draw
counts and instance upload bytes. Optional `config` and `diagnostics` features
provide existing Config adapters and explicit Diagnostics handles. See
[submission and reusable benchmarks](../../docs/render-submission.md) for APIs,
budgets, shader interfaces, lifetime contracts and example configuration.

## Optional uploaded models and textures

The `model-assets` feature adapts quasar-assets CPU model data to the caller's
GAL. `UploadedModel` owns a replaceable GPU generation and exposes ordinary
surface draws; `UploadedTexture` provides RGBA uploads, color/normal-aware mips
and explicit sampler ownership. Applications opt into classic PBR approximation
and retain control of asset loading, posing, replacement and submission.

Imported materials preserve alpha behavior, single/double-sided policy and
mirrored winding. Existing mesh/material APIs and default face policy remain
available. See [model asset architecture](../../docs/model-assets-3d.md) for
budgets, sampler registration and command/descriptor lifetime requirements.
