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
            #[cfg(feature = "text-render2d")]
            if let Some(text) = &mut self.text
                && text.draw_widget(canvas, ui, node, entry)
            {
                continue;
            }
            match &node.widget {
                Widget::Panel { .. } => {
                    let color = if node.modal {
                        theme.panel
                    } else {
                        theme.panel_alt
                    };
                    canvas.rect(rect, to_color(color));
                    if node.modal {
                        focus_border(canvas, rect, theme.focus, theme.focus_width * scale);
                    }
                }
                Widget::Label { text, style } => {
                    let size = match style {
                        crate::TextStyle::Heading => heading,
                        crate::TextStyle::Body => font,
                        crate::TextStyle::Small | crate::TextStyle::Dim => small,
                    };
                    draw_text(
                        canvas,
                        entry.rect.x,
                        entry.rect.y + (entry.rect.height - size).max(0.0) * 0.5,
                        ui.resolve_text(text),
                        size,
                        theme.color(*style),
                    );
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
                    let track = Rect::new(
                        entry.rect.x,
                        entry.rect.y + entry.rect.height - 5.0 * scale,
                        entry.rect.width,
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
                        entry.rect.x,
                        entry.rect.y,
                        ui.resolve_text(label),
                        small,
                        theme.dim_text,
                    );
                    let value_text = format_slider_value(*value, *step);
                    draw_text(
                        canvas,
                        entry.rect.right()
                            - value_text.chars().count() as f32 * small * TEXT_ADVANCE,
                        entry.rect.y,
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

    #[test]
    fn slider_precision_follows_the_step() {
        assert_eq!(format_slider_value(0.0026, 0.0001), "0.0026");
        assert_eq!(format_slider_value(8.0, 1.0), "8");
        assert_eq!(format_slider_value(0.6, 0.05), "0.60");
        assert_eq!(format_slider_value(1.0, 0.25), "1.00");
        assert_eq!(format_slider_value(0.5, 0.0), "0.50");
    }
}
