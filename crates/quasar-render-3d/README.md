# quasar-render-3d

An optional mesh renderer consuming VulkanicGAL's public API. No window,
application loop, scene graph, ECS, asset decoder, or frame coordinator is
required. This is an initial mesh/material layer, not a full game world renderer.
The default `native` feature forwards upstream Vulkan, Metal, DX12 and GLES;
individual backend features are available. WGSL and native resource types flow
through VulkanicGAL. No custom backend, shader compiler or retirement engine is
required. See [native graphics migration](../../docs/native-wgpu-migration.md).

- `Mesh::upload(&gpu, vertices, indices)` validates and uploads indexed triangles.
- `Mat4` uses column-major, right-handed transforms and [0,1] clip depth.
- `Material::textured(&view)` retains a native texture view. Materials are Clone.
- `Camera3D` supplies explicit projection/view and directional light inputs.
- `RenderState3D::prepare(&gpu, &target, camera, draws, pass)` returns a native,
  single-use command buffer and work statistics.

Caller order and color/depth load/clear policy remain explicit. Native command
buffers retain their resources; releasing owners/caches cannot invalidate already
encoded work. Commands can be dropped or submitted in any order; there is no
three-frame registry or cloned command receipt. Use the same native device for
meshes, sampled views, target and renderer. Ordinary Drop releases owner
references; `release` clears state and marks the renderer released.

Compose world, raw native commands and 2D HUD on one queue. Native wgpu derives
transitions; 2D leaves world depth unchanged. The renderer never acquires or
presents a surface. See [composition](../../examples/composition/README.md).

## Opt-in per-pixel lighting

`LitRenderState3D` adds a forward lighting/material path. `RenderState3D`,
`Camera3D`, `Material`, `Draw3D` and the legacy lighting behavior remains available. Existing consumers do not allocate light buffers or adopt the new
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
        material: LitMaterial::textured(&texture_view),
    }],
    PassOptions::default(),
)?;
// Order `prepared.commands` alongside 2D and custom GAL passes, then submit.
```

The input structs are plain Rust data and borrowed slices. They contain no
entity/node IDs, scene storage, scheduler or application loop. A future ECS or
scene adapter can extract these values from its own storage; an ordinary game
can construct exactly the same values directly. The renderer owns its caches
and native buffers, while applications own meshes, sampled views, targets and GAL.

See [3D lighting architecture](../../docs/lighting-3d.md) for shader extension,
color space, bounded resources, depth transitions, validation and the staged
capability matrix. [Nightwatch](../../examples/nightwatch/README.md) demonstrates
lit world/weapon passes, native GPU passes and a 2D HUD on one device/frame.

## Optional shadows and normal maps

`SurfaceMaterial` wraps `LitMaterial` with a normal map and shadow-reception flag;
`SurfaceRenderState3D` records the extended shading path. Pass `None` for shadows
to use normal maps independently. Existing meshes and legacy/lit APIs are unchanged.

`ShadowRenderState3D` accepts explicit borrowed caster slices and light matrices,
then returns depth-map commands and a native texture-array binding for receivers.
It supports directional and spotlight maps, alpha cutouts, raster/depth/normal
bias, hard comparison and 3×3/5×5 PCF. Default preparation uses independent native
arrays within configured budgets. `prepare_into` accepts a reusable `ShadowTarget`
for ordered whole-frame submission, as used by Nightwatch. There is no required
scene registry, ECS or frame runner. Applications retain
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
custom WGSL shaders use explicit native bindings.

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
