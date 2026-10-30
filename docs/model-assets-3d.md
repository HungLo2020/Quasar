# Model loading and GPU assets

The CPU model layer extends `quasar-assets`; the optional GPU adapter lives in
`quasar-render-3d`. Model decoding does not depend on GAL, an editor, an ECS,
scene storage or a window. Renderer consumers can still upload raw meshes and
textures directly on their own GAL. Games decide which assets to instantiate,
where to put them and when to apply replacements.

## CPU assets and mounted dependencies

Enable `quasar-assets`'s `model` feature for renderer-neutral mesh, material,
image, sampler and instance types. The `gltf` feature adds the glTF 2.0/GLB typed
loader and PNG/JPEG codecs. All format dependencies are optional; the default
asset crate remains dependency-free.

```rust
let mut assets = AssetServer::with_budget(CacheBudget::new(64 * 1024 * 1024, 32));
assets.mount_file("", "assets")?;
assets.register(GltfLoader::default());
let model = assets.load::<GltfLoader>("models/prop.glb")?;
// Plain CPU arrays: model.primitives, materials, textures, images, instances.
```

`Model3D` contains indexed triangle primitives with positions/normals/UV0,
linear material factors, image bytes, sampler policy and borrowed-data-friendly
instance records. Each instance references a primitive and has a file-local
node index/name plus a composed transform. These are import metadata, not
engine object identities or a runtime node system. Hierarchy transforms are
composed into extracted instances; the source file's default scene (otherwise
first scene) is selected unless `GltfLoader::scene` explicitly selects one.
Meshes without normals are expanded into flat-shaded triangles. Missing indices
are generated. Geometry is right-handed and preserves authored coordinates.

Embedded GLB binary chunks, base64 data URIs and mounted external buffers/images
are supported. Texture references preserve filtering, wrapping, base UV0
transform, normal scale, alpha mode/cutoff and face policy. Material PBR scalar
values and extra map references are retained; the current GPU path is classic
shading rather than a full glTF PBR implementation.

`LoadContext::read_relative` is reusable by any typed loader. It normalizes
file-relative dot segments, rejects paths escaping the virtual root and reads
only through mounted sources. glTF URI percent decoding occurs before path
validation; absolute/network/drive paths, queries and fragments are rejected.
Successful reads automatically join the asset's dependency metadata. Editing an
external image and invalidating its path with `cascade=true` therefore invalidates
its dependent models. Previous typed handles remain valid at their old revision.

`AssetLoader::max_source_bytes`, `AssetSource::read_limited` and
`AssetServer::read_limited` provide format-specific read budgets. Built-in file
and memory sources enforce them before copying whole files; filesystem reads
also reject files growing beyond the limit. Custom sources can override the
bounded read method; its compatibility default checks after their `read` returns.

## Bounds and supported subset

`ModelLimits` bounds root source bytes, total binary-buffer bytes, decoded
vertices/indices, primitive/instance/material/image counts, image dimensions,
decoded image bytes, node count and hierarchy depth. Ranges and component types
are checked before accessor iteration. Hierarchy cycles, multiple parents,
non-finite values, invalid indices and truncated buffers return asset errors.
PNG/JPEG decoding receives dimension/allocation limits; codec allocation limits
are best-effort, while dimensions and decoded RGBA output budgets are checked
explicitly. These are payload/work bounds, not a promise about allocator RSS.

This initial loader supports static triangle geometry and UV0. Sparse accessors,
vertex colors, morph targets, skinning, other primitive modes and unsupported
required extensions are rejected explicitly. Higher texture coordinate sets and
normal-texture transforms are not supported. Authored tangent streams are not
used by the GPU adapter; its existing normal-map shader derives the tangent frame
from derivatives, with the same seam limitations as other surface materials.

Animation clips are rejected by default. `allow_animation_default_pose=true`
explicitly imports the authored default transforms without evaluating clips;
a warning remains in the model. Nightwatch uses this for crate lid assets.
Skins and morph targets remain unsupported under that policy. The policy does
not add a global clock or an animation system.

The existing cache key is loader Rust type plus path. Invalidate cached paths
when changing a registered loader's scene/limits/policy, or use distinct loader
types for concurrently different configurations. `GltfLoader` also works through
the existing bounded `AsyncAssetServer`; watching and applying results remain
explicit application decisions.

## GPU upload and materials

Enable `quasar-render-3d`'s `model-assets` feature. It depends only on the CPU
model types, so callers can construct them without enabling the glTF decoder.

```rust
let mut gpu = UploadedModel::upload(&mut gal, &model, ModelUploadOptions {
    material_conversion: MaterialConversion::ClassicApproximation,
    ..ModelUploadOptions::default()
})?;
gpu.bind_samplers(&mut gal, &mut surfaces)?;
gpu.bind_shadow_samplers(&mut gal, &mut shadows)?;
let draws = gpu.draws(root_transform);
// Extract shadow casters, prepare receiver passes and submit as usual.
```

GPU models own primitive meshes, uploaded textures and samplers on the caller's
GAL. Bounds, direct primitive draws (including scene-less assets) and individual
instance draws are exposed for placement and
application-owned posing. Uploads submit through the same explicit queue used
by existing mesh uploads, without a device-idle wait. There is no implicit
renderer, frame runner, asset server or GPU cache creation.

`ModelUploadOptions` bounds mesh and texture payload bytes. The default material
conversion rejects non-unlit PBR materials; choose `ClassicApproximation`
explicitly to map base color, emission and approximate metallic/roughness
response into classic surface shading. Metallic/roughness, emissive and occlusion
texture maps are rejected by that adapter. Base and normal UV transforms must
agree for its single-UV vertex path. This avoids silently advertising PBR support.

Opaque materials ignore sampled alpha; mask materials share cutoff/opacity with
shadow casters; blend materials retain caller ordering and disable depth writes.
Single-sided meshes enable backface culling with mirrored-transform winding
correction. Double-sided lit/surface shading flips backface normals. Legacy raw
meshes keep their previous face behavior unless an explicit policy is selected.

`UploadedTexture` is independently usable for RGBA upload and sampler ownership.
CPU mip generation is backend-independent: color mips decode/average/re-encode
sRGB, linear mips average numeric values, and normal mips renormalize vectors.
Odd source dimensions include edge texels. Alpha is averaged linearly; there is
no coverage-preserving alpha filter. Color and normal uses of one source texture
receive distinct GPU variants so their mip semantics agree with their role.

Per-view sampler registration on surface and shadow renderers preserves authored
filtering/wrapping in both receiver and cutout passes. Raw consumers retain the
existing default samplers. `clear_bindings` also clears registrations, so rebind
live asset generations after invalidation or shadow resizing.

## Replacement, lifetimes and failure

Upload a complete candidate before swapping application-owned generations. An
upload failure releases partial GPU allocations and leaves the current generation
untouched. Both generations can coexist while their prepared commands remain
queued. Uploaded models reject release while mesh commands retain them; accepted
GPU work retires through GAL, including resources retained by descriptor caches.

Submit or abandon retained commands before clearing receiver/shadow binding
caches, swap generations at the application's chosen boundary, and explicitly
release the old owner. Clearing caches removes old view/mesh references and sampler
associations. Rebind the new owner before subsequent preparation. No stale async
result is applied automatically; callers can compare existing asset revisions.
For direct GAL consumers of an `UploadedTexture`, include its
`submission_usage()` in `CommandOp::TrackSubmission` or retain the owner until
all uses finish. Plain raw handles do not transfer ownership.

Nightwatch stages all seven selected models before replacing any active model.
F5 stages decoding/uploads synchronously as a development reload path. Games
can use the existing async worker to decode CPU data before applying a GPU
generation on their render thread. F5 clears obsolete bindings and requests a
fresh CPU/GPU generation; errors are
reported through diagnostics and the old models remain playable. Procedural floor,
lights, hands and enemies remain game-generated. Named prop definitions now share
placements and fitting anchors with explicit cylinder, cuboid and compound
colliders; see [3D collision](collision-3d.md). Source models
and their license/integrity records are described in
[the asset manifest](../examples/nightwatch/assets/README.md).

## Verification

CPU tests cover mounted/external, GLB and base64 sources, normal generation,
malformed ranges/types/indices/graphs, budgets, dependency invalidation, retained
handles, actual authored files and color/normal mip semantics. Vulkan/wgpu GPU
tests cover generation overlap/reversed submission, failed replacement, opaque
alpha, material face policy, mirrored transforms, sampled mip energy and balanced
teardown. Nightwatch tests exercise exact scalar/instanced/culling parity, changing
textures, successful/failed F5-equivalent reloads, resize and gameplay state.

```sh
cargo test --locked -p quasar-assets --features gltf,async
cargo test --locked -p quasar-render-3d --features model-assets \
  --test model_assets -- --ignored --test-threads=1
VULKANIC_BACKEND=wgpu cargo test --locked -p quasar-render-3d \
  --features model-assets,wgpu --test model_assets -- --ignored --test-threads=1
cargo test --locked -p nightwatch --test render -- --ignored --test-threads=1
```

GPU checks require a device and are ignored by ordinary CPU CI. Linux Vulkan and
wgpu runs establish local behavior, not other operating-system/hardware parity.
The format follows the [glTF 2.0 specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html);
this subset does not claim complete glTF conformance.

## Explicit collision profiles

Models do not acquire collision automatically. Optional
`quasar-physics/model-assets` reads decoded CPU `Model3D` data for bounds,
selected structural instances and an explicitly requested containing Y-cylinder.
`Bounds3::fit_matrix` shares the model anchor and transform with an authored
primitive/compound. Nightwatch uses named assets and explicit collision profiles
without requiring a scene graph. See [3D collision](collision-3d.md) for units,
scale, reload ownership, approximation limits and alignment tests.
