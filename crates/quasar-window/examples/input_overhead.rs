//! Small local input-adapter overhead probe; not a statistically rigorous
//! benchmark. Run with:
//!
//! ```sh
//! cargo run --release -p quasar-window --example input_overhead
//! ```
use quasar_input::{Binding, Bindings, InputState, Key};
use quasar_window::input::process_key_event;
use std::{
    hint::black_box,
    time::{Duration, Instant},
};
use winit::{
    event::ElementState,
    keyboard::{KeyCode, PhysicalKey},
};

const EVENTS: u64 = 2_000_000;

fn report(label: &str, elapsed: Duration) {
    let per_event = elapsed.as_nanos() as f64 / EVENTS as f64;
    println!(
        "{label:<32} {per_event:>7.2} ns/event  {:.1} Mevent/s",
        EVENTS as f64 / elapsed.as_secs_f64() / 1_000_000.0
    );
}

fn main() {
    let mut input = InputState::default();
    let mut bindings = Bindings::default();
    bindings.bind("fire", Binding::key(Key::KeyW));
    input.set_bindings(bindings).unwrap();
    let key = PhysicalKey::Code(KeyCode::KeyW);

    let start = Instant::now();
    for _ in 0..EVENTS {
        process_key_event(&mut input, key, ElementState::Pressed, false, Some("w"));
        black_box(input.action("fire").pressed);
        process_key_event(&mut input, key, ElementState::Released, false, None);
        black_box(input.action("fire").released);
        input.consume_edges();
    }
    report("press/release/action/tick", start.elapsed());

    let start = Instant::now();
    for _ in 0..EVENTS {
        process_key_event(&mut input, key, ElementState::Pressed, true, Some("w"));
        black_box(input.text.len());
        input.text.clear();
    }
    report("auto-repeat text (no edge)", start.elapsed());
}
