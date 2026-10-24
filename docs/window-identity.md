# Desktop window identity and compositor compatibility

`quasar-window` owns native window identity and compositor presentation hints so
games no longer need private Xlib/XCB code. The crate still exposes an ordinary
winit event loop, remains independent of VulkanicGAL and every rendering crate,
and accepts caller-owned graphics initialization.

## Reference audit

The audit was read-only. Only Quasar was changed.

| Project | Confirmed native behavior | Source of truth |
|---|---|---|
| MissileCommand | Xlib/XCB window id, `WM_CLASS` `missile-command`/`MissileCommand`, `_NET_WM_PID`, `_NET_WM_WINDOW_TYPE_NORMAL`, `_NET_WM_WINDOW_OPACITY = 0xFFFFFFFF`, and `_NET_WM_OPAQUE_REGION` updated from physical drawable size | `src/desktop.rs`, `DevUtils/window_smoke.py` |
| BlackOps | Xlib class hint `blackops-native`/`BlackOpsNative`, PID, full opacity and opaque region at creation, re-applied when the size changes | `src/window.rs`, `src/main.rs`, `analysis/window-*.json` |
| Jackal | Historical verified window used `jackal-native`/`JackalNative` and `opacity 4294967295` | `analysis/verification.txt`; current SDL path sets `SDL_APP_NAME=Jackal` in `src/desktop/window.rs` |
| MicroMages | SDL X11 identity hint `MicroMagesRust` and `SDL_APP_NAME=Micro Mages` | `src/window.rs`, `scripts/verify_desktop.py` |
| MattMC | GLFW/LWJGL owns the window; uses `GLFW_NO_API` for the Vulkan path and no custom X11 properties were found | `src/main/java/net/blaze3d/platform/Window.java` |
| Quasar-Snake | Uses `WindowConfig.application_id = "quasar-snake"` and has no native identity code | `src/desktop.rs`, `Cargo.toml` |

Confirmed requirements:

- Metadata must exist before a slow Vulkan presenter makes the first frame.
- The opaque region is physical pixels and must follow every resize.
- X11 properties have exact types/formats; Xlib's native `unsigned long` and
  XCB's `u32` both represent format-32 CARDINAL/ATOM values.
- A normal top-level window type and a process id are expected by the reference
  smoke tests.
- Opaque windows must explicitly request full opacity so a compositor rule for a
  differently identified window cannot inherit accidental transparency.
- Wayland has no equivalent X11 properties; identity is the application id.
- Windows/macOS must not acquire an X11 dependency.

MissileCommand's two startup fixes were reproduced as requirements, not copied:
metadata is attached immediately after native creation (`9028ec4`), and a native
resize that occurs while Vulkan initializes is reconciled later (`9ea3a7b`).

Not confirmed:

- No evidence identifies a specific KWin rule as the sole cause of every
  historical transparency bug. The safe requirement is an explicit class/id and
  explicit full opacity, which Quasar now provides.
- The current Jackal SDL window does not contain its own `WM_CLASS` code; its
  verified class values are treated as a compatibility requirement.

## Public API

`WindowConfig` is unchanged, so existing callers keep compiling. Its
`application_id` remains the Wayland app id, the X11 fallback name and the Win32
class name.

```rust
use quasar_window::{DesktopConfig, GameWindow, WindowConfig, X11Identity};

let config = WindowConfig {
    title: "Missile Command".into(),
    application_id: "missile-command".into(),
    ..WindowConfig::default()
};

// Default policy: opaque, full-window opaque region, identity from application_id.
let window = GameWindow::create(event_loop, &config)?;

// Reference-game policy: distinct X11 instance/class names.
let desktop = DesktopConfig {
    x11: Some(X11Identity::with_names("missile-command", "MissileCommand")),
    ..DesktopConfig::for_application("missile-command")
};
let window = GameWindow::create_with_desktop(event_loop, &config, &desktop)?;
```

Types:

- `DesktopConfig` selects `WindowTransparency` (`Opaque` or `Transparent`) and
  `OpaqueRegionPolicy` (`FullWindow` or `Disabled`). A transparent window cannot
  request a full opaque region.
- `X11Identity` carries optional instance/class names plus `set_pid` and
  `normal_window_type`. Unset names fall back to `application_id`.
- `desktop_capabilities()` reports which properties are compiled for the current
  target. X11 capabilities are false on Windows/macOS and in
  `--no-default-features` Linux builds. Wayland app-id and Win32 class support are
  reported separately.
- `DesktopError` covers validation and property-write failures.
- `GameWindow::sync_desktop()` re-applies metadata from the current physical
  size. `GameWindow::process()` does this automatically; `desktop_error()` exposes
  an asynchronous write failure without terminating the event loop.

The X11 property backend is enabled by the default `x11` feature. Disabling it
removes `x11rb` but keeps winit identity attributes and Wayland/Windows support:

```sh
cargo check -p quasar-window --no-default-features
```

## Lifecycle and timing

1. `create_with_desktop` validates identity strings and policy.
2. Winit creates the window with the backend-appropriate identity attribute.
   X11 and Wayland share winit's name slot, so Quasar inspects the active display
   backend and sets only the matching name; it never overwrites `WM_CLASS` with
   the Wayland application id, or vice versa.
3. Immediately after native creation, Quasar writes PID, full opacity (or removes
   it for transparent windows) and clears a stale opaque region when needed.
4. The initial opaque region uses `Window::inner_size()` — physical pixels.
5. `create_with_desktop` returns; the caller can now initialize VulkanicGAL or
   any other graphics API. The presenter therefore cannot delay identity or
   opacity metadata.
6. Every `process()` call reconciles the cached physical size. A resize that
   occurred during graphics initialization is picked up by the next event or by
   an explicit `sync_desktop()`.
7. `ScaleFactorChanged`, `Resized`, fullscreen transitions and restore all pass
   through the same reconciliation. Zero-sized/minimized surfaces keep the last
   valid rectangle; the restore resize replaces it.
8. Suspension/recreation creates a new `GameWindow` and repeats steps 2–4.
9. Repeated identical sizes do not rewrite `_NET_WM_OPAQUE_REGION`.

## Property contract (Linux X11)

| Property | Type / format | Value |
|---|---|---|
| `WM_CLASS` | `STRING` / 8 | `"instance\0Class\0"` set by winit at creation |
| `_NET_WM_PID` | `CARDINAL` / 32 | current process id |
| `_NET_WM_WINDOW_TYPE` | `ATOM` / 32 | `_NET_WM_WINDOW_TYPE_NORMAL` when enabled |
| `_NET_WM_WINDOW_OPACITY` | `CARDINAL` / 32 | `0xFFFFFFFF` for opaque; absent for transparent |
| `_NET_WM_OPAQUE_REGION` | `CARDINAL` / 32 | `[0, 0, physical_width, physical_height]`; absent for transparent |

Only the game window is modified. No root-window or compositor-global settings
are read or written.

## Platform behavior

| Capability | Linux X11 | Linux Wayland | Windows | macOS |
|---|---|---|---|---|
| Application identity | `WM_CLASS` via winit | `xdg_toplevel.app_id` via winit | window class via winit | bundle identity; no window property |
| Process id | `_NET_WM_PID` | not applicable | process-owned window | process-owned window |
| Normal window type | `_NET_WM_WINDOW_TYPE_NORMAL` | compositor default toplevel | ordinary window | ordinary window |
| Explicit full opacity | `_NET_WM_WINDOW_OPACITY` | not applicable | native window is opaque | native window is opaque |
| Opaque region | `_NET_WM_OPAQUE_REGION` | not applicable | not applicable | not applicable |
| Transparency | alpha visual via winit | alpha surface via winit | `with_transparent` via winit | `with_transparent` via winit |

Unsupported optional properties are not fabricated. Use
`desktop_capabilities()` if a game needs to report the difference.

## Migration

### MissileCommand

Delete the private `Desktop`/`X11` structs, the `x11rb` and
`raw-window-handle` imports, the manual `WM_CLASS`/PID/type/opacity writes and the
resize hook. Remove those two optional dependencies and their feature entries
from `Cargo.toml` once no other module uses them. Keep the `Desktop` struct only
if it holds unrelated state.

```rust
let desktop = DesktopConfig {
    x11: Some(X11Identity::with_names("missile-command", "MissileCommand")),
    ..DesktopConfig::for_application("missile-command")
};
let window = GameWindow::create_with_desktop(event_loop, &config, &desktop)?;
// Create the presenter after this line; no metadata call is needed.
```

Existing `window.process(&event).resized` handling and rendering stay unchanged.
The game's private `DevUtils/window_smoke.py` can eventually point at
`tools/x11_window_smoke.py` behavior.

### Jackal

Replace the SDL hint expectation with a Quasar window when Jackal adopts
Quasar (it currently uses SDL directly):

```rust
let desktop = DesktopConfig {
    x11: Some(X11Identity::with_names("jackal-native", "JackalNative")),
    ..DesktopConfig::for_application("jackal-native")
};
```

The full-opacity and normal-window defaults match the historical verified
values. Jackal's current `SDL_APP_NAME=Jackal` behavior is equivalent to the
Wayland/application-id half of the API.

### BlackOps

Replace `blackops_native::window::configure(&window)` and its resize-time call:

```rust
let desktop = DesktopConfig {
    x11: Some(X11Identity::with_names("blackops-native", "BlackOpsNative")),
    ..DesktopConfig::for_application("blackops-native")
};
let window = GameWindow::create_with_desktop(event_loop, &config, &desktop)?;
```

`window.process()` now performs the resize-aware opaque-region update, so the
manual `if size != window_size { configure(...) }` branch can disappear. The
`transparency: false` minifb option maps to the default `Opaque` policy. After
deleting `src/window.rs`, remove `x11-dl` from `Cargo.toml`; keep
`raw-window-handle` only if the GPU surface code still needs it.

### MicroMages

Replace `SDL_VIDEO_X11_WMCLASS=MicroMagesRust` and `SDL_APP_NAME`:

```rust
let desktop = DesktopConfig {
    x11: Some(X11Identity::with_names("micromages", "MicroMagesRust")),
    ..DesktopConfig::for_application("micromages")
};
```

The class keeps MicroMages' verified value. The application id is the Wayland
identity. No X11 properties are attempted when winit selects Wayland.

## Validation

```sh
cargo fmt --all -- --check
cargo clippy -p quasar-window --all-targets --locked -- -D warnings
cargo clippy -p quasar-window --all-targets --no-default-features --locked -- -D warnings
cargo test -p quasar-window --locked
cargo check -p quasar-window --no-default-features --locked
RUSTDOCFLAGS='-D warnings' cargo doc -p quasar-window --no-deps --locked
```

Native X11 tests create a real window, read the properties back with x11rb,
resize it externally, and verify the opaque region follows physical size:

```sh
xvfb-run -a sh -c 'QUASAR_X11_TESTS=1 cargo test -p quasar-window --test x11_native -- --test-threads=1'
cargo build -p quasar-window --example desktop_identity
xvfb-run -a python3 tools/x11_window_smoke.py
xvfb-run -a python3 tools/x11_window_smoke.py --transparent
WINIT_X11_SCALE_FACTOR=2 xvfb-run -a python3 tools/x11_window_smoke.py
```

The smoke script polls `xprop`/`xwininfo` with deadlines (no fixed sleeps), uses
`xdotool` when present, and falls back to an Xlib resize for minimal images. It
checks 640×480 plus 800×600, 1100×650 and 440×700, matching MissileCommand's
smoke sequence.

Wayland identity is exercised with a real mapped surface. `DISPLAY` is removed
from the child environment so winit must use Wayland, and KWin must report
`resourceClass` equal to `WindowConfig.application_id`:

```sh
cargo build -p framebuffer
python3 tools/wayland_window_smoke.py --timeout 20
```

### KDE/KWin

Xvfb proves property correctness but not compositor behavior. On a KWin session
(either native X11 or XWayland), run:

```sh
python3 tools/x11_window_smoke.py --kwin --timeout 15
```

The script loads a short KWin scripting module through D-Bus, finds the window by
`resourceClass`, and reads KWin's own `resourceName`, `normalWindow` and
`opacity` (expected `1` for opaque windows). It polls the user journal instead of
relying on the active-window tool, which can report another application. This
verifies the exact condition behind the historical KWin inheritance problem:
KWin sees the intended class and applies full opacity. A visual check can still
capture the active window:

```sh
spectacle -b -n -a -o /tmp/quasar-window.png
```

The opaque window must contain no desktop bleed-through. Transparent mode should
show the desktop and must not expose `_NET_WM_WINDOW_OPACITY` or
`_NET_WM_OPAQUE_REGION`. This repository's automated checks verify the identity
and property side; the screenshot step remains manual because it depends on the
running desktop theme and capture permissions.

## Compatibility matrix

| Requirement | Status |
|---|---|
| MissileCommand `WM_CLASS`, PID, normal type, full opacity | Implemented; native X11 and KWin class/instance/opacity verified |
| MissileCommand resize-aware opaque region | Implemented; 800×600, 1100×650, 440×700 verified |
| Jackal / BlackOps distinct instance/class | Implemented; verified with equivalent values |
| BlackOps PID/opacity/opaque region and resize updates | Implemented; covered by the same native tests |
| MicroMages SDL class identity | Representable via `X11Identity::with_names`; Wayland app-id delivery verified end-to-end |
| Existing Pong/Snake/Tilequest/Framebuffer callers | `WindowConfig` unchanged; workspace tests pass |
| Metadata before graphics initialization | Properties applied inside `create_with_desktop` before return |
| DPI, fullscreen, minimize/restore, recreation | Reconciliation on resize/scale/fullscreen; scale-2 physical size verified; zero-size keeps last valid region; recreation repeats creation |
| No X11 dependency on Windows/macOS | Target-specific `x11rb`/`raw-window-handle`; compile checks configured in the matrix |
| Independently usable window crate | No renderer/ECS/runner dependency; ordinary winit loop |

Per-game view:

| Game | Required behavior | Quasar representation | Verified |
|---|---|---|---|
| MissileCommand | `missile-command`/`MissileCommand`, PID, normal type, full opacity, resize region | `X11Identity::with_names`; default opaque policy; `process` | Native X11 test, Xvfb/Xephyr smoke, KWin class/opacity |
| Jackal | `jackal-native`/`JackalNative`, full opacity | `X11Identity::with_names`; default opaque policy | Equivalent native identity/opacity test |
| BlackOps | `blackops-native`/`BlackOpsNative`, PID, opacity, opaque region, resize | `X11Identity::with_names`; `process` replaces resize hook | Native identity/property/resize test |
| MicroMages | `MicroMagesRust` X11 class, application name | `X11Identity::with_names`; `application_id` for Wayland | X11 class path plus mapped Wayland app-id test |
| MattMC | GLFW-owned identity; no custom X11 properties | No change required; not a Quasar consumer | Read-only audit |
| Pong/Snake/Tilequest/Framebuffer | Existing `WindowConfig.application_id` | `GameWindow::create` default policy | Workspace tests |

### Results in this environment

- Isolated nested X server (`Xephyr`, 24-bit, no window manager): native
  integration test passed, including 640×480 initial geometry, three resize
  sizes, transparent-window property removal, and fullscreen reconciliation.
- `tools/x11_window_smoke.py` passed for both opaque and transparent windows
  using the libX11 resize fallback (`xdotool` is not installed here).
- With `WINIT_X11_SCALE_FACTOR=2`, the requested 640×480 logical window became
  1280×960 physical and the opaque region followed that physical size, including
  all resize sizes.
- KDE Plasma/KWin 6.6.6 Wayland with XWayland: the X11 smoke reported KWin
  `class=QuasarSmoke`, `name=quasar-smoke`, `normal=true`, `opacity=1`; the
  transparent run kept normal-window identity and removed the X11 opacity/opaque
  properties.
- `tools/wayland_window_smoke.py` passed with a mapped `framebuffer` surface and
  no `DISPLAY`, proving KWin received `quasar-framebuffer` as the Wayland
  `resourceClass` with `normal=true`, `opacity=1`.
- CI is configured to compile-check Windows and macOS in the headless matrix;
  neither platform was available for compilation or runtime testing here.

## Known limitations

- Wayland app-id delivery was verified on KWin with a mapped framebuffer
  surface; other compositors (Mutter, Sway, Weston) were not available here.
- Windows and macOS were neither compiled nor run locally; their compile checks
  run in CI.
- The KWin screenshot check is manual; `--kwin` automates identity, normal-window
  and compositor opacity verification.
- The API exposes full-window opaque regions only; partial-region transparency
  is future work.
- Minimize/restore is covered by zero-size reconciliation rules and native
  fullscreen tests, not by a compositor-specific minimize test.
