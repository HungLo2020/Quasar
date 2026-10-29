# Nightwatch model assets

These are selected, unmodified low-poly model/image files from Kenney. They are
bundled so running Nightwatch requires no network access or asset-generation
step. The example applies material tints for its industrial palette; original
geometry, textures, names and files are retained.

| Files | Source | License |
|---|---|---|
| `blaster-kit/blaster-d.glb`, `blaster-f.glb` | [Blaster Kit 2.1](https://kenney.nl/assets/blaster-kit) | CC0 1.0 |
| `blaster-kit/crate-medium.glb`, `crate-wide.glb`, `Textures/colormap.png` | [Blaster Kit 2.1](https://kenney.nl/assets/blaster-kit) | CC0 1.0 |
| `survival-kit/barrel.glb`, `workbench.glb`, `metal-panel.glb`, `Textures/colormap.png` | [Survival Kit 2.0](https://kenney.nl/assets/survival-kit) | CC0 1.0 |

Each pack directory includes the author's original `License.txt`.
[CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/) permits copying,
modification and redistribution, including commercial use. Credit is optional;
we retain it here for provenance. No other pack files or third-party assets are
included. `manifest.json` records the original archive SHA-256 and each selected
model/image SHA-256. Downloads came from the official pack pages on 2026-10-28.

The guns are stylized blasters used as the sample's pistol and rifle. Authored
node names expose their detachable magazines. Crates include opening clips;
Nightwatch explicitly imports their default pose and leaves those clips
unevaluated. Gun recoil/reload posing is ordinary game code. This is not a
skeletal animation import or skinning implementation.

Run with `--asset-root PATH` to use an editable copy with this same layout, or
set `assets.root` in TOML/JSON / `NIGHTWATCH_ASSET_ROOT`. F5 reloads the selected
models and their images; failed imports leave the old GPU generation usable.
The generic asset APIs accept any mounted paths and impose none of these
Nightwatch filenames.
