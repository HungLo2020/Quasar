# Falling Sand

Interactive cellular sand with Rapier 2D obstacles, one dynamic ball and
`quasar-spatial` queries. Rendering uses `quasar-render-2d` and VulkanicGAL.

```sh
cargo run -p falling-sand
cargo run -p falling-sand -- --headless --ticks 400 --autoplay --screenshot /tmp/sand.png
cargo run -p falling-sand -- --benchmark --benchmark-frames 120 --benchmark-file /tmp/sand.json
```

## Controls

| Input | Action |
|---|---|
| Left mouse | Paint with the selected tool |
| Right mouse | Erase sand/obstacles |
| `1` | Sand brush |
| `2` | Rock brush |
| `3` | Erase tool |
| `R` | Reset |
| `F11` | Fullscreen |
| `Escape` | Pause menu |

## Menus and settings

The game opens on a retro main menu (`Start`, `Options`, `Quit`). `Escape`
during play opens the pause menu (`Resume`, `Options`, `Quit to menu`,
`Restart`). Quit and restart require a modal confirmation. The options screen
covers fullscreen, brush radius and UI scale; **Apply** commits and persists
them to a game-owned TOML file (`$XDG_CONFIG_HOME/quasar/falling-sand.toml`),
while **Cancel** restores the last applied values. Menus are built with
`quasar-ui` and drawn onto the shared `quasar-render-2d` canvas.

## What it demonstrates

- A deterministic 240×135 cellular sand simulation with gravity and diagonal
  flow.
- Static Rapier colliders for ground, walls and user-placed rocks; one dynamic
  ball with restitution and friction.
- `quasar-spatial`: obstacle AABBs and the moving ball are indexed; brush edits,
  overlap rejection and the HUD's nearby-object count use grid queries.
- `quasar-app` fixed-step lifecycle, `quasar-input` mouse/keyboard, diagnostics,
  `quasar-profile` frame measurements and `quasar-frame` presentation.
- `quasar-ui` retro main/pause/options/confirmation menus drawn onto the same
  2D canvas with the bitmap 5×7 font, game-owned TOML persistence and
  apply/cancel semantics. The demo intentionally enables no `quasar-text`
  feature, showing that font dependencies remain optional.
- Headless deterministic ticks with `--headless --autoplay`, screenshots through
  `quasar-frame::OffscreenTarget`, and an optional benchmark JSON artifact.

## Measured on the development host

Release, Xephyr at 1280x720, 120 benchmark frames. The X server was
presentation-bound at roughly 40–45 Hz:

| Metric | p50 | p99 |
|---|---:|---:|
| CPU frame | 23.1 ms | 28.8 ms |
| GPU frame | 41.4 µs | 63.5 µs |
| Submit | 361 µs | 570 µs |
| Draws | 348 | 721 |
| Host writes | 28 KB | 58 KB |

Headless simulation was ~0.8–1.0 ms per tick with autoplay. These are
single-host observations; the CPU frame reflects presentation cadence, not GPU
cost.

## Design/limits

Sand cells are not individual Rapier bodies; this keeps thousands of grains
cheap. Sand does not physically push the dynamic ball, and the ball does not
displace sand. Obstacles are axis-aligned rectangles. The world is a bounded
grid; sand cannot leave it.

## Verification

```sh
cargo test -p falling-sand
cargo test -p falling-sand --test menu
cargo test -p falling-sand --test sand -- --ignored
cargo clippy -p falling-sand --all-targets -- -D warnings
```

Menu tests cover Enter-to-start, Escape pause, options apply/cancel and settings
persistence without touching game save schemas.
