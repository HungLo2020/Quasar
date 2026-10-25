#![cfg(feature = "text")]

use quasar_text::{FontData, FontStack};
use quasar_ui::{
    Edges, Layer, LayoutItem, ShapedTextMeasure, TextMeasure, TextStyle, Ui, UiInput, UiText,
    UiTextStyles, Widget,
};
use std::cell::RefCell;
use std::rc::Rc;

const DEJAVU: &[u8] = include_bytes!("../../quasar-text/tests/fonts/DejaVuSans.ttf");
const DEJAVU_BOLD: &[u8] = include_bytes!("../../quasar-text/tests/fonts/DejaVuSans-Bold.ttf");
const DEVANAGARI: &[u8] =
    include_bytes!("../../quasar-text/tests/fonts/NotoSansDevanagari-Regular.ttf");

fn measure() -> ShapedTextMeasure {
    let mut fonts = FontStack::new();
    fonts
        .add_font(FontData::new("DejaVuSans.ttf", DEJAVU))
        .add_font(FontData::new("DejaVuSans-Bold.ttf", DEJAVU_BOLD))
        .add_font(FontData::new("NotoSansDevanagari-Regular.ttf", DEVANAGARI));
    fonts.with_default_family("DejaVu Sans");
    let engine = Rc::new(RefCell::new(fonts.build("en-US").unwrap()));
    ShapedTextMeasure::new(engine, UiTextStyles::new("DejaVu Sans"))
}

#[test]
fn shaped_measurement_uses_styles_fallback_and_multiline_metrics() {
    let measure = measure();
    let body = measure.measure_style("Hello नमस्ते", TextStyle::Body, 18.0);
    let heading = measure.measure_style("Hello नमस्ते", TextStyle::Heading, 18.0);
    assert!(body.x > 0.0);
    assert!(body.y >= 18.0);
    assert!(heading.x > body.x, "heading should use the bold face");

    let multiline = measure.measure_style("one\ntwo\nthree", TextStyle::Body, 18.0);
    assert!(multiline.y >= body.y * 2.5);

    let x = measure
        .caret_x("Hello world", TextStyle::Body, 18.0, 5)
        .unwrap();
    let hit = measure
        .caret_index("Hello world", TextStyle::Body, 18.0, x)
        .unwrap();
    assert_eq!(hit, 5);
    assert_eq!(
        measure.next_boundary("a\u{0301}b", 0),
        2,
        "combining mark should stay in one editing cluster"
    );
    assert_eq!(measure.previous_boundary("a\u{0301}b", 2), 0);
}

#[test]
fn text_input_editing_caret_hit_selection_and_scroll_share_layout() {
    let measure = measure();
    let mut ui = Ui::new(quasar_ui::Theme::modern());
    let root = ui.add_root(
        Widget::Panel {
            layout: quasar_ui::PanelLayout::Stack {
                direction: quasar_ui::StackDirection::Vertical,
                gap: 4.0,
            },
            padding: Edges::all(8.0),
            clip: true,
            scrollable: false,
        },
        LayoutItem::fill(),
        Layer::Normal,
        false,
    );
    let input = ui
        .add_child(
            root,
            Widget::TextInput {
                text: String::new(),
                placeholder: UiText::literal("Name"),
                cursor: 0,
                anchor: 0,
                max_len: 64,
                password: false,
                action: None,
            },
            LayoutItem::size(quasar_ui::Length::Fill, quasar_ui::Length::fixed(32.0)),
        )
        .unwrap();
    let viewport = quasar_ui::Rect::new(0.0, 0.0, 320.0, 120.0);
    let update = |ui: &mut Ui, input: UiInput| {
        ui.update(&input, viewport, 1.0, Edges::ZERO, &measure, 1.0 / 60.0)
    };

    update(&mut ui, UiInput::default());
    update(
        &mut ui,
        UiInput {
            text: "hello नमस्ते".into(),
            ..UiInput::default()
        },
    );
    let text = match &ui.node(input).unwrap().widget {
        Widget::TextInput { text, .. } => text.clone(),
        _ => unreachable!(),
    };
    assert_eq!(text, "hello नमस्ते");

    // Shift + Left creates a visual selection without moving the anchor.
    update(
        &mut ui,
        UiInput {
            left: true,
            shift: true,
            ..UiInput::default()
        },
    );
    let (cursor, anchor) = match &ui.node(input).unwrap().widget {
        Widget::TextInput { cursor, anchor, .. } => (*cursor, *anchor),
        _ => unreachable!(),
    };
    assert_eq!(anchor, 12);
    assert!(cursor < anchor, "selection should end before the anchor");

    // Move the caret left and delete one grapheme.
    update(
        &mut ui,
        UiInput {
            left: true,
            ..UiInput::default()
        },
    );
    update(
        &mut ui,
        UiInput {
            backspace: true,
            ..UiInput::default()
        },
    );
    let text = match &ui.node(input).unwrap().widget {
        Widget::TextInput { text, .. } => text.clone(),
        _ => unreachable!(),
    };
    assert!(text.starts_with("hello "));

    // Select all and replace it through the committed-text input channel.
    update(
        &mut ui,
        UiInput {
            select_all: true,
            text: "مرحبا".into(),
            ..UiInput::default()
        },
    );
    let (text, cursor, anchor) = match &ui.node(input).unwrap().widget {
        Widget::TextInput {
            text,
            cursor,
            anchor,
            ..
        } => (text.clone(), *cursor, *anchor),
        _ => unreachable!(),
    };
    assert_eq!(text, "مرحبا");
    assert_eq!(cursor, 5);
    assert_eq!(anchor, 5);

    // Long text scrolls to keep the caret visible.
    update(
        &mut ui,
        UiInput {
            select_all: true,
            text: "a very long pilot name that cannot fit in the field".into(),
            ..UiInput::default()
        },
    );
    assert!(ui.node(input).unwrap().text_scroll > 0.0);

    // Pointer hit-testing uses the same shaped layout and scroll offset.
    let entry = ui
        .layout_entries()
        .iter()
        .find(|entry| entry.id == input)
        .unwrap()
        .rect;
    let pointer = quasar_ui::Vec2::new(entry.x + entry.width * 0.5, entry.y + entry.height * 0.5);
    update(
        &mut ui,
        UiInput {
            pointer: Some(pointer),
            primary_pressed: true,
            primary_down: true,
            ..UiInput::default()
        },
    );
    let hit = match &ui.node(input).unwrap().widget {
        Widget::TextInput { cursor, .. } => *cursor,
        _ => unreachable!(),
    };
    let shaped = measure
        .layout(
            "a very long pilot name that cannot fit in the field",
            TextStyle::Body,
            ui.theme().font_size,
            None,
        )
        .unwrap();
    let expected = shaped.hit(
        pointer.x - (entry.x + ui.theme().padding) + ui.node(input).unwrap().text_scroll,
        1.0,
    );
    assert!(
        hit.abs_diff(expected) <= 1,
        "widget cursor {hit} should match shaped hit {expected}"
    );
}

#[test]
fn ime_preedit_is_rendered_without_mutating_committed_text() {
    let measure = measure();
    let mut ui = Ui::new(quasar_ui::Theme::retro());
    let root = ui.add_root(
        Widget::Panel {
            layout: quasar_ui::PanelLayout::Stack {
                direction: quasar_ui::StackDirection::Vertical,
                gap: 4.0,
            },
            padding: Edges::all(4.0),
            clip: true,
            scrollable: false,
        },
        LayoutItem::fill(),
        Layer::Normal,
        false,
    );
    let input = ui
        .add_child(
            root,
            Widget::TextInput {
                text: "base".into(),
                placeholder: UiText::literal(""),
                cursor: 4,
                anchor: 4,
                max_len: 32,
                password: false,
                action: None,
            },
            LayoutItem::size(quasar_ui::Length::Fill, quasar_ui::Length::fixed(28.0)),
        )
        .unwrap();
    ui.update(
        &UiInput::default(),
        quasar_ui::Rect::new(0.0, 0.0, 240.0, 80.0),
        1.0,
        Edges::ZERO,
        &measure,
        1.0 / 60.0,
    );
    ui.update(
        &UiInput {
            preedit: "かな".into(),
            preedit_cursor: 1,
            ..UiInput::default()
        },
        quasar_ui::Rect::new(0.0, 0.0, 240.0, 80.0),
        1.0,
        Edges::ZERO,
        &measure,
        1.0 / 60.0,
    );
    let node = ui.node(input).unwrap();
    assert_eq!(node.preedit, "かな");
    assert_eq!(node.preedit_cursor, 1);
    match &node.widget {
        Widget::TextInput { text, .. } => assert_eq!(text, "base"),
        _ => unreachable!(),
    }
}
