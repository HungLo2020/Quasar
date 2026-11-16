//! Word-wrapped labels, per-node styles, root offsets and anchors.
use quasar_ui::{
    Anchor, Color, Edges, Layer, LayoutItem, Length, MonospaceMeasure, NodeStyle, PanelLayout,
    Rect, StackDirection, TextMeasure, TextStyle, TextWrap, Theme, Ui, UiError, UiText, Widget,
    rounded_border_spans, rounded_rect_spans, wrap_greedy,
};

fn rect_of(ui: &Ui, id: quasar_ui::NodeId) -> Rect {
    ui.layout_entries()
        .iter()
        .find(|entry| entry.id == id)
        .expect("node laid out")
        .rect
}

fn stack(direction: StackDirection) -> Widget {
    Widget::Panel {
        layout: PanelLayout::Stack {
            direction,
            gap: 0.0,
        },
        padding: Edges::ZERO,
        clip: false,
        scrollable: false,
    }
}

fn absolute() -> Widget {
    Widget::Panel {
        layout: PanelLayout::Absolute,
        padding: Edges::ZERO,
        clip: false,
        scrollable: false,
    }
}

const TEXT: &str = "the quick brown fox jumps over the lazy dog";

#[test]
fn greedy_wrap_breaks_words_newlines_and_overlong_words() {
    let width = |part: &str| part.chars().count() as f32;
    assert_eq!(
        wrap_greedy("aa bb cc", 5.0, width),
        vec!["aa bb".to_owned(), "cc".to_owned()]
    );
    assert_eq!(
        wrap_greedy("one\ntwo three", 20.0, width),
        vec!["one".to_owned(), "two three".to_owned()]
    );
    assert_eq!(
        wrap_greedy("abcdefgh", 3.0, width),
        vec!["abc".to_owned(), "def".to_owned(), "gh".to_owned()]
    );
    // Non-finite limits disable wrapping but keep explicit breaks.
    assert_eq!(
        wrap_greedy("a b\nc", f32::INFINITY, width),
        vec!["a b".to_owned(), "c".to_owned()]
    );
    for line in wrap_greedy(TEXT, 9.0, width) {
        assert!(width(&line) <= 9.0, "{line:?}");
    }
}

#[test]
fn wrapped_label_height_grows_with_line_count_during_layout() {
    let mut ui = Ui::new(Theme::retro());
    let root = ui.add_root(
        stack(StackDirection::Vertical),
        LayoutItem::size(Length::fixed(120.0), Length::fixed(400.0)),
        Layer::Hud,
        false,
    );
    let single = ui
        .add_child(
            root,
            Widget::label(UiText::literal(TEXT), TextStyle::Body),
            LayoutItem::auto(),
        )
        .unwrap();
    let wrapped = ui
        .add_child(
            root,
            Widget::wrapped_label(UiText::literal(TEXT), TextStyle::Body, None),
            LayoutItem::auto(),
        )
        .unwrap();
    let limited = ui
        .add_child(
            root,
            Widget::wrapped_label(UiText::literal(TEXT), TextStyle::Body, Some(60.0)),
            LayoutItem::auto(),
        )
        .unwrap();
    ui.layout(
        Rect::new(0.0, 0.0, 640.0, 480.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
    );
    let font = Theme::retro().font_size;
    let single = rect_of(&ui, single);
    let wrapped_rect = rect_of(&ui, wrapped);
    let limited_rect = rect_of(&ui, limited);
    assert!(single.width > 120.0, "unwrapped label keeps one long line");
    assert_eq!(single.height, font);
    assert!(wrapped_rect.width <= 120.0 + 1e-3);
    assert!(wrapped_rect.height >= font * 3.0);
    assert!(limited_rect.width <= 60.0 + 1e-3);
    assert!(limited_rect.height > wrapped_rect.height);
    // The following sibling starts below the wrapped height, not one line.
    assert!(limited_rect.y >= wrapped_rect.y + wrapped_rect.height - 1e-3);
    let lines = MonospaceMeasure.wrap_lines(TEXT, TextStyle::Body, font, 120.0);
    assert_eq!(wrapped_rect.height, font * lines.len() as f32);
}

#[test]
fn wrapped_label_respects_dpi_scale_and_fixed_width() {
    let mut ui = Ui::new(Theme::retro());
    let root = ui.add_root(
        stack(StackDirection::Vertical),
        LayoutItem::fill(),
        Layer::Hud,
        false,
    );
    let label = ui
        .add_child(
            root,
            Widget::wrapped_label(UiText::literal(TEXT), TextStyle::Body, None),
            LayoutItem::size(Length::fixed(100.0), Length::Auto),
        )
        .unwrap();
    ui.layout(
        Rect::new(0.0, 0.0, 1000.0, 1000.0),
        2.0,
        Edges::ZERO,
        &MonospaceMeasure,
    );
    let rect = rect_of(&ui, label);
    assert_eq!(rect.width, 200.0);
    let font = Theme::retro().font_size * 2.0;
    let lines = MonospaceMeasure.wrap_lines(TEXT, TextStyle::Body, font, 200.0);
    assert!(lines.len() > 1);
    assert_eq!(rect.height, font * lines.len() as f32);
}

#[test]
fn wrapped_labels_in_horizontal_stacks_wrap_to_width_not_height() {
    let mut ui = Ui::new(Theme::retro());
    let root = ui.add_root(
        stack(StackDirection::Horizontal),
        LayoutItem::size(Length::fixed(300.0), Length::fixed(30.0)),
        Layer::Hud,
        false,
    );
    let label = ui
        .add_child(
            root,
            Widget::wrapped_label(UiText::literal(TEXT), TextStyle::Body, None),
            LayoutItem::auto(),
        )
        .unwrap();
    ui.layout(
        Rect::new(0.0, 0.0, 640.0, 480.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
    );
    // Wrapping at the 30-unit height would make a very tall, narrow label.
    assert!(rect_of(&ui, label).width > 200.0);
}

#[test]
fn roots_honour_offsets_margins_and_anchors() {
    let viewport = Rect::new(0.0, 0.0, 400.0, 300.0);
    let mut ui = Ui::new(Theme::retro());
    let plain = ui.add_root(
        absolute(),
        LayoutItem::size(Length::fixed(40.0), Length::fixed(20.0)),
        Layer::Hud,
        false,
    );
    let offset = ui.add_root(
        absolute(),
        LayoutItem::size(Length::fixed(40.0), Length::fixed(20.0))
            .with_offset(quasar_ui::Vec2::new(10.0, 5.0)),
        Layer::Hud,
        false,
    );
    let corner = ui.add_root(
        absolute(),
        LayoutItem::size(Length::fixed(40.0), Length::fixed(20.0))
            .with_margin(Edges::all(16.0))
            .with_anchor(Anchor::BottomRight),
        Layer::Hud,
        false,
    );
    let bottom = ui.add_root(
        absolute(),
        LayoutItem::size(Length::fixed(100.0), Length::fixed(20.0)).with_anchor(Anchor::Bottom),
        Layer::Hud,
        false,
    );
    let stretched = ui.add_root(
        absolute(),
        LayoutItem::auto()
            .with_margin(Edges::all(8.0))
            .with_anchor(Anchor::Stretch),
        Layer::Hud,
        false,
    );
    ui.layout(viewport, 2.0, Edges::ZERO, &MonospaceMeasure);
    assert_eq!(rect_of(&ui, plain), Rect::new(0.0, 0.0, 80.0, 40.0));
    assert_eq!(rect_of(&ui, offset), Rect::new(20.0, 10.0, 80.0, 40.0));
    assert_eq!(
        rect_of(&ui, corner),
        Rect::new(400.0 - 32.0 - 80.0, 300.0 - 32.0 - 40.0, 80.0, 40.0)
    );
    assert_eq!(
        rect_of(&ui, bottom),
        Rect::new(100.0, 300.0 - 40.0, 200.0, 40.0)
    );
    assert_eq!(
        rect_of(&ui, stretched),
        Rect::new(16.0, 16.0, 400.0 - 32.0, 300.0 - 32.0)
    );
}

#[test]
fn absolute_children_use_all_nine_anchors() {
    let mut ui = Ui::new(Theme::retro());
    let root = ui.add_root(absolute(), LayoutItem::fill(), Layer::Hud, false);
    let anchors = [
        (Anchor::TopLeft, 0.0, 0.0),
        (Anchor::Top, 0.5, 0.0),
        (Anchor::TopRight, 1.0, 0.0),
        (Anchor::Left, 0.0, 0.5),
        (Anchor::Center, 0.5, 0.5),
        (Anchor::Right, 1.0, 0.5),
        (Anchor::BottomLeft, 0.0, 1.0),
        (Anchor::Bottom, 0.5, 1.0),
        (Anchor::BottomRight, 1.0, 1.0),
    ];
    let ids: Vec<_> = anchors
        .iter()
        .map(|(anchor, _, _)| {
            ui.add_child(
                root,
                Widget::Spacer,
                LayoutItem::size(Length::fixed(20.0), Length::fixed(10.0))
                    .with_anchor(*anchor)
                    .with_offset(quasar_ui::Vec2::new(1.0, 2.0)),
            )
            .unwrap()
        })
        .collect();
    ui.layout(
        Rect::new(0.0, 0.0, 200.0, 100.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
    );
    for (id, (_, fx, fy)) in ids.iter().zip(anchors) {
        let rect = rect_of(&ui, *id);
        assert_eq!(rect.x, (200.0 - 20.0) * fx + 1.0);
        assert_eq!(rect.y, (100.0 - 10.0) * fy + 2.0);
        assert_eq!((rect.width, rect.height), (20.0, 10.0));
    }
}

#[test]
fn node_styles_validate_and_round_trip() {
    let mut ui = Ui::new(Theme::retro());
    let root = ui.add_root(absolute(), LayoutItem::fill(), Layer::Hud, false);
    let style = NodeStyle::filled(Color::rgba(10, 20, 30, 200))
        .with_border(2.0, Color::WHITE)
        .with_radius(6.0);
    ui.set_style(root, Some(style)).unwrap();
    assert_eq!(ui.node(root).unwrap().style, Some(style));
    assert_eq!(
        ui.set_style(root, Some(NodeStyle::transparent().with_radius(f32::NAN))),
        Err(UiError::InvalidValue)
    );
    assert_eq!(
        ui.set_style(
            root,
            Some(NodeStyle::transparent().with_border(-1.0, Color::WHITE))
        ),
        Err(UiError::InvalidValue)
    );
    ui.set_style(root, None).unwrap();
    assert_eq!(ui.node(root).unwrap().style, None);
    let mut item = LayoutItem::anchored(Anchor::Center);
    item.width = Length::fixed(5.0);
    ui.set_layout_item(root, item).unwrap();
    assert_eq!(ui.node(root).unwrap().item.anchor, Some(Anchor::Center));
}

fn area(spans: &[Rect]) -> f32 {
    spans.iter().map(|span| span.width * span.height).sum()
}

#[test]
fn rounded_spans_cover_the_rounded_shape_without_overlap() {
    let rect = Rect::new(10.0, 20.0, 100.0, 60.0);
    assert_eq!(rounded_rect_spans(rect, 0.0), vec![rect]);
    let spans = rounded_rect_spans(rect, 12.0);
    let expected = 100.0 * 60.0 - (4.0 - std::f32::consts::PI) * 12.0 * 12.0;
    assert!((area(&spans) - expected).abs() < expected * 0.01);
    for span in &spans {
        assert!(span.x >= rect.x - 1e-3 && span.right() <= rect.right() + 1e-3);
        assert!(span.y >= rect.y - 1e-3 && span.bottom() <= rect.bottom() + 1e-3);
    }
    // Rows are disjoint vertically.
    let mut rows: Vec<_> = spans.iter().map(|span| (span.y, span.bottom())).collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    for pair in rows.windows(2) {
        assert!(pair[0].1 <= pair[1].0 + 1e-3);
    }
    // Square border: four strips with the frame's area.
    let square = rounded_border_spans(rect, 0.0, 2.0);
    assert!((area(&square) - (100.0 * 60.0 - 96.0 * 56.0)).abs() < 1e-3);
    // Rounded border never covers the inner area.
    let border = rounded_border_spans(rect, 12.0, 2.0);
    let inner = Rect::new(rect.x + 14.0, rect.y + 14.0, 72.0, 32.0);
    for span in &border {
        let overlap_x = span.right().min(inner.right()) - span.x.max(inner.x);
        let overlap_y = span.bottom().min(inner.bottom()) - span.y.max(inner.y);
        assert!(overlap_x <= 1e-3 || overlap_y <= 1e-3, "{span:?}");
    }
    assert!(area(&border) < area(&spans));
}

#[test]
fn text_wrap_defaults_to_none() {
    let Widget::Label {
        wrap, max_width, ..
    } = Widget::label(UiText::literal("x"), TextStyle::Body)
    else {
        unreachable!()
    };
    assert_eq!((wrap, max_width), (TextWrap::None, None));
}

#[cfg(feature = "render2d")]
mod painting {
    use super::*;
    use quasar_ui::UiRenderer;

    fn painted(style: Option<NodeStyle>) -> Vec<quasar_render_2d::Quad> {
        let mut ui = Ui::new(Theme::retro());
        let root = ui.add_root(absolute(), LayoutItem::fill(), Layer::Hud, false);
        ui.set_style(root, style).unwrap();
        ui.layout(
            Rect::new(0.0, 0.0, 64.0, 48.0),
            1.0,
            Edges::ZERO,
            &MonospaceMeasure,
        );
        let mut canvas = quasar_render_2d::Canvas::new(64, 48).unwrap();
        UiRenderer::default().draw(&mut canvas, &ui);
        canvas.quads().to_vec()
    }

    #[test]
    fn transparent_style_suppresses_the_theme_panel() {
        assert_eq!(painted(None).len(), 1, "theme paints one panel quad");
        assert!(painted(Some(NodeStyle::transparent())).is_empty());
        let filled = painted(Some(NodeStyle::filled(Color::rgba(1, 2, 3, 255))));
        assert_eq!(filled.len(), 1);
        let rounded = painted(Some(
            NodeStyle::filled(Color::rgba(1, 2, 3, 255))
                .with_radius(8.0)
                .with_border(1.0, Color::WHITE),
        ));
        assert!(rounded.len() > 10);
    }

    #[test]
    fn wrapped_bitmap_labels_draw_multiple_lines_inside_their_rect() {
        let mut ui = Ui::new(Theme::retro());
        let root = ui.add_root(
            stack(StackDirection::Vertical),
            LayoutItem::size(Length::fixed(160.0), Length::fixed(300.0)),
            Layer::Hud,
            false,
        );
        ui.set_style(root, Some(NodeStyle::transparent())).unwrap();
        let label = ui
            .add_child(
                root,
                Widget::wrapped_label(UiText::literal(TEXT), TextStyle::Body, None),
                LayoutItem::auto(),
            )
            .unwrap();
        let measure = quasar_ui::BitmapTextMeasure;
        ui.layout(
            Rect::new(0.0, 0.0, 320.0, 320.0),
            1.0,
            Edges::ZERO,
            &measure,
        );
        let rect = rect_of(&ui, label);
        assert!(rect.width <= 160.0 + 1e-3);
        let mut canvas = quasar_render_2d::Canvas::new(320, 320).unwrap();
        UiRenderer::default().draw(&mut canvas, &ui);
        let quads = canvas.quads();
        assert!(!quads.is_empty());
        let mut rows = std::collections::BTreeSet::new();
        for quad in quads {
            let ([x, y], w, h) = (quad.origin, quad.x[0], quad.y[1]);
            assert!(x >= rect.x - 1e-3 && x + w <= rect.right() + 1.0, "{x} {w}");
            assert!(y >= rect.y - 1e-3 && y + h <= rect.bottom() + 1.0);
            rows.insert((y / Theme::retro().font_size) as i32);
        }
        assert!(rows.len() >= 3, "text spans several lines: {rows:?}");
    }
}
