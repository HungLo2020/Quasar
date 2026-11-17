//! Optional `quasar-render-2d` painter. The caller owns the canvas, textures,
//! target, passes and submission; this adapter only appends ordered quads.
use crate::theme::Color;
use crate::widget::Widget;
use crate::{Rect as UiRect, TextMeasure, Ui, Vec2};
use quasar_render_2d::{Canvas, Color as RenderColor, Rect, Sprite, TextureId};
use std::collections::BTreeMap;

/// Horizontal advance of the bitmap font per character, relative to font size.
const TEXT_ADVANCE: f32 = 6.0 / 7.0;

#[derive(Default)]
pub struct UiRenderer {
    textures: BTreeMap<u64, TextureId>,
    #[cfg(feature = "text-render2d")]
    text: Option<crate::TextRenderer>,
}

impl UiRenderer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set_texture(&mut self, image: u64, texture: TextureId) {
        self.textures.insert(image, texture);
    }
    pub fn remove_texture(&mut self, image: u64) {
        self.textures.remove(&image);
    }

    /// Enables shaped-font rendering for all text widgets. Without this call
    /// the renderer keeps the built-in bitmap font, so existing games are
    /// unchanged.
    #[cfg(feature = "text-render2d")]
    pub fn enable_text(&mut self, renderer: crate::TextRenderer) {
        self.text = Some(renderer);
    }

    #[cfg(feature = "text-render2d")]
    pub fn text(&self) -> Option<&crate::TextRenderer> {
        self.text.as_ref()
    }

    #[cfg(feature = "text-render2d")]
    pub fn text_mut(&mut self) -> Option<&mut crate::TextRenderer> {
        self.text.as_mut()
    }

    /// Warms text layouts and uploads changed atlas pages before `draw`.
    /// Call once per frame after `Ui::update` and before drawing the canvas.
    #[cfg(feature = "text-render2d")]
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_text(
        &mut self,
        state: &mut quasar_render_2d::RenderState2D,
        gal: &quasar_render_2d::Gpu,
        textures: &mut quasar_render_2d::Textures,
        ui: &Ui,
    ) -> quasar_render_2d::GraphicsResult<()> {
        if let Some(text) = &mut self.text {
            text.prepare(state, gal, textures, ui)?;
        }
        Ok(())
    }

    pub fn draw(&mut self, canvas: &mut Canvas, ui: &Ui) {
        let theme = ui.theme();
        let scale = ui.scale();
        let font = theme.font_size * scale;
        let small = theme.small_size * scale;
        let heading = theme.heading_size * scale;
        let padding = theme.padding * scale;
        for entry in ui.layout_entries() {
            let Some(node) = ui.node(entry.id) else {
                continue;
            };
            if !node.visible {
                continue;
            }
            canvas.set_clip(Some(to_rect(entry.clip)));
            let rect = to_rect(entry.rect);
            if let Some(style) = &node.style {
                paint_style(canvas, entry.rect, style, scale);
            }
            #[cfg(feature = "text-render2d")]
            if let Some(text) = &mut self.text
                && text.draw_widget(canvas, ui, node, entry)
            {
                continue;
            }
            match &node.widget {
                Widget::Panel { .. } if node.style.is_some() => {}
                Widget::Panel { .. } => {
                    let color = if node.modal {
                        theme.panel
                    } else {
                        theme.panel_alt
                    };
                    canvas.rect(rect, to_color(color));
                    if node.modal && theme.modal_outline {
                        focus_border(canvas, rect, theme.focus, theme.focus_width * scale);
                    }
                }
                Widget::Label {
                    text, style, wrap, ..
                } => {
                    let size = match style {
                        crate::TextStyle::Heading => heading,
                        crate::TextStyle::Body => font,
                        crate::TextStyle::Small | crate::TextStyle::Dim => small,
                    };
                    let text = ui.resolve_text(text);
                    let lines = match wrap {
                        crate::LabelWrap::None => text.split('\n').map(str::to_owned).collect(),
                        crate::LabelWrap::Word => {
                            crate::wrap_greedy(text, entry.rect.width + 0.5, |part| {
                                part.chars().count() as f32 * size * TEXT_ADVANCE
                            })
                        }
                    };
                    let total = size * lines.len() as f32;
                    let top = entry.rect.y + (entry.rect.height - total).max(0.0) * 0.5;
                    for (index, line) in lines.iter().enumerate() {
                        draw_text(
                            canvas,
                            entry.rect.x,
                            top + index as f32 * size,
                            line,
                            size,
                            theme.color(*style),
                        );
                    }
                }
                Widget::Button { text, .. } => {
                    let fill = if node.state.pressed {
                        theme.button_active
                    } else if node.state.hovered || node.state.focus_t > 0.5 {
                        theme.button_hover
                    } else {
                        theme.button
                    };
                    canvas.rect(rect, to_color(fill));
                    draw_centered_text(
                        canvas,
                        to_rect(entry.rect),
                        ui.resolve_text(text),
                        font,
                        if node.enabled {
                            theme.text
                        } else {
                            theme.disabled
                        },
                    );
                    if node.state.focus_t > 0.01 {
                        focus_border(
                            canvas,
                            rect,
                            theme.focus.with_alpha((255.0 * node.state.focus_t) as u8),
                            theme.focus_width * scale,
                        );
                    }
                }
                Widget::Slider {
                    label,
                    value,
                    min,
                    max,
                    step,
                    ..
                } => {
                    let inner = entry
                        .rect
                        .inset(crate::Edges::all(theme.control_inset * scale));
                    let track = Rect::new(
                        inner.x,
                        inner.y + inner.height - 5.0 * scale,
                        inner.width,
                        4.0 * scale,
                    );
                    canvas.rect(track, to_color(theme.track));
                    let fraction = if *max > *min {
                        ((*value - *min) / (*max - *min)).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    canvas.rect(
                        Rect::new(track.x, track.y, track.width * fraction, track.height),
                        to_color(theme.fill),
                    );
                    let knob = 6.0 * scale;
                    canvas.rect(
                        Rect::new(
                            track.x + track.width * fraction - knob * 0.5,
                            track.y - knob * 0.5,
                            knob,
                            knob,
                        ),
                        to_color(if node.state.focus_t > 0.5 {
                            theme.focus
                        } else {
                            theme.accent
                        }),
                    );
                    draw_text(
                        canvas,
                        inner.x,
                        inner.y,
                        ui.resolve_text(label),
                        small,
                        theme.dim_text,
                    );
                    let value_text = format_slider_value(*value, *step);
                    draw_text(
                        canvas,
                        inner.right() - value_text.chars().count() as f32 * small * TEXT_ADVANCE,
                        inner.y,
                        &value_text,
                        small,
                        theme.text,
                    );
                    if node.state.focus_t > 0.01 {
                        focus_border(canvas, rect, theme.focus, theme.focus_width * scale);
                    }
                }
                Widget::Checkbox { label, checked, .. } => {
                    let box_size = (entry.rect.height - 4.0 * scale).max(8.0);
                    let box_rect =
                        Rect::new(entry.rect.x, entry.rect.y + 2.0 * scale, box_size, box_size);
                    canvas.rect(box_rect, to_color(theme.track));
                    if *checked {
                        canvas.rect(
                            Rect::new(
                                box_rect.x + 3.0 * scale,
                                box_rect.y + 3.0 * scale,
                                box_size - 6.0 * scale,
                                box_size - 6.0 * scale,
                            ),
                            to_color(theme.fill),
                        );
                    }
                    draw_text(
                        canvas,
                        box_rect.x + box_rect.width + 6.0 * scale,
                        entry.rect.y + (entry.rect.height - font).max(0.0) * 0.5,
                        ui.resolve_text(label),
                        font,
                        theme.text,
                    );
                    if node.state.focus_t > 0.01 {
                        focus_border(canvas, rect, theme.focus, theme.focus_width * scale);
                    }
                }
                Widget::Select {
                    label,
                    options,
                    selected,
                    ..
                } => {
                    canvas.rect(rect, to_color(theme.button));
                    draw_text(
                        canvas,
                        rect.x + padding,
                        rect.y + padding,
                        ui.resolve_text(label),
                        font,
                        theme.text,
                    );
                    if let Some(option) = options.get(*selected) {
                        let text = ui.resolve_text(option);
                        draw_text(
                            canvas,
                            rect.x + rect.width
                                - text.chars().count() as f32 * font * TEXT_ADVANCE
                                - padding,
                            rect.y + padding,
                            text,
                            font,
                            theme.accent,
                        );
                    }
                    if node.state.focus_t > 0.01 {
                        focus_border(canvas, rect, theme.focus, theme.focus_width * scale);
                    }
                }
                Widget::Tabs { tabs, selected, .. } => {
                    let width = rect.width / tabs.len().max(1) as f32;
                    for (index, tab) in tabs.iter().enumerate() {
                        let tab_rect =
                            Rect::new(rect.x + index as f32 * width, rect.y, width, rect.height);
                        if index == *selected {
                            canvas.rect(tab_rect, to_color(theme.selection));
                        }
                        draw_centered_text(
                            canvas,
                            tab_rect,
                            ui.resolve_text(tab),
                            font,
                            if index == *selected {
                                theme.accent
                            } else {
                                theme.dim_text
                            },
                        );
                    }
                    if node.state.focus_t > 0.01 {
                        focus_border(canvas, rect, theme.focus, theme.focus_width * scale);
                    }
                }
                Widget::TextInput {
                    text,
                    placeholder,
                    cursor,
                    password,
                    ..
                } => {
                    canvas.rect(rect, to_color(theme.track));
                    let value = if text.is_empty() {
                        ui.resolve_text(placeholder).to_owned()
                    } else if *password {
                        "*".repeat(text.chars().count())
                    } else {
                        text.clone()
                    };
                    let color = if text.is_empty() {
                        theme.dim_text
                    } else {
                        theme.text
                    };
                    draw_text(
                        canvas,
                        rect.x + padding,
                        rect.y + (rect.height - font).max(0.0) * 0.5,
                        &value,
                        font,
                        color,
                    );
                    if node.state.focus_t > 0.5 {
                        let prefix: String = text.chars().take(*cursor).collect();
                        let x =
                            rect.x + padding + prefix.chars().count() as f32 * font * TEXT_ADVANCE;
                        canvas.rect(
                            Rect::new(x, rect.y + padding * 0.5, 1.0, rect.height - padding),
                            to_color(theme.focus),
                        );
                    }
                    if node.state.focus_t > 0.01 {
                        focus_border(canvas, rect, theme.focus, theme.focus_width * scale);
                    }
                }
                Widget::Progress { label, value, max } => {
                    canvas.rect(rect, to_color(theme.track));
                    let fraction = if *max > 0.0 {
                        (*value / *max).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    canvas.rect(
                        Rect::new(rect.x, rect.y, rect.width * fraction, rect.height),
                        to_color(theme.fill),
                    );
                    draw_text(
                        canvas,
                        rect.x + padding,
                        rect.y + padding,
                        ui.resolve_text(label),
                        small,
                        theme.text,
                    );
                }
                Widget::Image { image, source, .. } => {
                    if let Some(texture) = self.textures.get(image) {
                        let mut sprite = Sprite::new(*texture, rect);
                        sprite.source = source.map(to_rect);
                        canvas.sprite(sprite);
                    }
                }
                Widget::List {
                    items,
                    selected,
                    scroll,
                    ..
                } => {
                    canvas.rect(rect, to_color(theme.panel));
                    let row = ui.theme().row_height * scale;
                    for (index, item) in items.iter().enumerate().skip(*scroll).take(8) {
                        let row_rect = Rect::new(
                            rect.x,
                            rect.y + (index - *scroll) as f32 * row,
                            rect.width,
                            row,
                        );
                        if index == *selected {
                            canvas.rect(row_rect, to_color(theme.selection));
                        }
                        draw_text(
                            canvas,
                            row_rect.x + padding,
                            row_rect.y + (row - font).max(0.0) * 0.5,
                            ui.resolve_text(item),
                            font,
                            if index == *selected {
                                theme.accent
                            } else {
                                theme.text
                            },
                        );
                    }
                    if node.state.focus_t > 0.01 {
                        focus_border(canvas, rect, theme.focus, theme.focus_width * scale);
                    }
                }
                Widget::Binding { label, value, .. } => {
                    canvas.rect(rect, to_color(theme.button));
                    draw_text(
                        canvas,
                        rect.x + padding,
                        rect.y + padding,
                        ui.resolve_text(label),
                        font,
                        theme.text,
                    );
                    draw_text(
                        canvas,
                        rect.x + rect.width
                            - value.chars().count() as f32 * font * TEXT_ADVANCE
                            - padding,
                        rect.y + padding,
                        value,
                        font,
                        theme.accent,
                    );
                    if node.state.focus_t > 0.01 {
                        focus_border(canvas, rect, theme.focus, theme.focus_width * scale);
                    }
                }
                Widget::Spacer => {}
            }
        }
        canvas.set_clip(None);
    }
}

pub struct BitmapTextMeasure;

impl TextMeasure for BitmapTextMeasure {
    fn measure(&self, text: &str, size: f32) -> Vec2 {
        Vec2::new(text.chars().count() as f32 * size * TEXT_ADVANCE, size)
    }
}

fn to_color(color: Color) -> RenderColor {
    RenderColor([color.r, color.g, color.b, color.a])
}

/// Slider values are shown with enough precision for their step so small
/// values (for example mouse sensitivity) do not collapse to `0.00`.
pub(crate) fn format_slider_value(value: f32, step: f32) -> String {
    let step = step.abs();
    let decimals = if !step.is_finite() || step <= 0.0 {
        2
    } else if step >= 1.0 {
        0
    } else {
        let mut decimals = 0;
        let mut scaled = step;
        while decimals < 6 && (scaled - scaled.round()).abs() > 1e-5 * scaled.abs().max(1.0) {
            scaled *= 10.0;
            decimals += 1;
        }
        decimals
    };
    format!("{value:.decimals$}")
}

fn to_rect(rect: UiRect) -> Rect {
    Rect::new(rect.x, rect.y, rect.width, rect.height)
}

fn draw_text(canvas: &mut Canvas, x: f32, y: f32, text: &str, size: f32, color: Color) {
    if text.is_empty() || size <= 0.0 {
        return;
    }
    canvas.text([x, y], text, size / 7.0, to_color(color));
}

fn draw_centered_text(canvas: &mut Canvas, rect: Rect, text: &str, size: f32, color: Color) {
    let width = text.chars().count() as f32 * size * TEXT_ADVANCE;
    draw_text(
        canvas,
        rect.x + (rect.width - width).max(0.0) * 0.5,
        rect.y + (rect.height - size).max(0.0) * 0.5,
        text,
        size,
        color,
    );
}

/// Paints a [`crate::NodeStyle`] background and border with rounded corners.
pub(crate) fn paint_style(canvas: &mut Canvas, rect: UiRect, style: &crate::NodeStyle, scale: f32) {
    let radius = style.radius * scale;
    if let Some(color) = style.background
        && color.a > 0
    {
        for span in crate::rounded_rect_spans(rect, radius) {
            canvas.rect(to_rect(span), to_color(color));
        }
    }
    if let Some(border) = style.border
        && border.color.a > 0
    {
        for span in crate::rounded_border_spans(rect, radius, border.width * scale) {
            canvas.rect(to_rect(span), to_color(border.color));
        }
    }
}

fn focus_border(canvas: &mut Canvas, rect: Rect, color: Color, width: f32) {
    let color = to_color(color);
    canvas.rect(Rect::new(rect.x, rect.y, rect.width, width), color);
    canvas.rect(
        Rect::new(rect.x, rect.y + rect.height - width, rect.width, width),
        color,
    );
    canvas.rect(Rect::new(rect.x, rect.y, width, rect.height), color);
    canvas.rect(
        Rect::new(rect.x + rect.width - width, rect.y, width, rect.height),
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::format_slider_value;
    use crate::{
        Edges, Layer, LayoutItem, Length, NodeStyle, PanelLayout, Rect, Theme, Ui, UiInput,
        UiRenderer, UiText, Widget,
    };
    use quasar_render_2d::Canvas;

    fn panel() -> Widget {
        Widget::Panel {
            layout: PanelLayout::Absolute,
            padding: Edges::ZERO,
            clip: false,
            scrollable: false,
        }
    }

    fn settle(ui: &mut Ui) {
        for _ in 0..30 {
            ui.update(
                &UiInput::default(),
                Rect::new(0.0, 0.0, 400.0, 300.0),
                1.0,
                Edges::ZERO,
                &super::BitmapTextMeasure,
                1.0 / 30.0,
            );
        }
    }

    fn focus_quads(canvas: &Canvas, theme: &Theme) -> usize {
        let focus = super::to_color(theme.focus);
        canvas.quads().iter().filter(|q| q.color == focus).count()
    }

    fn draw(ui: &Ui) -> Canvas {
        let mut canvas = Canvas::new(400, 300).unwrap();
        UiRenderer::new().draw(&mut canvas, ui);
        canvas
    }

    #[test]
    fn styled_modal_panels_draw_no_focus_outline() {
        let mut ui = Ui::new(Theme::modern());
        let dimmer = ui.add_root(panel(), LayoutItem::fill(), Layer::Overlay, true);
        ui.set_style(
            dimmer,
            Some(NodeStyle {
                background: Some(crate::Color::rgba(0, 0, 0, 160)),
                border: None,
                radius: 0.0,
            }),
        )
        .unwrap();
        ui.set_modal(Some(dimmer));
        settle(&mut ui);
        assert_eq!(focus_quads(&draw(&ui), ui.theme()), 0);
    }

    #[test]
    fn unstyled_modal_outline_follows_the_theme() {
        for outline in [true, false] {
            let mut theme = Theme::modern();
            theme.modal_outline = outline;
            let mut ui = Ui::new(theme);
            let modal = ui.add_root(
                panel(),
                LayoutItem::size(Length::fixed(100.0), Length::fixed(80.0)),
                Layer::Overlay,
                true,
            );
            ui.set_modal(Some(modal));
            settle(&mut ui);
            assert_eq!(focus_quads(&draw(&ui), ui.theme()) > 0, outline);
        }
    }

    #[test]
    fn focused_slider_contents_sit_inside_the_focus_outline() {
        let mut ui = Ui::new(Theme::modern());
        let root = ui.add_root(panel(), LayoutItem::fill(), Layer::Normal, false);
        ui.set_style(root, Some(NodeStyle::transparent())).unwrap();
        let slider = ui
            .add_child(
                root,
                Widget::Slider {
                    label: UiText::literal("Volume"),
                    value: 0.5,
                    min: 0.0,
                    max: 1.0,
                    step: 0.1,
                    action: None,
                },
                LayoutItem::size(Length::fixed(240.0), Length::Auto),
            )
            .unwrap();
        ui.focus(Some(slider));
        settle(&mut ui);
        let rect = ui
            .layout_entries()
            .iter()
            .find(|e| e.id == slider)
            .unwrap()
            .rect;
        let canvas = draw(&ui);
        let theme = ui.theme();
        let focus = super::to_color(theme.focus);
        let outline = theme.focus_width;
        let mut contents = 0;
        for quad in canvas.quads() {
            let (x0, x1) = (quad.origin[0], quad.origin[0] + quad.x[0]);
            let (y0, y1) = (quad.origin[1], quad.origin[1] + quad.y[1]);
            let touches = |a: f32, b: f32| (a - b).abs() < 1e-3;
            let is_outline = quad.color == focus
                && (touches(y0, rect.y)
                    || touches(y1, rect.bottom())
                    || touches(x0, rect.x)
                    || touches(x1, rect.right()));
            if is_outline {
                continue;
            }
            contents += 1;
            assert!(
                y0 >= rect.y + outline - 1e-3 && y1 <= rect.bottom() - outline + 1e-3,
                "slider content touches the focus outline: {quad:?} in {rect:?}"
            );
        }
        assert!(contents > 3, "label, track, fill and knob drawn");
    }

    #[test]
    fn slider_precision_follows_the_step() {
        assert_eq!(format_slider_value(0.0026, 0.0001), "0.0026");
        assert_eq!(format_slider_value(8.0, 1.0), "8");
        assert_eq!(format_slider_value(0.6, 0.05), "0.60");
        assert_eq!(format_slider_value(1.0, 0.25), "1.00");
        assert_eq!(format_slider_value(0.5, 0.0), "0.50");
    }
}
