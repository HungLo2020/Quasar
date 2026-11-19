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
renderer.set_fog(Some(Fog::distance([0.01, 0.015, 0.03], 12.0, 44.0)))?;
```

Clustering starts automatically at `LightClusters::default().min_lights`
(12) positional lights. `cluster_lights` returns the CPU grid for tools and
tests. Fog uses the same `Fog` type as `quasar-render-voxel`, so meshes fade
into the same colour as distant blocks; it applies to opaque and transparent
surface passes.

## Height fog, light scattering and fog shafts

`Fog` has three parts, all evaluated per fragment in the shared module:

- **Distance fog** from `start` to `end` metres (`Fog::distance`). An
  infinite `end` turns it off (`Fog::height` builds height-only fog).
- **Height fog** (`HeightFog { base, density, falloff }`): exponential haze
  that is densest at `base` and thins by `falloff` per metre above it. It is
  integrated analytically along each view ray, so it shows from a top-down
  camera where every surface is the same distance away: haze pools on the
  floor and thins over rock tops.
- **Scattering** (`with_scattering`): single scattering with an isotropic
  phase function, proportional to the local fog density (1 is physically
  plausible). Point and spot lights glow through the haze, including their
  cones and cookies; directional lights brighten it. `with_shafts(steps)`
  ray-marches the shadow map along each view ray for directional lights, so
  light through cave openings and windows forms volumetric shafts.
  Positional lights are taken from the fragment's light cell.

```rust
renderer.set_fog(Some(
    Fog::distance(night, 12.0, 44.0)
        .with_height(HeightFog { base: 0.0, density: 0.06, falloff: 1.4 })
        .with_scattering(1.0),
))?;
```

Voxels share the fog: `VoxelShading::fog` takes the same value, and the
voxel renderer rebases the height for its camera-relative positions.
Nightwatch's ground mist uses all three parts.

## Volumetric fog

The analytic `Fog` is cheap but uniform: it can only fade colours with
distance. `VolumetricFog` ray-marches every pixel through a fog volume
instead (`VolumetricFogSettings`): a layer densest at `base` that thins with
height, carved by drifting 3D noise (`noise_frequency`, `noise_strength`,
`wind`) into banks and gaps, plus optional thin `haze` everywhere. Each
sample is lit by the pass's lights (falloff and cookies included) with a
Henyey-Greenstein phase (`anisotropy`), so lamps you look towards wear
halos and spot lights draw beams, plus `ambient` light; unlit fog stays
dark. It composites `scene * transmittance + in-scattered light` into the
`SceneTarget` in one blended draw after the opaque passes, and fogs open sky
too, so there is no horizon seam. Cost is `steps x lights` per pixel. The
Graveyard example is built around it.

## Light falloff and cookies

`Attenuation::Curve { exponent }` gives a `(1 - d/range)^exponent` falloff
next to `Smooth` and `InverseSquare`: high exponents hug the light (a
flare), low ones spread an even pool (a lantern). Spot lights take a
`LightCookie`: a procedural pattern (`Rings`, `Bars`, `Spokes` or `Dapple`)
projected across the cone, with a count, contrast and rotation. Animate it
by changing `rotation` or the light's intensity per frame.
`LightCookie::sample` matches the shader for tools and tests. Nightwatch's
flashlight uses a faint reflector-ring cookie.

## Toon shading, ink edges and god rays

`ShadingModel::Toon` quantises the light reaching a surface (diffuse,
falloff and shadow together) into `ToonSettings::bands` with soft or hard
edges, a hard specular highlight and an optional rim light. Set the bands
per pass with `set_toon` (meshes) or `VoxelShading::toon` (voxels).

`ScreenEffects` adds two depth-driven passes that blend straight into a
`SceneTarget` with no colour copy:

- `prepare_edges(EdgeSettings)` inks silhouettes, depth steps and creases.
  Creases come from the surface reconstructed from depth (its direction
  either side of each pixel), so they follow real folds rather than depth
  slope. `crease_threshold` is the cosine of the bend that counts.
- `prepare_god_rays(GodRays)` blurs the open sky (pixels with no geometry)
  radially from a light's screen position and adds it, for sun shafts past
  silhouettes. It adds nothing when the light is behind the camera. For
  light through openings inside geometry, use fog shafts instead.

Voxel Drift's `--toon` style combines toon voxels and meshes with ink edges;
its sun streams god rays past the asteroid.

## Emissive maps

`SurfaceMaterial::emissive_map` (`EmissiveMap`) scales the emissive colour
per texel using the base texture's UVs and sampler, so veins, windows and
eyes glow on their own. Model upload accepts glTF `emissiveTexture` maps
(they must share the base texture's UV transform). Voxel materials have an
emissive mask layer instead: `MaterialDef::with_emissive_mask(colour,
layer)`.

## Scene looks

`SceneLook` gathers a place's sun, ambient light, fog, toon bands, colour
grade, bloom, vignette, god rays and ink edges in one value. `lerp` blends
two looks (colours, intensities and distances blend; discrete choices switch
half way, and the endpoints are exact), `lights_with_sun` and `lighting`
build the pass lighting, `post_settings` the post chain, `sun_rays` the god
rays and `apply` sets fog and toon bands on a surface renderer.

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
texture's alpha. `set_soft_distance(metres)` turns on soft particles: each
quad fades out where it nears the geometry behind it (read from the
target's depth), so dust and mist sit in the air without hard intersection
lines. `clear()` removes every live and queued particle at the next
prepare, for scene changes and loads.

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
