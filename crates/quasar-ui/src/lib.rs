//! Reusable, renderer-independent in-game UI.
//!
//! `quasar-ui` owns retained widgets, layout, focus, interaction state and
//! caller-defined actions. It owns no window, GPU, ECS or scene graph. A game
//! advances the UI with its own input snapshot and draws the resulting layout
//! itself or through the optional `render2d` adapter.
//!
//! ```
//! use quasar_ui::{ActionId, LayoutItem, Layer, MonospaceMeasure, Rect, Theme, Ui, UiInput, Widget};
//!
//! let mut ui = Ui::new(Theme::retro());
//! let root = ui.add_root(
//!     Widget::Panel {
//!         layout: quasar_ui::PanelLayout::Stack {
//!             direction: quasar_ui::StackDirection::Vertical,
//!             gap: 8.0,
//!         },
//!         padding: quasar_ui::Edges::all(16.0),
//!         clip: true,
//!         scrollable: false,
//!     },
//!     LayoutItem::fill(),
//!     Layer::Normal,
//!     false,
//! );
//! ui.add_child(
//!     root,
//!     Widget::Button {
//!         text: quasar_ui::UiText::literal("Start"),
//!         action: Some(ActionId::new(1)),
//!     },
//!     LayoutItem::auto(),
//! )?;
//! let frame = ui.update(
//!     &UiInput::default(),
//!     Rect::new(0.0, 0.0, 320.0, 200.0),
//!     1.0,
//!     quasar_ui::Edges::ZERO,
//!     &MonospaceMeasure,
//!     1.0 / 60.0,
//! );
//! assert!(!frame.events.is_empty() || ui.layout_entries().len() >= 2);
//! # Ok::<(), quasar_ui::UiError>(())
//! ```
mod math;
mod theme;
mod ui;
mod widget;

pub use math::{Edges, Rect, Vec2};
pub use theme::{Color, TextStyle, Theme};
pub use ui::{Ui, UiError, UiNode, UiResult};
pub use widget::{
    ActionId, Align, Layer, LayoutEntry, LayoutItem, Length, MonospaceMeasure, NodeId, PanelLayout,
    StackDirection, TextMeasure, UiEvent, UiFrame, UiInput, UiText, UiValue, Widget, WidgetState,
};

#[cfg(feature = "input")]
mod input;
#[cfg(feature = "input")]
pub use input::{UiBindings, from_frame_input, from_input_state};

#[cfg(feature = "render2d")]
mod render2d;
#[cfg(feature = "render2d")]
pub use render2d::{BitmapTextMeasure, UiRenderer};

#[cfg(feature = "config")]
mod config;
#[cfg(feature = "config")]
pub use config::{config_to_value, value_to_config};

#[cfg(feature = "text")]
mod text;
#[cfg(feature = "text")]
pub use text::{FontFaceStyle, ShapedTextMeasure, UiTextStyles};
#[cfg(feature = "text-render2d")]
pub use text::{TextRenderer, TextRendererOptions};
