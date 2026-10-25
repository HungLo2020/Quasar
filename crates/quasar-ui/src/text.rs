//! Shaped text integration and optional GPU text rendering for widgets.
use crate::{TextMeasure, TextStyle, Vec2};
use quasar_text::{SharedLayout, TextEngine, TextStyle as TextFormat};
use std::cell::RefCell;
use std::rc::Rc;

/// Font attributes for one semantic UI text style.
#[derive(Clone, Debug, PartialEq)]
pub struct FontFaceStyle {
    pub family: Option<String>,
    pub weight: u16,
    pub italic: bool,
    /// Multiplier applied to the physical font size for line spacing.
    pub line_height: f32,
}

impl FontFaceStyle {
    pub fn new() -> Self {
        Self {
            family: None,
            weight: 400,
            italic: false,
            line_height: 1.25,
        }
    }

    pub fn family(mut self, family: impl Into<String>) -> Self {
        self.family = Some(family.into());
        self
    }

    pub fn weight(mut self, weight: u16) -> Self {
        self.weight = weight;
        self
    }

    pub fn italic(mut self, italic: bool) -> Self {
        self.italic = italic;
        self
    }

    pub fn line_height(mut self, line_height: f32) -> Self {
        self.line_height = line_height;
        self
    }
}

impl Default for FontFaceStyle {
    fn default() -> Self {
        Self::new()
    }
}

/// Semantic mapping from [`crate::TextStyle`] to shaped font attributes.
#[derive(Clone, Debug, PartialEq)]
pub struct UiTextStyles {
    pub body: FontFaceStyle,
    pub heading: FontFaceStyle,
    pub small: FontFaceStyle,
    pub dim: FontFaceStyle,
}

impl UiTextStyles {
    pub fn new(family: impl Into<String>) -> Self {
        let family = family.into();
        let base = FontFaceStyle::new().family(family.clone());
        Self {
            body: base.clone(),
            heading: base.clone().weight(700),
            small: base.clone(),
            dim: base,
        }
    }

    pub fn plain() -> Self {
        Self {
            body: FontFaceStyle::new(),
            heading: FontFaceStyle::new().weight(700),
            small: FontFaceStyle::new(),
            dim: FontFaceStyle::new(),
        }
    }

    pub fn for_style(&self, style: TextStyle) -> &FontFaceStyle {
        match style {
            TextStyle::Body => &self.body,
            TextStyle::Heading => &self.heading,
            TextStyle::Small => &self.small,
            TextStyle::Dim => &self.dim,
        }
    }

    pub fn format(&self, style: TextStyle, size: f32) -> TextFormat {
        let face = self.for_style(style);
        TextFormat {
            family: face.family.clone(),
            size,
            line_height: (size * face.line_height).max(size),
            weight: face.weight,
            italic: face.italic,
            align: quasar_text::TextAlign::Start,
            wrap: quasar_text::TextWrap::None,
        }
    }
}

/// `TextMeasure` backed by a shared [`TextEngine`] and UI style map.
///
/// The same handle can be cloned for the renderer, so layout measurement,
/// hit-testing, caret placement and glyph rasterization use one font database
/// and one shaping cache.
#[derive(Clone)]
pub struct ShapedTextMeasure {
    engine: Rc<RefCell<TextEngine>>,
    styles: UiTextStyles,
}

impl ShapedTextMeasure {
    pub fn new(engine: Rc<RefCell<TextEngine>>, styles: UiTextStyles) -> Self {
        Self { engine, styles }
    }

    pub fn engine(&self) -> Rc<RefCell<TextEngine>> {
        self.engine.clone()
    }

    pub fn styles(&self) -> &UiTextStyles {
        &self.styles
    }

    pub fn layout(
        &self,
        text: &str,
        style: TextStyle,
        size: f32,
        width: Option<f32>,
    ) -> Option<SharedLayout> {
        let format = self.styles.format(style, size);
        self.engine.borrow_mut().layout(text, &format, width).ok()
    }

    fn layout_or_empty(&self, text: &str, style: TextStyle, size: f32) -> Option<SharedLayout> {
        self.layout(text, style, size, None)
    }
}

impl TextMeasure for ShapedTextMeasure {
    fn measure(&self, text: &str, size: f32) -> Vec2 {
        self.measure_style(text, TextStyle::Body, size)
    }

    fn measure_style(&self, text: &str, style: TextStyle, size: f32) -> Vec2 {
        if let Some(layout) = self.layout_or_empty(text, style, size) {
            Vec2::new(layout.width(), layout.height())
        } else {
            Vec2::new(text.chars().count() as f32 * size * 0.6, size)
        }
    }

    fn caret_index(&self, text: &str, style: TextStyle, size: f32, x: f32) -> Option<usize> {
        let layout = self.layout_or_empty(text, style, size)?;
        Some(layout.hit(x, 1.0).min(layout.char_count()))
    }

    fn caret_x(&self, text: &str, style: TextStyle, size: f32, index: usize) -> Option<f32> {
        let layout = self.layout_or_empty(text, style, size)?;
        Some(layout.caret(index).x)
    }

    fn previous_boundary(&self, text: &str, index: usize) -> usize {
        self.layout_or_empty(text, TextStyle::Body, 16.0)
            .map_or_else(
                || index.min(text.chars().count()).saturating_sub(1),
                |layout| layout.previous_boundary(index),
            )
    }

    fn next_boundary(&self, text: &str, index: usize) -> usize {
        self.layout_or_empty(text, TextStyle::Body, 16.0)
            .map_or_else(
                || (index + 1).min(text.chars().count()),
                |layout| layout.next_boundary(index),
            )
    }
}

#[cfg(feature = "text-render2d")]
mod gpu {
    use super::{ShapedTextMeasure, UiTextStyles};
    use crate::render2d::format_slider_value;
    use crate::{Color, LayoutEntry, Rect, TextStyle, Theme, Ui, UiNode, Widget};
    use quasar_render_2d::{
        Canvas, Color as RenderColor, Filter, GalResult, GpuTextAtlas, Rect as RenderRect,
        RenderState2D, Sprite, Textures, VulkanicGal,
    };
    use quasar_text::{AtlasConfig, GlyphCache, SharedLayout, TextLayout, TextResult};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub struct TextRendererOptions {
        pub atlas: AtlasConfig,
    }

    /// Shaped UI text renderer. `prepare` warms text layouts and uploads changed
    /// atlas pages; `draw_widget` appends glyph quads and editing decorations
    /// through the caller's existing `Canvas`.
    pub struct TextRenderer {
        measure: ShapedTextMeasure,
        engine: Rc<RefCell<quasar_text::TextEngine>>,
        glyphs: GlyphCache,
        gpu: GpuTextAtlas,
    }

    impl TextRenderer {
        pub fn new(
            engine: Rc<RefCell<quasar_text::TextEngine>>,
            styles: UiTextStyles,
            options: TextRendererOptions,
        ) -> TextResult<Self> {
            engine.borrow_mut().set_layout_cache_capacity(4096);
            Ok(Self {
                measure: ShapedTextMeasure::new(engine.clone(), styles),
                engine,
                glyphs: GlyphCache::new(options.atlas)?,
                gpu: GpuTextAtlas::new(),
            })
        }

        pub fn measure(&self) -> ShapedTextMeasure {
            self.measure.clone()
        }

        pub fn measure_ref(&self) -> &ShapedTextMeasure {
            &self.measure
        }

        pub fn glyphs(&self) -> &GlyphCache {
            &self.glyphs
        }

        pub fn gpu(&self) -> &GpuTextAtlas {
            &self.gpu
        }

        pub fn prepare(
            &mut self,
            state: &mut RenderState2D,
            gal: &mut VulkanicGal,
            textures: &mut Textures,
            ui: &Ui,
        ) -> GalResult<()> {
            let theme = ui.theme();
            let scale = ui.scale();
            for entry in ui.layout_entries() {
                let Some(node) = ui.node(entry.id) else {
                    continue;
                };
                if !node.visible {
                    continue;
                }
                for (text, style, size) in self.texts(ui, node, theme, scale) {
                    let format = self.measure.styles().format(style, size);
                    let mut engine = self.engine.borrow_mut();
                    if let Ok(layout) = engine.layout(&text, &format, None) {
                        self.glyphs.warm(&mut engine, &layout);
                    }
                }
            }
            self.gpu.sync(state, gal, textures, self.glyphs.atlas_mut())
        }

        /// Draws all text belonging to one widget. Returns false for widgets
        /// without text so the caller can use its bitmap path.
        pub fn draw_widget(
            &mut self,
            canvas: &mut Canvas,
            ui: &Ui,
            node: &UiNode,
            entry: &LayoutEntry,
        ) -> bool {
            if self.gpu.page_count() == 0 {
                return false;
            }
            let theme = ui.theme();
            let scale = ui.scale();
            let rect = to_rect(entry.rect);
            let padding = theme.padding * scale;
            let font = theme.font_size * scale;
            let small = theme.small_size * scale;
            match &node.widget {
                Widget::Label { text, style } => {
                    let size = theme.size(*style) * scale;
                    let Some(layout) = self.layout(ui.resolve_text(text), *style, size) else {
                        return true;
                    };
                    let y = rect.y + (rect.height - layout.height()).max(0.0) * 0.5;
                    self.draw_layout(canvas, &layout, rect.x, y, theme.color(*style));
                    true
                }
                Widget::Button { text, .. } => {
                    let fill = if node.state.pressed {
                        theme.button_active
                    } else if node.state.hovered || node.state.focus_t > 0.5 {
                        theme.button_hover
                    } else {
                        theme.button
                    };
                    canvas.rect(rect, render_color(fill));
                    let Some(layout) = self.layout(ui.resolve_text(text), TextStyle::Body, font)
                    else {
                        return true;
                    };
                    let x = rect.x + (rect.width - layout.width()).max(0.0) * 0.5;
                    let y = rect.y + (rect.height - layout.height()).max(0.0) * 0.5;
                    self.draw_layout(
                        canvas,
                        &layout,
                        x,
                        y,
                        if node.enabled {
                            theme.text
                        } else {
                            theme.disabled
                        },
                    );
                    if node.state.focus_t > 0.01 {
                        focus_border(canvas, rect, theme.focus, theme.focus_width * scale);
                    }
                    true
                }
                Widget::Slider {
                    label,
                    value,
                    min,
                    max,
                    step,
                    ..
                } => {
                    let track = RenderRect::new(
                        rect.x,
                        rect.y + rect.height - 5.0 * scale,
                        rect.width,
                        4.0 * scale,
                    );
                    canvas.rect(track, render_color(theme.track));
                    let fraction = if *max > *min {
                        ((*value - *min) / (*max - *min)).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    canvas.rect(
                        RenderRect::new(track.x, track.y, track.width * fraction, track.height),
                        render_color(theme.fill),
                    );
                    let knob = 6.0 * scale;
                    canvas.rect(
                        RenderRect::new(
                            track.x + track.width * fraction - knob * 0.5,
                            track.y - knob * 0.5,
                            knob,
                            knob,
                        ),
                        render_color(if node.state.focus_t > 0.5 {
                            theme.focus
                        } else {
                            theme.accent
                        }),
                    );
                    if let Some(layout) =
                        self.layout(ui.resolve_text(label), TextStyle::Small, small)
                    {
                        self.draw_layout(canvas, &layout, rect.x, rect.y, theme.dim_text);
                    }
                    let value_text = format_slider_value(*value, *step);
                    if let Some(layout) = self.layout(&value_text, TextStyle::Small, small) {
                        self.draw_layout(
                            canvas,
                            &layout,
                            rect.x + rect.width - layout.width(),
                            rect.y,
                            theme.text,
                        );
                    }
                    let _ = (min, max);
                    if node.state.focus_t > 0.01 {
                        focus_border(canvas, rect, theme.focus, theme.focus_width * scale);
                    }
                    true
                }
                Widget::Checkbox { label, checked, .. } => {
                    let box_size = (rect.height - 4.0 * scale).max(8.0);
                    let box_rect =
                        RenderRect::new(rect.x, rect.y + 2.0 * scale, box_size, box_size);
                    canvas.rect(box_rect, render_color(theme.track));
                    if *checked {
                        canvas.rect(
                            RenderRect::new(
                                box_rect.x + 3.0 * scale,
                                box_rect.y + 3.0 * scale,
                                (box_size - 6.0 * scale).max(1.0),
                                (box_size - 6.0 * scale).max(1.0),
                            ),
                            render_color(theme.fill),
                        );
                    }
                    if let Some(layout) = self.layout(ui.resolve_text(label), TextStyle::Body, font)
                    {
                        let x = rect.x + box_size + 6.0 * scale;
                        let y = rect.y + (rect.height - layout.height()).max(0.0) * 0.5;
                        self.draw_layout(canvas, &layout, x, y, theme.text);
                    }
                    if node.state.focus_t > 0.01 {
                        focus_border(canvas, rect, theme.focus, theme.focus_width * scale);
                    }
                    true
                }
                Widget::Select {
                    label,
                    options,
                    selected,
                    ..
                } => {
                    canvas.rect(rect, render_color(theme.button));
                    if let Some(layout) = self.layout(ui.resolve_text(label), TextStyle::Body, font)
                    {
                        self.draw_layout(
                            canvas,
                            &layout,
                            rect.x + padding,
                            rect.y + padding,
                            theme.text,
                        );
                    }
                    if let Some(option) = options.get(*selected)
                        && let Some(layout) =
                            self.layout(ui.resolve_text(option), TextStyle::Body, font)
                    {
                        self.draw_layout(
                            canvas,
                            &layout,
                            rect.x + rect.width - layout.width() - padding,
                            rect.y + padding,
                            theme.accent,
                        );
                    }
                    if node.state.focus_t > 0.01 {
                        focus_border(canvas, rect, theme.focus, theme.focus_width * scale);
                    }
                    true
                }
                Widget::Tabs { tabs, selected, .. } => {
                    let width = rect.width / tabs.len().max(1) as f32;
                    for (index, tab) in tabs.iter().enumerate() {
                        let Some(layout) = self.layout(ui.resolve_text(tab), TextStyle::Body, font)
                        else {
                            continue;
                        };
                        let tab_rect = RenderRect::new(
                            rect.x + index as f32 * width,
                            rect.y,
                            width,
                            rect.height,
                        );
                        if index == *selected {
                            canvas.rect(tab_rect, render_color(theme.selection));
                        }
                        let x = tab_rect.x + (tab_rect.width - layout.width()).max(0.0) * 0.5;
                        let y = tab_rect.y + (tab_rect.height - layout.height()).max(0.0) * 0.5;
                        self.draw_layout(
                            canvas,
                            &layout,
                            x,
                            y,
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
                    true
                }
                Widget::TextInput {
                    text,
                    placeholder,
                    cursor,
                    anchor,
                    password,
                    ..
                } => {
                    canvas.rect(rect, render_color(theme.track));
                    let empty = text.is_empty();
                    let value = if empty {
                        ui.resolve_text(placeholder).to_owned()
                    } else if *password {
                        "*".repeat(text.chars().count())
                    } else {
                        text.clone()
                    };
                    let Some(layout) = self.layout(&value, TextStyle::Body, font) else {
                        return true;
                    };
                    let inner = RenderRect::new(
                        rect.x + padding,
                        rect.y + padding * 0.5,
                        (rect.width - padding * 2.0).max(1.0),
                        (rect.height - padding).max(1.0),
                    );
                    canvas.set_clip(Some(inner));
                    let scroll = node.text_scroll;
                    let origin_x = inner.x - scroll;
                    let origin_y = rect.y + (rect.height - layout.height()).max(0.0) * 0.5;
                    let color = if empty { theme.dim_text } else { theme.text };
                    if !empty && node.state.focus_t > 0.01 {
                        for selection in layout.selection_rects(*anchor, *cursor) {
                            canvas.rect(
                                RenderRect::new(
                                    origin_x + selection.x,
                                    origin_y + selection.y,
                                    selection.width,
                                    selection.height,
                                ),
                                RenderColor([
                                    theme.selection.r,
                                    theme.selection.g,
                                    theme.selection.b,
                                    190,
                                ]),
                            );
                        }
                    }
                    self.draw_layout(canvas, &layout, origin_x, origin_y, color);
                    if node.state.focus_t > 0.5 {
                        let caret = layout.caret(*cursor);
                        canvas.rect(
                            RenderRect::new(
                                origin_x + caret.x,
                                origin_y + caret.top,
                                1.0,
                                caret.height.max(1.0),
                            ),
                            RenderColor([
                                theme.focus.r,
                                theme.focus.g,
                                theme.focus.b,
                                theme.focus.a,
                            ]),
                        );
                        if !node.preedit.is_empty()
                            && let Some(preedit) = self.layout(&node.preedit, TextStyle::Body, font)
                        {
                            let x = origin_x + caret.x;
                            self.draw_layout(canvas, &preedit, x, origin_y, theme.accent);
                            canvas.rect(
                                RenderRect::new(
                                    x,
                                    origin_y + caret.top + preedit.height() - 1.0,
                                    preedit.width(),
                                    1.0,
                                ),
                                RenderColor([theme.accent.r, theme.accent.g, theme.accent.b, 220]),
                            );
                        }
                    }
                    canvas.set_clip(Some(to_rect(entry.clip)));
                    if node.state.focus_t > 0.01 {
                        focus_border(canvas, rect, theme.focus, theme.focus_width * scale);
                    }
                    true
                }
                Widget::Progress { label, value, max } => {
                    canvas.rect(rect, render_color(theme.track));
                    let fraction = if *max > 0.0 {
                        (*value / *max).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    canvas.rect(
                        RenderRect::new(rect.x, rect.y, rect.width * fraction, rect.height),
                        render_color(theme.fill),
                    );
                    if let Some(layout) =
                        self.layout(ui.resolve_text(label), TextStyle::Small, small)
                    {
                        self.draw_layout(
                            canvas,
                            &layout,
                            rect.x + padding,
                            rect.y + padding,
                            theme.text,
                        );
                    }
                    true
                }
                Widget::List {
                    items,
                    selected,
                    scroll,
                    ..
                } => {
                    canvas.rect(rect, render_color(theme.panel));
                    let row = theme.row_height * scale;
                    for (index, item) in items.iter().enumerate().skip(*scroll).take(8) {
                        let row_rect = RenderRect::new(
                            rect.x,
                            rect.y + (index - *scroll) as f32 * row,
                            rect.width,
                            row,
                        );
                        if index == *selected {
                            canvas.rect(row_rect, render_color(theme.selection));
                        }
                        if let Some(layout) =
                            self.layout(ui.resolve_text(item), TextStyle::Body, font)
                        {
                            let y = rect.y
                                + (index - *scroll) as f32 * row
                                + (row - layout.height()).max(0.0) * 0.5;
                            self.draw_layout(
                                canvas,
                                &layout,
                                rect.x + padding,
                                y,
                                if index == *selected {
                                    theme.accent
                                } else {
                                    theme.text
                                },
                            );
                        }
                    }
                    if node.state.focus_t > 0.01 {
                        focus_border(canvas, rect, theme.focus, theme.focus_width * scale);
                    }
                    true
                }
                Widget::Binding { label, value, .. } => {
                    canvas.rect(rect, render_color(theme.button));
                    if let Some(layout) = self.layout(ui.resolve_text(label), TextStyle::Body, font)
                    {
                        self.draw_layout(
                            canvas,
                            &layout,
                            rect.x + padding,
                            rect.y + padding,
                            theme.text,
                        );
                    }
                    if let Some(layout) = self.layout(value, TextStyle::Body, font) {
                        self.draw_layout(
                            canvas,
                            &layout,
                            rect.x + rect.width - layout.width() - padding,
                            rect.y + padding,
                            theme.accent,
                        );
                    }
                    if node.state.focus_t > 0.01 {
                        focus_border(canvas, rect, theme.focus, theme.focus_width * scale);
                    }
                    true
                }
                _ => false,
            }
        }

        fn texts(
            &self,
            ui: &Ui,
            node: &UiNode,
            theme: &Theme,
            scale: f32,
        ) -> Vec<(String, TextStyle, f32)> {
            let font = theme.font_size * scale;
            let small = theme.small_size * scale;
            match &node.widget {
                Widget::Label { text, style } => vec![(
                    ui.resolve_text(text).to_owned(),
                    *style,
                    theme.size(*style) * scale,
                )],
                Widget::Button { text, .. } => {
                    vec![(ui.resolve_text(text).to_owned(), TextStyle::Body, font)]
                }
                Widget::Slider {
                    label, value, step, ..
                } => vec![
                    (ui.resolve_text(label).to_owned(), TextStyle::Small, small),
                    (format_slider_value(*value, *step), TextStyle::Small, small),
                ],
                Widget::Checkbox { label, .. } => {
                    vec![(ui.resolve_text(label).to_owned(), TextStyle::Body, font)]
                }
                Widget::Select {
                    label,
                    options,
                    selected,
                    ..
                } => {
                    let mut texts =
                        vec![(ui.resolve_text(label).to_owned(), TextStyle::Body, font)];
                    if let Some(option) = options.get(*selected) {
                        texts.push((ui.resolve_text(option).to_owned(), TextStyle::Body, font));
                    }
                    texts
                }
                Widget::Tabs { tabs, .. } => tabs
                    .iter()
                    .map(|tab| (ui.resolve_text(tab).to_owned(), TextStyle::Body, font))
                    .collect(),
                Widget::TextInput {
                    text,
                    placeholder,
                    password,
                    ..
                } => {
                    let mut texts = Vec::new();
                    if text.is_empty() {
                        texts.push((
                            ui.resolve_text(placeholder).to_owned(),
                            TextStyle::Body,
                            font,
                        ));
                    } else {
                        texts.push((
                            if *password {
                                "*".repeat(text.chars().count())
                            } else {
                                text.clone()
                            },
                            TextStyle::Body,
                            font,
                        ));
                    }
                    if !node.preedit.is_empty() {
                        texts.push((node.preedit.clone(), TextStyle::Body, font));
                    }
                    texts
                }
                Widget::Progress { label, .. } => {
                    vec![(ui.resolve_text(label).to_owned(), TextStyle::Small, small)]
                }
                Widget::List { items, .. } => items
                    .iter()
                    .map(|item| (ui.resolve_text(item).to_owned(), TextStyle::Body, font))
                    .collect(),
                Widget::Binding { label, value, .. } => vec![
                    (ui.resolve_text(label).to_owned(), TextStyle::Body, font),
                    (value.clone(), TextStyle::Body, font),
                ],
                _ => Vec::new(),
            }
        }

        fn layout(&self, text: &str, style: TextStyle, size: f32) -> Option<SharedLayout> {
            let format = self.measure.styles().format(style, size);
            self.engine.borrow_mut().layout(text, &format, None).ok()
        }

        fn draw_layout(
            &mut self,
            canvas: &mut Canvas,
            layout: &TextLayout,
            x: f32,
            y: f32,
            color: Color,
        ) {
            let tint = [color.r, color.g, color.b, color.a];
            let mut engine = self.engine.borrow_mut();
            let frame = self.glyphs.frame(&mut engine, layout, tint);
            drop(engine);
            for quad in frame.quads {
                let Some(texture) = self.gpu.texture_id(quad.page) else {
                    continue;
                };
                let mut sprite = Sprite::new(
                    texture,
                    RenderRect::new(
                        x + quad.destination[0],
                        y + quad.destination[1],
                        quad.destination[2],
                        quad.destination[3],
                    ),
                );
                sprite.source = Some(RenderRect::new(
                    quad.source[0],
                    quad.source[1],
                    quad.source[2],
                    quad.source[3],
                ));
                sprite.tint = RenderColor(quad.color);
                sprite.filter = Some(Filter::Linear);
                canvas.sprite(sprite);
            }
        }
    }

    fn to_rect(rect: Rect) -> RenderRect {
        RenderRect::new(rect.x, rect.y, rect.width, rect.height)
    }

    fn render_color(color: Color) -> RenderColor {
        RenderColor([color.r, color.g, color.b, color.a])
    }

    fn focus_border(canvas: &mut Canvas, rect: RenderRect, color: Color, width: f32) {
        let color = render_color(color);
        canvas.rect(RenderRect::new(rect.x, rect.y, rect.width, width), color);
        canvas.rect(
            RenderRect::new(rect.x, rect.y + rect.height - width, rect.width, width),
            color,
        );
        canvas.rect(RenderRect::new(rect.x, rect.y, width, rect.height), color);
        canvas.rect(
            RenderRect::new(rect.x + rect.width - width, rect.y, width, rect.height),
            color,
        );
    }
}
#[cfg(feature = "text-render2d")]
pub use gpu::{TextRenderer, TextRendererOptions};
