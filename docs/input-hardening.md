# Input edge hardening

Audit scope: `quasar-input`, `quasar-window`, the gamepad layer and the winit
adapter. Existing public APIs and binding semantics remain unchanged.

## Findings and fixes

| Finding | Layer | Resolution |
|---|---|---|
| Keyboard auto-repeat can produce duplicate gameplay press edges | `quasar-window::input` | `process_key_event` ignores `KeyEvent::repeat` for physical/action state. A held action remains held without a new press edge; `text` is handled separately below. |
| Text-entry auto-repeat is lost if all repeats are discarded | `quasar-window::input` | Repeated printable `text` is still appended to `InputState::text`, preserving normal chat/console/UI key repeat without creating a physical or action edge. |
| IME/chat text must not be tied to a physical key release or repeat | `quasar-window::input` | `WindowEvent::Ime(Ime::Commit)` appends committed text; preedit is ignored until commit and no key edge is synthesized. |
| Duplicate down/up events can create duplicate action edges | `quasar-input` | Existing held/pressed/released sets already deduplicate; regression tests now cover duplicate keyboard and gamepad events explicitly. |
| A second input source can accidentally repeat an already active action press | `quasar-input` | Existing transition logic is retained: press occurs only on `0 -> nonzero`, release only on `nonzero -> 0`; tests cover keyboard plus gamepad coexistence and release only at zero. |
| Fast tap between simulation ticks can lose an edge | `quasar-input` | Existing accumulated edges are retained through `consume_edges`; regression tests exercise tap/consume ordering. |
| Focus loss leaves stale held keyboard/gamepad state | adapter + gamepad layer | Window focus loss already calls `InputState::release_all`; new `PlayerInput::release_all` and `GamepadHub::release_all` clear a player’s keyboard/mouse/gamepad state without losing assignments. |
| Device disconnect leaves a gamepad action held | `quasar-input` | `GamepadHub::disconnect` already releases the assigned player’s controller-derived input; tests cover disconnect and reconnect. |
| Keyboard and gamepad simultaneously drive the same action | `quasar-input` | Summing/clamping and edge semantics are tested; releasing one source keeps the action held while the other remains active. |
| Repeat/release ordering after a real press | adapter/input | `process_key_event` ignores repeats but accepts the real release; tests assert no duplicate pressed/released edges. |
| Stale raw edges survive a rebind | `quasar-input` | `set_bindings` now discards pending raw press/release sets while initializing held action values, matching its documented configuration boundary. |

The winit adapter deliberately does not turn `DeviceEvent::Removed` into a
global keyboard release: winit does not provide enough device-to-button state to
release only that device. Applications using gamepads should route removal
through `GamepadHub`, which owns device identity; controller-hotplug tests cover
this path.

## Behavior contract

- A real key press is a new press only if the key was not already held.
- Auto-repeat never creates a physical or action press edge.
- Repeated printable text is appended for chat, console and UI fields; text is
  separate from bindings and is cleared by `consume_edges`. Holding a gameplay
  key does not re-fire an action even while its text repeats.
- IME `Commit` text is appended once for chat/console/UI editing; `Preedit` is
  not committed, and IME text does not synthesize a physical key edge.
- A release is observable once; duplicate releases are ignored.
- Press/release edges accumulate until `consume_edges` or an action-specific
  `take_pressed`.
- Focus loss releases held devices and clears pending presses/motion/text; action
  releases remain observable.
- Gamepad disconnect releases only that controller’s contribution; keyboard and
  other players remain intact.
- A held action remains active while any compatible source remains nonzero.

## Local overhead measurement

`cargo run --release -p quasar-window --example input_overhead` processes
2,000,000 adapter events on the same Linux x86_64 development host used for the
timing probe. One local run reported:

| Path | Time | Notes |
|---|---:|---|
| Press/release + action update + tick consume | 199 ns/event | includes two hash-set transitions and action evaluation |
| Auto-repeat text (no edge) | 1.04 ns/event | one small text append |

The press/release number is the cost of a complete edge cycle (two events,
roughly 398 ns), not a per-frame cost; a 60 Hz frame processes only the events
the OS actually delivers. These are probe observations, not portable guarantees.

## Acceptance criteria

- A real press creates exactly one press edge; repeat and duplicate presses do
  not create additional edges.
- Auto-repeat never changes `held`, `pressed` or action press state.
- Repeated printable text remains available for text fields without affecting
  gameplay bindings.
- A fast press/release between ticks remains observable as both edges.
- Focus loss clears held/pending state and emits observable releases.
- Rebinding clears stale raw edges and does not synthesize a press.
- Controller disconnect releases only the affected player's controller input;
  reconnect restores preferences.
- Simultaneous keyboard/gamepad sources hold the action until every source is
  released.
- Text and IME commits remain separate from physical bindings.

## Tests

```sh
cargo test -p quasar-input
cargo test -p quasar-input --features gilrs,diagnostics
cargo test -p quasar-window --lib
cargo test -p quasar-window --features diagnostics --lib
cargo run --release -p quasar-window --example input_overhead
cargo clippy -p quasar-input --all-targets --features gilrs,diagnostics -- -D warnings
cargo clippy -p quasar-window --all-targets --features diagnostics -- -D warnings
```

Deterministic tests cover auto-repeat press suppression and text repeat, IME
commit text, duplicate keyboard/gamepad events, keyboard/gamepad overlap,
release-only-at-zero, fast taps, focus release (including text clearing),
rebinding edge clearing, gamepad release on disconnect/reconnect, other-player
isolation and hub-wide focus release. Native X11 window tests remain separate and
are run under Xvfb/Xephyr.

## Limitations

- No per-device keyboard state is modeled; winit's raw device events do not
  identify the logical key mapping for games. `DeviceEvent::Removed` therefore
  cannot selectively release a keyboard binding.
- Gamepad hotplug remains the responsibility of the optional `gilrs` layer or a
  caller-provided `GamepadEvent` source.
- Action edges are intentionally consumed once per simulation/UI boundary; games
  should not consume raw edges per render event.
- Platform-specific focus and auto-repeat behavior was verified through winit’s
  normalized events on Linux; Windows/macOS native behavior is not hardware-tested
  here.
