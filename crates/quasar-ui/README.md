# quasar-ui

Renderer-independent retained UI shared by games, development tools and the
future optional Quasar editor. The current controls support main menus, pause
screens, options dialogs, HUD widgets, modal confirmations and overlays.
Games can use it independently, without requiring a Quasar app runner, ECS,
editor or one specific renderer.

The editor and visual tilemap tools will use these same public widgets and
Quasar's window/input/rendering infrastructure on one caller-owned VulkanicGAL
device/frame, without Qt/KDE/GTK or a separate editor UI framework. General
controls belong in `quasar-ui`; editor-specific workflows belong in the
optional tools. Docking, trees/tables/inspectors and live embedded viewports
are planned, not implemented. See the [shared UI architecture and roadmap](../../docs/ui.md).

`quasar-ui` owns a retained widget tree, layout, focus, modal scopes,
interaction state, text editing and caller-defined actions. It owns no window,
GPU device, swapchain, ECS or scene graph. A game advances the UI with its own
input snapshot and draws the resulting layout itself or through the optional
`quasar-render-2d` adapter on the same caller-owned VulkanicGAL frame.

```sh
cargo run --release -p quasar-ui --example ui_benchmark
cargo test -p quasar-ui
cargo test -p quasar-ui --features input,config
```

## Feature matrix

| Feature | Default | Adds | Effect |
|---|---|---|---|
| *(none)* | yes | — | Core widget tree, layout, focus, events, themes. No graphics or input crates. |
| `input` | no | `quasar-input` | `UiBindings`, preferred `from_frame_input`, and `from_input_state` adapters. |
| `render2d` | no | `quasar-render-2d` | `UiRenderer` painter and `BitmapTextMeasure`. |
| `config` | no | `quasar-config` | `config_to_value` / `value_to_config` bridges. |
| `serde` | no | `serde` | `Theme` serialization. |
| `text` | no | `quasar-text` | `ShapedTextMeasure`, `UiTextStyles`, shaped caret/hit and grapheme editing. |
| `text-render2d` | no | `text` + `quasar-render-2d/text` | `TextRenderer` for widget text, selection, carets and IME preedit. |

The core crate has no graphics, window or ECS dependency. Every optional
adapter is additive and can be omitted.

## Composition model

A `Ui` owns nodes keyed by `NodeId`. Each node has a `Widget`, a `LayoutItem`,
a `Layer` and optional modal flag. Roots are added with `add_root`; children
with `add_child`. Layout and interaction run in one `update` call.

```rust
use quasar_ui::{
    ActionId, Edges, LayoutItem, Layer, MonospaceMeasure, PanelLayout, Rect, StackDirection,
    Theme, Ui, UiInput, UiText, Widget,
};

let mut ui = Ui::new(Theme::retro());
let panel = ui.add_root(
    Widget::Panel {
        layout: PanelLayout::Stack { direction: StackDirection::Vertical, gap: 8.0 },
        padding: Edges::all(16.0),
        clip: true,
        scrollable: false,
    },
    LayoutItem::fill(),
    Layer::Normal,
    false,
);
ui.add_child(
    panel,
    Widget::Label {
        text: UiText::literal("PAUSED"),
        style: quasar_ui::TextStyle::Heading,
    },
    LayoutItem::auto(),
)?;
ui.add_child(
    panel,
    Widget::Button {
        text: UiText::literal("Resume"),
        action: Some(ActionId::new(1)),
    },
    LayoutItem::auto(),
)?;

let frame = ui.update(
    &UiInput::default(),
    Rect::new(0.0, 0.0, 1280.0, 720.0),
    1.0,
    Edges::ZERO,
    &MonospaceMeasure,
    1.0 / 60.0,
);
assert!(ui.focused().is_some());
# Ok::<(), quasar_ui::UiError>(())
```

`update` returns a `UiFrame` containing the events produced this frame, the
focused node, whether input was consumed, and whether the UI is modal or
capturing a rebind. Games decide what an event means; the crate never invents
game semantics.

### Widgets

| Widget | Purpose | Focusable |
|---|---|---|
| `Panel` | Stack/grid/absolute container, padding, clipping, scrolling | no |
| `Label` | Literal or localized text with `Body`/`Heading`/`Small`/`Dim` style | no |
| `Button` | Keyboard activation or pointer release over original target | yes |
| `Slider` | Bounded, stepped float value | yes |
| `Checkbox` | Boolean toggle | yes |
| `Select` | One-of-many option list | yes |
| `Tabs` | One-of-many tab row | yes |
| `TextInput` | Committed text, cursor/anchor, max length, password | yes |
| `Progress` | Bounded read-only bar | no |
| `Image` | Texture id + source rect + size | no |
| `List` | Selectable row list with scroll index | yes |
| `Binding` | Rebindable action label/value row | yes |
| `Spacer` | Flexible gap | no |

`UiText::keyed(key, fallback)` and `Ui::set_localized` provide localization
without embedding a translation engine. Text measurement is a `TextMeasure`
trait; `MonospaceMeasure` is core, `BitmapTextMeasure` (feature `render2d`)
matches the 6×7 bitmap font used by `quasar-render-2d`.

### Layout

`LayoutItem` supports `Auto`, `Fixed`, `Percent` and `Fill` lengths, margins,
offsets and `Start`/`Center`/`End`/`Stretch` alignment. Panels lay out children
as a vertical/horizontal stack, a fixed-column grid or absolutely positioned.
Panels with `clip` clip descendants to their rect; `scrollable` panels clamp
`scroll` to the content extent and report `content_size`. Layout is computed in
physical pixels: `update` takes a viewport `Rect`, a DPI `scale` and safe-area
`Edges`, so resizing and DPI changes need no widget rebuild.

### Focus, modal scopes and input isolation

- `Ui` focuses the first focusable node automatically on first update;
  `focus(Some(id))` or `set_modal(Some(id))` set the initial focus explicitly.
- Modal roots/children trap focus: nodes outside the modal subtree are not in
  `focus_order` and do not receive pointer or keyboard input.
- `UiFrame::consumed`, `consumed_pointer`, `consumed_keyboard` and
  `consumed_text` report exactly what the UI claimed. Games suppress gameplay
  input for those channels only, so a text field can consume typing while mouse
  look still works if the pointer was not consumed.
- Pointer capture and slider drags survive pointer movement outside the widget.
- Rebinding uses `begin_rebind`, `finish_rebind` and `cancel_rebind`; while
  capturing, navigation and gameplay bindings are isolated and the game decides
  how to turn the captured event into a binding string.
- Navigation: `next`/`previous` cycle focus order. `up`/`down` change a focused
  list/select/tab selection or otherwise cycle focus. `left`/`right` adjust
  sliders/selects/tabs or move the text caret. Spatial grid navigation is not
  implemented. `activate` activates the focused control; `cancel` emits
  `Cancelled` and clears text-field focus when applicable. The caller owns
  page and modal dismissal.
- Focus feedback changes immediately. Keyboard/controller navigation suppresses
  stationary-pointer hover until the mouse moves again. Hover follows menu
  choices without stealing text editing focus. Buttons activate on release
  over the original target, allowing drag-away cancellation; tabs select the
  clicked tab.

### Text editing

`TextInput` supports committed text, cursor and anchor selection, insertion,
backspace/delete, home/end, Select All, shift/drag selection and grapheme-aware
movement. Text arrives as a separate `UiInput::text` string so IME/committed
text is not confused with physical key state; `UiInput::preedit` carries
uncommitted composition text. Games own validation and submit handling through
`TextChanged` and `Submitted`.

With feature `text`, `ShapedTextMeasure` uses `quasar-text` for styled
measurement, fallback and bidi. Its caret/hit methods and the renderer's glyph
positions come from the same shaped layout. With `text-render2d`,
`TextRenderer` warms layouts, keeps a cached glyph atlas and draws labels,
buttons, sliders, lists, text fields and HUD-style labels without replacing the
existing widget API. `UiRenderer::prepare_text` uploads only changed atlas pages;
`UiRenderer::draw` keeps the bitmap path unchanged unless text was enabled.

### Themes and accessibility

`Theme::retro()` and `Theme::modern()` provide pixel-art and modern palettes.
Every colour, size, row height, padding, gap, border and focus width is a public
field; games can build their own `Theme`. `Theme::contrast_ratio` implements the
WCAG 2.x relative-luminance formula for auditing foreground/background pairs.
Widgets expose hover and press transition amounts and immediate focus state
(`WidgetState::focus_t` switches between 0 and 1), so keyboard users get visible
focus as mouse users do. Labels are literal or
localized text; no widget depends on colour alone to convey state.

## Rendering

The core crate emits `LayoutEntry { id, rect, clip, layer, .. }` plus the widget
and its resolved text/state. Any renderer can consume that list. With feature
`render2d`:

```rust
use quasar_ui::UiRenderer;

let mut painter = UiRenderer::new();
painter.set_texture(image_id, texture_id); // for Widget::Image
painter.draw(&mut canvas, &ui);             // appends ordered quads
```

`UiRenderer::draw` only appends quads to the caller's `quasar_render_2d::Canvas`.
The caller still owns the canvas, textures, target, render pass and submission,
so UI can be composited in the same GAL frame as 2D/3D/custom passes in whatever
order the game requires. `Layer::{Normal, Overlay, Hud}` is preserved on each
layout entry for callers that draw layers separately.

## Measurements

`cargo run --release -p quasar-ui --example ui_benchmark` (129 nodes, 10 000
updates, release build on the development machine):

| Metric | Value |
|---|---|
| Update (layout + interaction) | ~22–28 µs/update |
| Layout p50 | ~20–25 µs |
| Layout p99 | ~27–43 µs |
| Events emitted | 276 over the run |

These are single-machine measurements for regression tracking, not a portability
claim. Layout is O(nodes) per update and allocates only event/measure buffers.

## Testing

| Suite | Coverage |
|---|---|
| `tests/layout.rs` | Stack/grid/absolute layout, DPI and safe area, scrolling/clipping, mouse and keyboard actions, sliders/checkboxes/lists/text fields, modal focus trapping and rebinding isolation, localization and contrast |
| `tests/integration.rs` | `quasar-input` adapter, config value round-trip, bounded interaction sequences |
| `tests/interaction.rs` | Immediate focus, mouse/keyboard switching, release/cancel behavior, hidden/disabled targets, tab hit testing, text focus and consumed-frame stationary clicks/slider drags |
| `tests/text.rs` | Styled/fallback shaping, caret-to-hit consistency, grapheme editing, selection, scrolling and IME preedit |
| `src/render2d.rs` tests | Slider value precision derived from the configured step |
| `tests/render.rs` | Vulkan offscreen bitmap/shaped text, 2× DPI, clipping, resizing, atlas reuse and resource balance (ignored without a Vulkan device) |

## Demo usage

- [Falling Sand](../../examples/falling-sand) uses `Theme::retro()`, a retro
  pixel menu stack and a TOML-persisted game-owned UI settings file.
- [Starfall](../../examples/starfall) uses `Theme::modern()` with real TTF text:
  `DejaVu Sans` plus a Devanagari fallback, Greek/Cyrillic/Hebrew/Arabic bidi
  labels, a Unicode callsign text field with IME preedit, and persisted options.
- [Falling Sand](../../examples/falling-sand) deliberately retains the bitmap
  5×7 retro path, proving the font dependencies are optional.

## Limitations

- The default renderer has no built-in font rasterizer or atlas: text is
  measured by a supplied `TextMeasure` and drawn by the game/renderer. `render2d`
  uses the existing bitmap font; `text`/`text-render2d` add shaped TTF/OTF text
  and glyph atlases.
- No rich text or inline images. The bitmap path is single-line; the shaped path
  supports wrapping/bidi, but current labels use natural single-line layout
  unless they contain explicit `\n`. Long text is the caller's responsibility to
  truncate or clip.
- No animations beyond hover/press transitions and no tween/timeline
  integration.
- No built-in persistence, validation or translation loading; `config` only
  bridges values. Games own schema, validation and file format (both demos use
  game-owned TOML).
- No accessibility tree or screen-reader bridge; contrast and focus visibility
  are provided as primitives.
- No virtualized list recycling; `List` draws all items.
- Docking, split panes, trees, tables, property inspectors, clipboard integration
  and live embedded viewport widgets remain planned shared-UI extensions.
- No multi-window support: one `Ui` targets one viewport.
