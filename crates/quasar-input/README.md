# quasar-input

Portable keyboard/mouse state and named actions, with no windowing, graphics,
file I/O, or native device library dependencies. Games own action names and
configuration versions; input can come from a window adapter, another host,
or scripted code.

```rust
use quasar_input::{Binding, Bindings, InputState, Key};

let mut bindings = Bindings::default();
bindings.bind("move", Binding::axis(Key::KeyW, Key::KeyS))
    .bind("serve", Binding::key(Key::Space))
    .bind("fullscreen", Binding::chord([Key::AltLeft, Key::Enter]));
let mut input = InputState::default();
input.set_bindings(bindings).unwrap();
input.key(Key::KeyW, true);
assert_eq!(input.action("move").value, -1.0);
input.consume_edges();
```

`Key` identifies physical locations rather than characters in a particular
layout. Text input is separate. `Button` supports physical keys and mouse
buttons. Buttons contribute +1; an opposing-button axis contributes -1/0/+1;
a chord requires every button to be held. Alternative bindings for an action
are summed and clamped to [-1,1]. Empty lists intentionally disable actions.
Mouse motion and line/pixel wheel deltas remain separately available; they
are not currently bindable axes. Controller/analog backends are future work.

Actions are evaluated on each actual button change. `pressed`/`released` track
transitions into/out of a nonzero value and accumulate until `consume_edges`.
A complete press/release between ticks therefore survives. Auto-repeat and an
additional alternate binding do not repeat an already-active action's press.

Consume once after a simulation tick, retaining held values for catch-up ticks.
Immediate commands can use `take_pressed(name)` to consume only their own edge.
Call `release_all` on focus loss: held devices clear, releases remain observable,
and pending presses/motion/text are cancelled. Replacing bindings initializes
values from currently held devices without inventing new action presses;
invalid replacements leave the previous configuration intact.

`Bindings` serializes through serde and validates empty action names, empty or
duplicate chords, and identical opposing buttons. Unknown physical key names
are rejected on deserialization. Games can perform further validation of their
own action names. Different actions may intentionally share a button.

`quasar-window::input` adapts winit events into this crate. It can be used with
`GameWindow` or called directly by a custom host. The core does not depend on
that adapter. See [Pong's controls](../../examples/pong/src/controls.rs) for
configuration loading and named gameplay/window commands.

External applications can consume this crate using a pinned Git dependency
(or a registry version after publication), without a sibling Quasar checkout:

```toml
quasar-input = { git = "https://github.com/HungLo2020/Quasar", rev = "<commit-containing-this-crate>" }
```

Cargo fetches the repository into its managed cache. Paths in this repository's
workspace manifests are internal workspace wiring, not a requirement imposed
on external applications. Replace the revision placeholder with a commit containing this crate.
