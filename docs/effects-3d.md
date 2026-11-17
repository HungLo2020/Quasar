# 3D effects, crowds and composition

Features of `quasar-render-3d` beyond basic meshes and lights. Every pass
takes caller-owned targets and returns command buffers, so they compose with
voxels and custom passes on one `SceneTarget`.

## Clustered lights and fog

The shared lighting module (`LIGHTING_WGSL`, used by meshes and voxels)
clusters positional lights in a world-space grid around the camera. Each
cell lists the point and spot lights whose range touches it, so a fragment
shades only nearby lights plus every directional light. Fragments outside
the grid shade every light, so clustering never changes the image.

```rust
renderer.set_light_clusters(LightClusters { min_lights: 8, cell_size: 4.0, dims: [16, 4, 16] })?;
renderer.set_fog(Some(Fog { color: [0.01, 0.015, 0.03], start: 12.0, end: 44.0 }))?;
```

Clustering starts automatically at `LightClusters::default().min_lights`
(12) positional lights. `cluster_lights` returns the CPU grid for tools and
tests. Fog uses the same `Fog` type as `quasar-render-voxel`, so meshes fade
into the same colour as distant blocks; it applies to opaque and transparent
surface passes.

## Vertex colours

`Mesh::upload_colored` stores one linear RGBA colour per vertex (8 bits per
channel) in otherwise unused vertex slots, so the vertex size is unchanged.
Every pass multiplies it into the texture and tint. glTF `COLOR_0` imports
into `ModelPrimitive::colors` and uploads automatically, and skinning
carries colours through. One mesh can now hold a multi-material character.

## Transparency on normal-mapped surfaces

`SurfaceRenderState3D::prepare_transparent` draws `SurfaceTransparentDraw3D`s
(normal maps, shadow receiving, any `BlendMode`) back to front into a linear
target after the opaque pass. Games no longer need a second lit state for
blended draws.

## Skinned crowds

`SkinnedCrowd` holds N posed copies of one skinned mesh in one vertex
buffer. `SkinningPass::prepare_crowds` uploads every palette at once and
poses all instances in one dispatch. Each slot (`crowd.instance(i)`) is an
ordinary `Mesh`. With `SubmissionConfig::instancing`, consecutive draws of
one crowd's slots with the same material become one instanced draw, in
lit, surface, transparent and shadow passes. Use it for hordes; a
`SkinnedMesh` per enemy costs one dispatch, one upload and one draw each.

## Particles

`ParticleSystem` keeps a ring of GPU particle slots. `emit(&ParticleBurst)`
spawns particles with a seeded generator (cone direction, speed, lifetime,
size and colour ranges). `prepare` uploads them, integrates every slot in
one compute step (gravity and drag) and draws live particles as
camera-facing quads in one instanced draw. Particles depth-test against the
target's depth without writing it. Additive particles are
order-independent. `set_sprite` replaces the default soft disc with a
texture's alpha.

## Decals

`DecalRenderer` projects textures onto whatever opaque geometry lies inside
a unit box (local Y is the projection axis), reading the `SceneTarget`'s
depth. It works on meshes, voxels and custom passes alike: scorch marks,
mining scars, footprints. Decals fade towards the box ends (`Decal::fade`).

## Outlines

`OutlineRenderer` draws screen-space rings around chosen meshes and a fill
over the parts hidden behind other geometry, so enemies behind walls stay
readable. Each `OutlineStyle` sets the visible ring, the occluded ring, the
occluded fill and the width in pixels (up to 64 styles per pass).

## Voxels and meshes on one scene

`quasar_render_voxel::HybridFrame` records a frame's passes in a fixed
order and rejects work added for an earlier stage. The stages are compute,
shadows, opaque, decals, translucent, effects and post.

- In the opaque stage, voxels clear the scene and meshes load it
  (`HybridFrame::mesh_pass_options`).
- Submitting through `HybridCommands` reports every voxel submission to
  its renderer.

`examples/voxel-drift` composes voxels, a mesh drone, scorch decals,
particles, an outline and the post chain this way.
