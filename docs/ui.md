# In-game UI architecture

`quasar-ui` addresses Quasar issue **#43**. It is an optional, independently
consumable retained-UI layer for main menus, pause screens, settings dialogs,
HUD widgets, modal confirmations and overlays. It is in-game UI, not the future
Qt/KDE development editor.

The crate owns UI state and layout. It owns no window, GPU device, swapchain,
ECS or scene graph, and it never submits GPU work itself. A game builds a widget
tree, advances it with an input snapshot, decides what each event means, and
draws the resulting layout with its own renderer or through the optional
`quasar-render-2d` adapter.

## Why a retained tree rather than immediate mode

The audited games repeatedly hand-build menus with page stacks, focus indices,
confirmation timers, rebind capture state and input suppression flags:

- **MicroMages** has custom pixel settings pages, fixed key/gamepad mapping,
  rebind capture, pause/input suppression and TOML controls/display/audio.
- **Jackal** has a 256-sized custom menu canvas, page stack, confirmation timer,
  rebind capture, display apply/keep/revert, focus-loss pause and a pixel HUD.
- **MissileCommand** has a pixel HUD, CLI options, a simple pause/help state and
  mouse-capture edges, but no full settings menu.
- **MattMC** has retained `Screen`/widget trees, grid layout, `EditBox`,
  narration/accessibility metadata, options sub-screens and modal dialogs.
- **BlackOps** was not present locally, so no audit was claimed for it.

A retained tree gives every game the parts that were being rewritten: stable
node identity, explicit focus order, modal scopes, pointer capture, drag state,
rebind isolation and consumed-input reporting. It also keeps layout separate
from drawing so a software renderer (Jackal/MissileCommand style) and a GAL
renderer can consume the same tree.

The alternative — immediate mode — was rejected because these games already
retain page/focus state across frames; rebuilding that state per frame would
move semantics into the engine or force every game to duplicate it. A pure
immediate API would also make modal trapping and rebind capture harder to audit.

## Composition model

```text
Ui
├─ Theme { colors, sizes, padding, gap, borders, transition rate }
├─ nodes: BTreeMap<NodeId, UiNode>
│    UiNode { widget, layout item, layer, visible, enabled, modal, state, scroll }
├─ roots: [NodeId]
├─ focus order / current focus
├─ modal scope (node id)
├─ pointer capture / slider drag / rebind capture
└─ localized strings: BTreeMap<key, value>

update(UiInput, viewport, dpi scale, safe area, measure, dt) -> UiFrame
  ├─ layout: stack | grid | absolute
  ├─ interaction: hover, press, drag, keyboard navigation
  ├─ text editing: cursor, anchor, insert/delete, selection
  └─ events: pressed | changed | text | submitted | focus | rebind | cancelled
```

- `add_root(widget, layout, layer, modal)` creates a layer-scoped root.
- `add_child(parent, widget, layout)` inherits the parent's layer.
- `set_visible`, `set_enabled`, `set_text`, `set_value`, `set_checked`,
  `set_selected` and `set_scroll` mutate nodes without rebuilding the tree.
- `set_modal(Some(id))` traps focus and pointer/keyboard input to that subtree.
- `focus(Some(id))` selects a focus target explicitly.
- `update` is the only clock and the only place that produces events. Layout is
  computed in physical pixels from the viewport rect, DPI scale and safe-area
  insets, so resizing and DPI changes do not rebuild widgets.

Games keep ownership of semantics. The crate emits action ids and values; it
does not know that action 7 means "quit" or that a slider is master volume.
Actions are `ActionId(u64)` supplied by the caller. Values are `Float`, `Bool`,
`UInt` or `Text`.

## Layout

`LayoutItem` supports `Auto`, `Fixed`, `Percent` and `Fill` lengths, margins,
offsets and `Start`/`Center`/`End`/`Stretch` alignment. Panels lay out children
as a vertical/horizontal stack, a fixed-column grid or absolute positioning.

- Panels with `clip` clip descendants to their rect.
- Panels with `scrollable` clamp `scroll` to the content extent and report
  `content_size`; scrolling never grows unbounded memory.
- Layout is deterministic and bounded; invalid or non-finite values are clamped
  or rejected rather than propagated into draw commands.
- `Layer::{Normal, Overlay, Hud}` is preserved on every `LayoutEntry`, so games
  can draw HUD/overlay layers in their own order. A modal root is drawn with the
  modal panel treatment and a focus border.

## Widgets

| Widget | Behaviour | Focusable |
|---|---|---|
| `Panel` | Stack/grid/absolute container; padding, clip, scroll | no |
| `Label` | Literal/localized text; `Body`/`Heading`/`Small`/`Dim` | no |
| `Button` | Press → `Pressed(ActionId)` | yes |
| `Slider` | Bounded/stepped float; drag and arrow keys → `Changed` | yes |
| `Checkbox` | Boolean toggle → `Changed(Bool)` | yes |
| `Select` | One-of-many list → `Changed(UInt)` | yes |
| `Tabs` | One-of-many tab row → `Changed(UInt)` | yes |
| `TextInput` | Committed text, cursor/anchor, max length, password | yes |
| `Progress` | Read-only bounded bar | no |
| `Image` | Texture id, optional source rect, size | no |
| `List` | Selectable rows with scroll index | yes |
| `Binding` | Rebindable action label/value row | yes |
| `Spacer` | Flexible gap | no |

Text is `UiText::literal(...)` or `UiText::keyed(key, fallback)` resolved through
`Ui::set_localized`. Measurement is the `TextMeasure` trait: core provides
`MonospaceMeasure`; feature `render2d` provides `BitmapTextMeasure`, matching the
6×7 bitmap font used by `quasar-render-2d`; feature `text` provides
`ShapedTextMeasure` backed by `quasar-text`, including fallback, bidi and caret
geometry.

## Focus, modality and input isolation

- The first focusable node is focused automatically on the first update unless
  `focus` or `set_modal` already set the target. This fixed the "Enter does
  nothing on first launch" class of bug in the demos.
- `next`/`previous` cycle the focus order; `up`/`down`/`left`/`right` move within
  stacks, grids and sliders; `activate` presses; `cancel` bubbles a `Cancelled`
  event and releases focus/modal state.
- Modal subtrees exclude outside nodes from focus order and interaction. This is
  the confirmation-dialog guarantee (quit/restart prompts cannot be bypassed by
  clicking the menu behind them).
- `UiFrame` reports `consumed`, `consumed_pointer`, `consumed_keyboard` and
  `consumed_text` independently. Games suppress only the gameplay channels the
  UI actually claimed. A text field can consume typing while mouse look keeps
  working when the pointer was not consumed.
- Pointer capture and slider drags survive movement outside the widget bounds.
- Rebinding: `begin_rebind`, `finish_rebind`, `cancel_rebind`. While capturing,
  navigation/gameplay input is isolated and the game decides how the next event
  becomes a binding string. This mirrors MicroMages/Jackal rebind flows without
  putting key encoding in the UI crate.

## Text editing

`TextInput` supports committed text, cursor and anchor selection, insertion,
backspace/delete, home/end and shift-selection. Committed text arrives as a
separate `UiInput::text` string so IME/OS text is never confused with physical
key state. Validation and submit handling stay in the game through
`TextChanged`/`Submitted`. Password fields mask rendering but keep the same
editing model.

## Themes and accessibility primitives

`Theme::retro()` (pixel-art palette, 14 px body, 24 px rows) and
`Theme::modern()` (16 px body, 30 px rows, blue accent) are ready-made palettes.
Every colour, font size, row height, padding, gap, border and focus width is a
public field; games can build a custom theme without forking widgets.

`Theme::contrast_ratio(foreground, background)` implements the WCAG 2.x
relative-luminance formula (alpha ignored) so palettes can be audited. Widgets
track `WidgetState { hovered, pressed, hover_t, focus_t, press_t }`, and focus is
drawn with a distinct border, so keyboard users get visible focus equivalent to
mouse hover. Text is always literal or localized; no widget conveys state by
colour alone. There is no accessibility tree/screen-reader bridge in this
milestone; those primitives are provided rather than a platform integration.

## Rendering integration

The core crate produces `LayoutEntry { id, rect, clip, layer }` plus each node's
widget, resolved text and transition state. Any renderer can consume this list.

Feature `render2d` adds `UiRenderer`, which appends ordered quads to a caller's
`quasar_render_2d::Canvas`:

```rust
let mut painter = UiRenderer::new();
painter.set_texture(image_id, texture_id);
painter.draw(&mut canvas, &ui);
```

The adapter creates no device, swapchain, target or submission. The caller still
owns the canvas, textures, render pass and submission, so UI is composited in the
same GAL frame as 2D/3D/custom passes in whatever order the game chooses. This
was verified in Falling Sand and Starfall, where the menu is drawn onto the
shared 2D canvas between the game pass and presentation.

## Feature matrix

| Feature | Default | Adds |
|---|---|---|
| *(none)* | yes | Core tree, layout, focus, events, themes |
| `input` | no | `UiBindings`, `from_input_state` |
| `render2d` | no | `UiRenderer`, `BitmapTextMeasure` |
| `config` | no | `config_to_value`, `value_to_config` |
| `serde` | no | `Theme` serialization |
| `text` | no | `ShapedTextMeasure`, `UiTextStyles` |
| `text-render2d` | no | `TextRenderer` GPU widget text |

Core has no graphics, window or ECS dependency. Text features are additive;
the default `UiRenderer` and every existing widget API remain bitmap-backed. Every adapter is optional and
additive. `quasar-config` bridging converts only values; games own persistence,
schema and validation. Falling Sand and Starfall both persist a game-owned TOML
file outside the engine config schema.

## Measurements

`cargo run --release -p quasar-ui --example ui_benchmark` builds 129 nodes and
runs 10 000 updates. On the development machine:

| Metric | Value |
|---|---|
| Update (layout + interaction) | ~22–28 µs/update |
| Layout p50 | ~20–25 µs |
| Layout p99 | ~27–43 µs |
| Events | 276 over the run |

These are single-machine regression numbers, not a portability claim. Layout is
O(nodes) per update. The benchmark is kept in the crate so regressions are
measurable rather than anecdotal.

## Verification

- 7 layout/interaction tests: stack/grid/absolute layout, DPI and safe area,
  scroll clamping and clipping, mouse/keyboard actions, sliders/checkboxes/
  lists/text fields, modal focus trapping and rebind isolation, localization and
  contrast.
- 3 integration tests: `quasar-input` adapter, config value round-trip, bounded
  interaction sequences.
- 3 shaped-text tests: styled measurement/fallback, editing/caret/scroll and
  IME preedit.
- 1 `render2d` unit test: slider value precision follows its step.
- 3 Vulkan render tests (ignored without a device): bitmap UI, shaped UI at 2×
  DPI, clipping/resizing/unchanged-page reuse and resource balance.
- Native verification under Xvfb/Xephyr in both demos: menus render at 1× and
  2× DPI, keyboard apply/cancel, fullscreen/UI-scale persistence and teardown
  balances. Starfall also exercises Unicode fallback and interactive text entry.

## Limitations

- The default renderer has no built-in font rasterizer/atlas; text is measured
  by a supplied `TextMeasure` and drawn by the game or the bitmap font. Enable
  `text`/`text-render2d` for TTF/OTF shaping and glyph atlases (see
  [text](text.md)).
- No rich text or inline images. The bitmap path is single-line and truncation
  is the caller's responsibility; the shaped `quasar-text` path supports
  wrapping, alignment and bidi, but current widget labels use natural single-line
  layout unless they contain explicit `\n`.
- No animation beyond hover/focus/press transitions and no tween/timeline
  integration.
- No persistence/validation/translation loading. `config` only bridges values;
  both demos own their TOML schema.
- No accessibility tree or screen-reader bridge.
- No virtualized list recycling; `List` draws all items.
- One `Ui` targets one viewport; multi-window is not modelled.
- Audits are read-only and cover MicroMages, Jackal, MissileCommand and MattMC.
  BlackOps was unavailable locally and was not inspected.
