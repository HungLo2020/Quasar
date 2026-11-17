# Voxel Drift

A Space Engineers-style scene on Quasar's voxel stack. It has a smooth,
diggable asteroid and a ship of oriented, multi-cell and sloped blocks on a
dynamic 2.5 m grid. The ship splits into separate drifting bodies when it is
cut apart. Everything is original and procedural, and edits stay in memory.

```sh
cargo run --release -p voxel-drift
cargo run --release -p voxel-drift -- --headless --autoplay --ticks 600 --screenshot /tmp/drift.png
```

## Controls

| Input | Action |
|---|---|
| `W/A/S/D`, mouse | Fly, look |
| `Space` / `Left Ctrl` | Up / down |
| `Left Shift` | Boost |
| Left mouse | Use the tool: drill or fill the asteroid, or place a block on a ship |
| Right mouse | Remove the outlined ship block |
| `R` | Turn the next block (four headings around the mounted face) |
| `1`-`8`, wheel | Select a tool: drill, filler, armor, dark armor, slope, window, thruster, light |
| `F11` / `Escape` | Fullscreen / quit |

## What it uses

- **Asteroid:** a `Grid<Cube32>` with an `i8` density channel and a paletted
  material channel (rock, ore, ice) from a deterministic noise `VoxelSource`.
  `LodSelector` picks 32³-cell nodes over three levels around the camera.
  Coarse nodes sample with `sample_lod`. `SurfaceNetsExtractor` meshes nodes
  on a job pool, with `SmoothOptions::seam_faces` adding skirts where levels
  meet. The drill and filler are spherical `Brush` edits that remesh only the
  touched nodes. Full-resolution trimesh colliders come from
  `VoxelColliders::rebuild_smooth`.
- **Ships:** `Grid<Cube16>`s with a `BlockCatalog` of armor, a slope (custom
  wedge model whose side faces cull only against other slopes), a window
  (translucent), a two-cell thruster and a light. Blocks place against the
  targeted face with one of 24 `Orientation`s, and mount points decide what
  holds together. Each ship is a Rapier dynamic body built from merged-box
  colliders (`VoxelColliders::dynamic_body` and `rebuild_cubic`). Removing a
  block runs `split_check`. Every piece but the largest moves to a new grid
  (`Volume::extract_cells`) and a new body with the parent's velocity plus a
  push away from the cut.
- **Rendering:** one `VoxelRenderer` draws the asteroid nodes (scaled
  placements per level) and every ship grid. Ship transforms are rebased to
  the camera in `f64` each frame. Smooth materials are triplanar and block
  faces use the texture array.

`--headless --autoplay` flies in, drills and fills the asteroid, builds on the
ship and then cuts its neck so the halves drift apart. It prints counts, the
LOD node mix and a state hash, which is the same for any thread count.

## Composition and effects

The renderer draws everything into one linear HDR `SceneTarget` through
`quasar_render_voxel::HybridFrame`, in this order:

1. Opaque voxels, then a lit mesh drone sharing their depth.
2. Scorch decals projected onto freshly dug rock.
3. Mining dust and cutting sparks as GPU particles.
4. The drone's outline, with a see-through fill where the asteroid hides
   it.
5. Bloom and tonemapping in the post chain, then the 2D HUD.
