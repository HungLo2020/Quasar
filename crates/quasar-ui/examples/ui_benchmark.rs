//! Local UI layout/interaction probe.
//!
//! ```sh
//! cargo run --release -p quasar-ui --example ui_benchmark
//! ```
use quasar_ui::{
    ActionId, Edges, Layer, LayoutItem, Length, MonospaceMeasure, PanelLayout, Rect,
    StackDirection, Theme, Ui, UiInput, UiText, Widget,
};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

fn percentile(samples: &mut [Duration], fraction: f64) -> Duration {
    samples.sort_unstable();
    samples[((samples.len() - 1) as f64 * fraction).round() as usize]
}

fn main() {
    let mut ui = Ui::new(Theme::modern());
    let root = ui.add_root(
        Widget::Panel {
            layout: PanelLayout::Stack {
                direction: StackDirection::Vertical,
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
    for index in 0..128u64 {
        ui.add_child(
            root,
            if index.is_multiple_of(4) {
                Widget::Button {
                    text: UiText::literal(format!("Button {index}")),
                    action: Some(ActionId::new(index + 1)),
                }
            } else {
                Widget::Label {
                    text: UiText::literal(format!("Label {index}")),
                    style: quasar_ui::TextStyle::Body,
                }
            },
            LayoutItem::size(Length::Fill, Length::fixed(22.0)),
        )
        .unwrap();
    }
    let viewport = Rect::new(0.0, 0.0, 1280.0, 720.0);
    let start = Instant::now();
    let mut events = 0usize;
    for frame in 0..10_000u64 {
        let input = UiInput {
            pointer: Some(quasar_ui::Vec2::new(
                640.0 + (frame as f32 * 0.01).sin() * 200.0,
                360.0 + (frame as f32 * 0.013).cos() * 150.0,
            )),
            primary_pressed: frame.is_multiple_of(11),
            primary_down: frame % 11 < 3,
            ..UiInput::default()
        };
        let output = ui.update(
            &input,
            viewport,
            1.0,
            Edges::ZERO,
            &MonospaceMeasure,
            1.0 / 60.0,
        );
        events += output.events.len();
        black_box(&output);
    }
    let elapsed = start.elapsed();
    let mut samples = Vec::new();
    for _ in 0..1_000 {
        let started = Instant::now();
        ui.layout(viewport, 1.0, Edges::ZERO, &MonospaceMeasure);
        samples.push(started.elapsed());
    }
    println!(
        "nodes=129 updates=10000 total_ms={:.3} per_update_us={:.3} events={events} layout_p50_us={:.3} layout_p99_us={:.3}",
        elapsed.as_secs_f64() * 1_000.0,
        elapsed.as_secs_f64() * 1_000_000.0 / 10_000.0,
        percentile(&mut samples, 0.50).as_secs_f64() * 1_000_000.0,
        percentile(&mut samples, 0.99).as_secs_f64() * 1_000_000.0,
    );
}
