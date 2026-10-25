//! Local shaping, layout, rasterization and atlas probe.
//!
//! ```sh
//! cargo run --release -p quasar-text --example text_benchmark
//! ```
use quasar_text::{AtlasConfig, FontData, FontStack, GlyphCache, TextAlign, TextStyle, TextWrap};
use std::hint::black_box;
use std::time::Instant;

const DEJAVU: &[u8] = include_bytes!("../tests/fonts/DejaVuSans.ttf");
const DEJAVU_BOLD: &[u8] = include_bytes!("../tests/fonts/DejaVuSans-Bold.ttf");
const DEVANAGARI: &[u8] = include_bytes!("../tests/fonts/NotoSansDevanagari-Regular.ttf");

fn main() {
    let mut stack = FontStack::new();
    stack
        .add_font(FontData::new("DejaVuSans.ttf", DEJAVU))
        .add_font(FontData::new("DejaVuSans-Bold.ttf", DEJAVU_BOLD))
        .add_font(FontData::new("NotoSansDevanagari-Regular.ttf", DEVANAGARI));
    stack.with_default_family("DejaVu Sans");
    let mut engine = stack.build("en-US").unwrap();
    let body = TextStyle::new(18.0)
        .with_family("DejaVu Sans")
        .with_wrap(TextWrap::Word)
        .with_align(TextAlign::Start);
    let text =
        "Quasar text layout: office مرحبا Ελληνικά नमस्ते\nSecond line with kerning and wrapping.";
    let mut layout = engine.layout(text, &body, Some(420.0)).unwrap();

    let start = Instant::now();
    for _ in 0..5_000 {
        layout = engine.layout(text, &body, Some(420.0)).unwrap();
        black_box(layout.width());
    }
    let layout_elapsed = start.elapsed();
    let stats = engine.layout_stats();
    let start = Instant::now();
    for index in 0..500 {
        let cold = format!("Cold layout {index}: office مرحبا Ελληνικά नमस्ते");
        let layout = engine.layout(&cold, &body, Some(420.0)).unwrap();
        black_box(layout.width());
    }
    let cold_elapsed = start.elapsed();

    let mut cache = GlyphCache::new(AtlasConfig::default()).unwrap();
    let start = Instant::now();
    let first = cache.frame(&mut engine, &layout, [255; 4]);
    let first_elapsed = start.elapsed();
    let first_rasterized = first.stats.rasterized;
    for page in first.pages_dirty {
        cache.atlas_mut().mark_clean(page);
    }
    let start = Instant::now();
    for _ in 0..2_000 {
        let frame = cache.frame(&mut engine, &layout, [255; 4]);
        black_box(frame.quads.len());
    }
    let repeat_elapsed = start.elapsed();
    let repeat = cache.frame(&mut engine, &layout, [255; 4]);

    println!(
        "layout calls={} cached_us/call={:.3} cold_us/call={:.3} cache_hits={} entries={}",
        stats.calls,
        layout_elapsed.as_secs_f64() * 1_000_000.0 / 5_000.0,
        cold_elapsed.as_secs_f64() * 1_000_000.0 / 500.0,
        stats.cache_hits,
        stats.cache_entries
    );
    println!(
        "first-frame glyphs={} quads={} us={:.3} rasterized={} pages={} bytes={}",
        first.stats.glyphs,
        first.stats.quads,
        first_elapsed.as_secs_f64() * 1_000_000.0,
        first_rasterized,
        cache.atlas().pages().len(),
        cache.atlas().bytes()
    );
    println!(
        "repeat-frame quads={} us/frame={:.3} rasterized={} cache_hits={}",
        repeat.stats.quads,
        repeat_elapsed.as_secs_f64() * 1_000_000.0 / 2_000.0,
        repeat.stats.rasterized,
        repeat.stats.cache_hits
    );
    println!(
        "atlas pages={} bytes={} readback_rows={}",
        cache.atlas().pages().len(),
        cache.atlas().bytes(),
        cache
            .atlas()
            .pages()
            .iter()
            .map(|page| page.height)
            .sum::<u32>()
    );
}
