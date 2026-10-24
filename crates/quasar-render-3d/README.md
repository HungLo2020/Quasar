# quasar-render-3d

An optional mesh renderer consuming VulkanicGAL's public API. No window,
application loop, scene graph, ECS, asset decoder, or frame coordinator is
required. This is an initial mesh/material layer, not a full game world renderer.
These APIs were introduced in Git tag `2.1`. The current release is `v0.2.5`,
with Cargo package versions set to `0.2.5`. The crate defaults to the `vulkan`
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
there is no automatic scene sorting, animation, PBR, shadow system or culling.
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
