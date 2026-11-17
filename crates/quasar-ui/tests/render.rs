#![cfg(feature = "render2d")]
use quasar_frame::Gpu;

use quasar_frame::OffscreenTarget;
use quasar_render_2d::{DisplayOptions, LoadOp, RenderState2D, Textures};
use quasar_ui::{
    ActionId, BitmapTextMeasure, Edges, Layer, LayoutItem, Length, PanelLayout, Rect, Theme, Ui,
    UiRenderer, UiText, Widget,
};

#[test]
#[ignore = "requires a Vulkan-capable graphics device"]
fn ui_renders_into_a_shared_offscreen_frame_and_resources_balance() {
    let mut ui = Ui::new(Theme::modern());
    let root = ui.add_root(
        Widget::Panel {
            layout: PanelLayout::Stack {
                direction: quasar_ui::StackDirection::Vertical,
                gap: 8.0,
            },
            padding: Edges::all(12.0),
            clip: true,
            scrollable: false,
        },
        LayoutItem::fill(),
        Layer::Normal,
        false,
    );
    ui.add_child(
        root,
        Widget::label(UiText::literal("Quasar UI"), quasar_ui::TextStyle::Heading),
        LayoutItem::auto(),
    )
    .unwrap();
    ui.add_child(
        root,
        Widget::Button {
            text: UiText::literal("Start"),
            action: Some(ActionId::new(1)),
        },
        LayoutItem::size(Length::Fill, Length::fixed(32.0)),
    )
    .unwrap();
    ui.layout(
        Rect::new(0.0, 0.0, 480.0, 270.0),
        1.0,
        Edges::ZERO,
        &BitmapTextMeasure,
    );
    let mut canvas = quasar_render_2d::Canvas::new(480, 270).unwrap();
    canvas.clear = quasar_render_2d::Color([8, 10, 16, 255]);
    UiRenderer::new().draw(&mut canvas, &ui);
    let gal = Gpu::headless().unwrap();
    let mut state_2d = RenderState2D::new(&gal).unwrap();
    let target = OffscreenTarget::new(&gal, 480, 270).unwrap();
    let report = target
        .render(&gal, |gal, target| {
            let prepared = state_2d.prepare(
                gal,
                &canvas,
                &Textures::default(),
                target,
                DisplayOptions::default(),
                LoadOp::Clear,
            )?;
            Ok(vec![prepared.commands])
        })
        .unwrap();
    assert_eq!(report.rgba.len(), 480 * 270 * 4);
    let colors: std::collections::BTreeSet<_> =
        report.rgba.as_chunks::<4>().0.iter().copied().collect();
    assert!(colors.len() >= 3);
    target.release(&gal).unwrap();
    state_2d.release(&gal).unwrap();

    drop(state_2d);
    gal.wait().unwrap();
    assert_eq!(
        gal.instance
            .generate_report()
            .unwrap()
            .hub
            .buffers
            .num_kept_from_user,
        0
    );
}

#[cfg(feature = "text-render2d")]
#[test]
#[ignore = "requires a Vulkan-capable graphics device"]
fn shaped_text_renders_through_shared_gal_and_reuses_atlas_pages() {
    use quasar_text::{FontData, FontStack};
    use quasar_ui::{ShapedTextMeasure, TextRenderer, TextRendererOptions, UiTextStyles};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn fonts() -> FontStack {
        let mut fonts = FontStack::new();
        fonts
            .add_font(FontData::new(
                "DejaVuSans.ttf",
                include_bytes!("../../quasar-text/tests/fonts/DejaVuSans.ttf").as_slice(),
            ))
            .add_font(FontData::new(
                "DejaVuSans-Bold.ttf",
                include_bytes!("../../quasar-text/tests/fonts/DejaVuSans-Bold.ttf").as_slice(),
            ))
            .add_font(FontData::new(
                "NotoSansDevanagari-Regular.ttf",
                include_bytes!("../../quasar-text/tests/fonts/NotoSansDevanagari-Regular.ttf")
                    .as_slice(),
            ));
        fonts.with_default_family("DejaVu Sans");
        fonts
    }

    let engine = Rc::new(RefCell::new(fonts().build("en-US").unwrap()));
    let styles = UiTextStyles::new("DejaVu Sans");
    let renderer =
        TextRenderer::new(engine, styles.clone(), TextRendererOptions::default()).unwrap();
    let measure: ShapedTextMeasure = renderer.measure();
    let mut ui = Ui::new(Theme::modern());
    let root = ui.add_root(
        Widget::Panel {
            layout: PanelLayout::Stack {
                direction: quasar_ui::StackDirection::Vertical,
                gap: 8.0,
            },
            padding: Edges::all(12.0),
            clip: true,
            scrollable: false,
        },
        LayoutItem::fill(),
        Layer::Normal,
        false,
    );
    ui.add_child(
        root,
        Widget::label(
            UiText::literal("Shaped text: नमस्ते مرحبا"),
            quasar_ui::TextStyle::Heading,
        ),
        LayoutItem::auto(),
    )
    .unwrap();
    ui.add_child(
        root,
        Widget::TextInput {
            text: "Pilot".into(),
            placeholder: UiText::literal("Callsign"),
            cursor: 5,
            anchor: 5,
            max_len: 32,
            password: false,
            action: None,
        },
        LayoutItem::size(Length::Fill, Length::fixed(32.0)),
    )
    .unwrap();
    let viewport = Rect::new(0.0, 0.0, 480.0, 200.0);
    ui.update(
        &quasar_ui::UiInput::default(),
        viewport,
        2.0,
        Edges::ZERO,
        &measure,
        1.0 / 60.0,
    );
    let mut ui_renderer = UiRenderer::new();
    ui_renderer.enable_text(renderer);
    let gal = Gpu::headless().unwrap();
    let mut state_2d = RenderState2D::new(&gal).unwrap();
    let target = OffscreenTarget::new(&gal, 480, 200).unwrap();
    let mut textures = Textures::default();
    let mut last_uploads = 0;
    let mut last_upload_bytes = 0;
    for frame in 0..2 {
        let report = target
            .render(&gal, |gal, target| {
                ui_renderer.prepare_text(&mut state_2d, gal, &mut textures, &ui)?;
                let mut canvas = quasar_render_2d::Canvas::new(480, 200).unwrap();
                canvas.clear = quasar_render_2d::Color([8, 10, 16, 255]);
                ui_renderer.draw(&mut canvas, &ui);
                let prepared = state_2d.prepare(
                    gal,
                    &canvas,
                    &textures,
                    target,
                    DisplayOptions::default(),
                    LoadOp::Clear,
                )?;
                Ok(vec![prepared.commands])
            })
            .unwrap();
        let colors: std::collections::BTreeSet<_> =
            report.rgba.as_chunks::<4>().0.iter().copied().collect();
        assert!(colors.len() >= 8, "frame {frame} rendered no text");
        let stats = ui_renderer.text().unwrap().gpu().stats();
        let atlas_bytes = ui_renderer.text().unwrap().glyphs().atlas().bytes() as u64;
        if frame == 0 {
            assert!(stats.uploads > 0);
            assert_eq!(stats.upload_bytes, atlas_bytes);
        } else {
            assert_eq!(
                stats.uploads, last_uploads,
                "unchanged atlas page uploaded again"
            );
            assert_eq!(stats.upload_bytes, last_upload_bytes);
        }
        last_uploads = stats.uploads;
        last_upload_bytes = stats.upload_bytes;
    }
    target.release(&gal).unwrap();
    state_2d.release(&gal).unwrap();

    drop(state_2d);
    gal.wait().unwrap();
    assert_eq!(
        gal.instance
            .generate_report()
            .unwrap()
            .hub
            .buffers
            .num_kept_from_user,
        0
    );
}

#[cfg(feature = "text-render2d")]
#[test]
#[ignore = "requires a Vulkan-capable graphics device"]
fn shaped_text_clipping_and_resize_stay_inside_logical_bounds() {
    use quasar_text::{FontData, FontStack};
    use quasar_ui::{ShapedTextMeasure, TextRenderer, TextRendererOptions, UiTextStyles};
    use std::cell::RefCell;
    use std::rc::Rc;

    let mut fonts = FontStack::new();
    fonts
        .add_font(FontData::new(
            "DejaVuSans.ttf",
            include_bytes!("../../quasar-text/tests/fonts/DejaVuSans.ttf").as_slice(),
        ))
        .add_font(FontData::new(
            "DejaVuSans-Bold.ttf",
            include_bytes!("../../quasar-text/tests/fonts/DejaVuSans-Bold.ttf").as_slice(),
        ));
    fonts.with_default_family("DejaVu Sans");
    let engine = Rc::new(RefCell::new(fonts.build("en-US").unwrap()));
    let renderer = TextRenderer::new(
        engine,
        UiTextStyles::new("DejaVu Sans"),
        TextRendererOptions::default(),
    )
    .unwrap();
    let measure: ShapedTextMeasure = renderer.measure();
    let mut ui = Ui::new(Theme::modern());
    let root = ui.add_root(
        Widget::Panel {
            layout: PanelLayout::Stack {
                direction: quasar_ui::StackDirection::Vertical,
                gap: 8.0,
            },
            padding: Edges::all(8.0),
            clip: true,
            scrollable: false,
        },
        LayoutItem::fill(),
        Layer::Normal,
        false,
    );
    let clip = ui
        .add_child(
            root,
            Widget::Panel {
                layout: PanelLayout::Stack {
                    direction: quasar_ui::StackDirection::Vertical,
                    gap: 0.0,
                },
                padding: Edges::all(2.0),
                clip: true,
                scrollable: false,
            },
            LayoutItem::size(Length::fixed(120.0), Length::fixed(48.0)),
        )
        .unwrap();
    ui.add_child(
        clip,
        Widget::label(
            UiText::literal("THIS LONG LABEL MUST BE CLIPPED AT THE PANEL BOUNDARY"),
            quasar_ui::TextStyle::Heading,
        ),
        LayoutItem::auto(),
    )
    .unwrap();
    ui.update(
        &quasar_ui::UiInput::default(),
        Rect::new(0.0, 0.0, 480.0, 200.0),
        1.0,
        Edges::ZERO,
        &measure,
        1.0 / 60.0,
    );
    let mut ui_renderer = UiRenderer::new();
    ui_renderer.enable_text(renderer);
    let gal = Gpu::headless().unwrap();
    let mut state_2d = RenderState2D::new(&gal).unwrap();
    let target = OffscreenTarget::new(&gal, 480, 200).unwrap();
    let mut textures = Textures::default();
    let report = target
        .render(&gal, |gal, target| {
            ui_renderer.prepare_text(&mut state_2d, gal, &mut textures, &ui)?;
            let mut canvas = quasar_render_2d::Canvas::new(480, 200).unwrap();
            canvas.clear = quasar_render_2d::Color([8, 10, 16, 255]);
            ui_renderer.draw(&mut canvas, &ui);
            let prepared = state_2d.prepare(
                gal,
                &canvas,
                &textures,
                target,
                DisplayOptions::default(),
                LoadOp::Clear,
            )?;
            Ok(vec![prepared.commands])
        })
        .unwrap();
    let clip_rect = ui
        .layout_entries()
        .iter()
        .find(|entry| entry.id == clip)
        .unwrap()
        .rect;
    let row = clip_rect.y as usize + clip_rect.height as usize / 2;
    let background = report.rgba[(row * 480 + 479) * 4..(row * 480 + 479) * 4 + 4].to_vec();
    for x in (clip_rect.right() as usize + 2)..479 {
        let pixel = &report.rgba[(row * 480 + x) * 4..(row * 480 + x) * 4 + 4];
        assert_eq!(pixel, background, "text escaped clip at x={x}");
    }

    // Reflow at a different logical size. No new page upload is required.
    ui.update(
        &quasar_ui::UiInput::default(),
        Rect::new(0.0, 0.0, 240.0, 160.0),
        1.0,
        Edges::ZERO,
        &measure,
        1.0 / 60.0,
    );
    let uploads = ui_renderer.text().unwrap().gpu().stats().uploads;
    let second = target
        .render(&gal, |gal, target| {
            ui_renderer.prepare_text(&mut state_2d, gal, &mut textures, &ui)?;
            let mut canvas = quasar_render_2d::Canvas::new(480, 200).unwrap();
            ui_renderer.draw(&mut canvas, &ui);
            let prepared = state_2d.prepare(
                gal,
                &canvas,
                &textures,
                target,
                DisplayOptions::default(),
                LoadOp::Clear,
            )?;
            Ok(vec![prepared.commands])
        })
        .unwrap();
    assert!(second.rgba.iter().any(|byte| *byte != 0));
    assert_eq!(
        ui_renderer.text().unwrap().gpu().stats().uploads,
        uploads,
        "resizing should not re-upload unchanged glyph pages"
    );
    target.release(&gal).unwrap();
    state_2d.release(&gal).unwrap();

    drop(state_2d);
    gal.wait().unwrap();
    assert_eq!(
        gal.instance
            .generate_report()
            .unwrap()
            .hub
            .buffers
            .num_kept_from_user,
        0
    );
}

#[cfg(feature = "text-render2d")]
#[test]
#[ignore = "requires a Vulkan-capable graphics device"]
fn wrapped_labels_draw_every_line_on_the_first_frame() {
    use quasar_text::{FontData, FontStack};
    use quasar_ui::{TextRenderer, TextRendererOptions, UiTextStyles};
    use std::cell::RefCell;
    use std::rc::Rc;

    let mut fonts = FontStack::new();
    fonts.add_font(FontData::new(
        "DejaVuSans.ttf",
        include_bytes!("../../quasar-text/tests/fonts/DejaVuSans.ttf").as_slice(),
    ));
    fonts.with_default_family("DejaVu Sans");
    let engine = Rc::new(RefCell::new(fonts.build("en-US").unwrap()));
    let renderer = TextRenderer::new(
        engine,
        UiTextStyles::new("DejaVu Sans"),
        TextRendererOptions::default(),
    )
    .unwrap();
    let measure = renderer.measure();
    let mut ui = Ui::new(Theme::modern());
    let root = ui.add_root(
        Widget::Panel {
            layout: PanelLayout::Stack {
                direction: quasar_ui::StackDirection::Vertical,
                gap: 0.0,
            },
            padding: Edges::all(4.0),
            clip: false,
            scrollable: false,
        },
        LayoutItem::size(Length::fixed(150.0), Length::Auto),
        Layer::Normal,
        false,
    );
    let label = ui
        .add_child(
            root,
            Widget::wrapped_label(
                UiText::literal("Every wrapped line of this paragraph must be visible at once"),
                quasar_ui::TextStyle::Body,
                None,
            ),
            LayoutItem::auto(),
        )
        .unwrap();
    ui.update(
        &quasar_ui::UiInput::default(),
        Rect::new(0.0, 0.0, 320.0, 240.0),
        1.0,
        Edges::ZERO,
        &measure,
        1.0 / 60.0,
    );
    let rect = ui
        .layout_entries()
        .iter()
        .find(|e| e.id == label)
        .unwrap()
        .rect;
    let mut ui_renderer = UiRenderer::new();
    ui_renderer.enable_text(renderer);
    let gal = Gpu::headless().unwrap();
    let mut state_2d = RenderState2D::new(&gal).unwrap();
    let target = OffscreenTarget::new(&gal, 320, 240).unwrap();
    let mut textures = Textures::default();
    let mut frames = Vec::new();
    for _ in 0..2 {
        let report = target
            .render(&gal, |gal, target| {
                ui_renderer.prepare_text(&mut state_2d, gal, &mut textures, &ui)?;
                let mut canvas = quasar_render_2d::Canvas::new(320, 240).unwrap();
                canvas.clear = quasar_render_2d::Color([0, 0, 0, 255]);
                ui_renderer.draw(&mut canvas, &ui);
                let prepared = state_2d.prepare(
                    gal,
                    &canvas,
                    &textures,
                    target,
                    DisplayOptions::default(),
                    LoadOp::Clear,
                )?;
                Ok(vec![prepared.commands])
            })
            .unwrap();
        frames.push(report.rgba);
    }
    // Text pixels in the bottom line band prove the last wrapped line drew.
    let line = Theme::modern().font_size;
    assert!(rect.height >= line * 3.0, "label wraps to several lines");
    let lit = |rgba: &[u8], y0: f32, y1: f32| {
        (y0 as usize..y1 as usize)
            .flat_map(|y| (rect.x as usize..rect.right() as usize).map(move |x| (x, y)))
            .filter(|&(x, y)| rgba[(y * 320 + x) * 4] > 100)
            .count()
    };
    let bottom = lit(&frames[0], rect.bottom() - line, rect.bottom());
    assert!(
        bottom > 20,
        "last wrapped line missing on frame one ({bottom} px)"
    );
    assert_eq!(
        frames[0], frames[1],
        "first frame differs from the settled one"
    );
    target.release(&gal).unwrap();
    state_2d.release(&gal).unwrap();
}
