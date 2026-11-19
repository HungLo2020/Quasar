//! Auto layout measures fixed-size children, and rich text lays out
//! words, key caps and inline images in wrapped rows.
use quasar_ui::{
    Edges, LabelWrap, Layer, LayoutItem, Length, MonospaceMeasure, PanelLayout, Rect, RichSpan,
    StackDirection, TextStyle, Theme, Ui, UiText, Widget,
};

fn stack() -> Widget {
    Widget::Panel {
        layout: PanelLayout::Stack {
            direction: StackDirection::Vertical,
            gap: 0.0,
        },
        padding: Edges::all(5.0),
        clip: false,
        scrollable: false,
    }
}

fn rect(ui: &Ui, id: quasar_ui::NodeId) -> Rect {
    ui.layout_entries()
        .iter()
        .find(|e| e.id == id)
        .unwrap()
        .rect
}

#[test]
fn auto_panels_measure_fixed_size_children() {
    let mut ui = Ui::new(Theme::modern());
    let root = ui.add_root(stack(), LayoutItem::auto(), Layer::Normal, false);
    // An empty fixed-size holder, a spacer and a fixed-width button.
    ui.add_child(
        root,
        Widget::Panel {
            layout: PanelLayout::Absolute,
            padding: Edges::ZERO,
            clip: false,
            scrollable: false,
        },
        LayoutItem::size(Length::fixed(120.0), Length::fixed(30.0)),
    )
    .unwrap();
    ui.add_child(
        root,
        Widget::Spacer,
        LayoutItem::size(Length::Auto, Length::fixed(16.0)),
    )
    .unwrap();
    let button = ui
        .add_child(
            root,
            Widget::Button {
                text: UiText::literal("Go"),
                action: None,
            },
            LayoutItem::size(Length::fixed(200.0), Length::fixed(44.0)),
        )
        .unwrap();
    ui.layout(
        Rect::new(0.0, 0.0, 800.0, 600.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
    );
    let panel = rect(&ui, root);
    assert_eq!(panel.width, 200.0 + 10.0, "widest fixed child plus padding");
    assert_eq!(panel.height, 30.0 + 16.0 + 44.0 + 10.0);
    assert_eq!(rect(&ui, button).width, 200.0);
    // Scale applies to fixed sizes too.
    ui.layout(
        Rect::new(0.0, 0.0, 800.0, 600.0),
        2.0,
        Edges::ZERO,
        &MonospaceMeasure,
    );
    assert_eq!(rect(&ui, root).width, (200.0 + 10.0) * 2.0);
}

#[test]
fn rich_text_wraps_words_key_caps_and_images() {
    let mut ui = Ui::new(Theme::modern());
    let root = ui.add_root(stack(), LayoutItem::fill(), Layer::Normal, false);
    let spans = vec![
        RichSpan::text("Press "),
        RichSpan::key("F"),
        RichSpan::text(" to open the hatch, or hold "),
        RichSpan::image(7, 24.0, 24.0),
        RichSpan::Text(UiText::keyed("prompt.skip", " to skip")),
    ];
    let single = ui
        .add_child(
            root,
            Widget::RichText {
                spans: spans.clone(),
                style: TextStyle::Body,
                wrap: LabelWrap::None,
                max_width: None,
            },
            LayoutItem::auto(),
        )
        .unwrap();
    let wrapped = ui
        .add_child(
            root,
            Widget::RichText {
                spans,
                style: TextStyle::Body,
                wrap: LabelWrap::Word,
                max_width: Some(160.0),
            },
            LayoutItem::auto(),
        )
        .unwrap();
    ui.layout(
        Rect::new(0.0, 0.0, 1000.0, 600.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
    );
    let (one, many) = (rect(&ui, single), rect(&ui, wrapped));
    let font = ui.theme().font_size;
    assert!(
        one.height >= 24.0 && one.height < font * 2.0 + 1.0,
        "{one:?}"
    );
    assert!(one.width > 300.0, "one row: {one:?}");
    assert!(many.width <= 160.5, "{many:?}");
    assert!(many.height > one.height * 2.0, "wraps into rows: {many:?}");
    assert_eq!(ui.missing_keys(), vec!["prompt.skip".to_string()]);
}
