# Graveyard

A small walkable night scene built to show fog that looks like fog: a
first-person walk through a walled graveyard of headstones, crosses, slabs,
obelisks, dead trees and a mausoleum, lit by a cold moon, four lanterns and a
flashlight.

```sh
cargo run -p graveyard
cargo run -p graveyard -- --screenshot /tmp/graveyard.png
cargo run -p graveyard -- --no-flashlight --screenshot /tmp/graveyard-dark.png
```

| Action | Key |
|---|---|
| Move | `WASD` |
| Look | Mouse |
| Run | `Shift` |
| Flashlight | `F` |
| Fullscreen | `F11` |
| Quit | `Escape` |

The fog is `quasar_render_3d::VolumetricFog`, a ray-marched pass over the
scene depth:

- A dense layer hugs the ground and thins by head height, broken into
  drifting banks by 3D noise, so it has shape rather than fading colours
  evenly with distance.
- Every sample is lit by the scene's lights with a forward-scattering phase:
  the lanterns wear warm halos, the flashlight cuts a beam (with a faint ring
  cookie), and unlit fog stays dark.
- Open sky is fogged too, so the edge of the world dissolves without a seam.

Post-processing adds ACES tonemapping, bloom on the lantern glass, a cold
grade and a vignette. Ground textures are generated with periodic noise from
`quasar_core::noise`.
