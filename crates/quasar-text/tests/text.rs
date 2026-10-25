use quasar_text::{
    AtlasConfig, FontData, FontStack, GlyphCache, TextAlign, TextEngine, TextStyle, TextWrap,
};

const DEJAVU: &[u8] = include_bytes!("fonts/DejaVuSans.ttf");
const DEJAVU_BOLD: &[u8] = include_bytes!("fonts/DejaVuSans-Bold.ttf");
const DEVANAGARI: &[u8] = include_bytes!("fonts/NotoSansDevanagari-Regular.ttf");
const NIMBUS: &[u8] = include_bytes!("fonts/NimbusSans-Regular.otf");

fn engine() -> TextEngine {
    let mut stack = FontStack::new();
    stack
        .add_font(FontData::new("DejaVuSans.ttf", DEJAVU).with_license("Bitstream Vera"))
        .add_font(FontData::new("DejaVuSans-Bold.ttf", DEJAVU_BOLD).with_license("Bitstream Vera"))
        .add_font(
            FontData::new("NotoSansDevanagari-Regular.ttf", DEVANAGARI).with_license("OFL-1.1"),
        );
    stack.with_default_family("DejaVu Sans");
    stack.build("en-US").unwrap()
}

#[test]
fn loads_ttf_and_otf_and_uses_fallback_for_devanagari() {
    let mut engine = engine();
    let layout = engine
        .layout(
            "Hello नमस्ते مرحبا Ελληνικά",
            &TextStyle::new(20.0).with_family("DejaVu Sans"),
            None,
        )
        .unwrap();
    assert!(layout.width() > 0.0);
    assert!(!layout.glyphs().is_empty());
    let fonts: std::collections::BTreeSet<_> =
        layout.glyphs().iter().map(|glyph| glyph.font).collect();
    assert!(fonts.len() >= 2, "fallback face should be used: {fonts:?}");
    assert!(layout.glyphs().iter().any(|glyph| glyph.rtl));

    let mut otf = FontStack::new();
    otf.add_font(FontData::new("NimbusSans-Regular.otf", NIMBUS));
    let mut otf = otf.build("en-US").unwrap();
    let layout = otf
        .layout(
            "OpenType",
            &TextStyle::new(18.0).with_family("Nimbus Sans"),
            None,
        )
        .unwrap();
    assert!(!layout.glyphs().is_empty());
}

#[test]
fn weight_and_family_select_the_bold_face() {
    let mut engine = engine();
    let regular = engine
        .layout(
            "Weight",
            &TextStyle::new(18.0).with_family("DejaVu Sans"),
            None,
        )
        .unwrap();
    let bold = engine
        .layout(
            "Weight",
            &TextStyle::new(18.0)
                .with_family("DejaVu Sans")
                .with_weight(700),
            None,
        )
        .unwrap();
    assert!(
        regular
            .glyphs()
            .iter()
            .all(|glyph| glyph.font_weight == 400)
    );
    assert!(bold.glyphs().iter().all(|glyph| glyph.font_weight == 700));
    assert_ne!(regular.glyphs()[0].font, bold.glyphs()[0].font);
    assert!(bold.width() > regular.width());
}

#[test]
fn wrapping_alignment_and_multiline_metrics_are_consistent() {
    let mut engine = engine();
    let style = TextStyle::new(16.0)
        .with_family("DejaVu Sans")
        .with_wrap(TextWrap::Word)
        .with_align(TextAlign::Center);
    let layout = engine
        .layout(
            "The quick brown fox jumps over the lazy dog\nSecond paragraph",
            &style,
            Some(160.0),
        )
        .unwrap();
    assert!(layout.lines().len() >= 3);
    assert!(layout.lines().iter().all(|line| line.width <= 160.5));
    assert!(layout.height() >= style.line_height * layout.lines().len() as f32);
    assert!(layout.glyphs().iter().any(|glyph| glyph.line_top > 0.0));
    assert!(
        layout
            .lines()
            .iter()
            .filter(|line| line.width < 160.0)
            .any(|line| line.char_range.start > 0 && line.width > 0.0)
    );
}

#[test]
fn caret_hit_selection_and_graphemes_use_the_same_layout() {
    let mut engine = engine();
    let text = "abc مرحبا def";
    let layout = engine
        .layout(text, &TextStyle::new(18.0).with_family("DejaVu Sans"), None)
        .unwrap();
    let start = layout.caret(0);
    let end = layout.caret(layout.char_count());
    assert_eq!(start.x, 0.0);
    assert!(end.x > start.x);
    assert_eq!(layout.hit(start.x, start.top + 2.0), 0);
    let middle = layout.char_count() / 2;
    let caret = layout.caret(middle);
    let hit = layout.hit(caret.x + 0.1, caret.top + 2.0);
    assert!(
        hit.abs_diff(middle) <= 1,
        "caret {middle} hit {hit} at {}",
        caret.x
    );
    let rects = layout.selection_rects(0, layout.char_count());
    assert!(!rects.is_empty());
    assert!(
        rects
            .iter()
            .all(|rect| rect.width > 0.0 && rect.height > 0.0)
    );

    let composed = engine
        .layout(
            "a\u{0301}b",
            &TextStyle::new(18.0).with_family("DejaVu Sans"),
            None,
        )
        .unwrap();
    assert_eq!(composed.char_count(), 3);
    assert_eq!(composed.next_boundary(0), 2);
    assert_eq!(composed.previous_boundary(2), 0);
}

#[test]
fn raster_and_atlas_reuse_unchanged_glyphs() {
    let mut engine = engine();
    let layout = engine
        .layout(
            "Cache me",
            &TextStyle::new(20.0).with_family("DejaVu Sans"),
            None,
        )
        .unwrap();
    let mut cache = GlyphCache::new(AtlasConfig {
        size: 256,
        padding: 1,
        max_pages: 2,
    })
    .unwrap();
    let first = cache.frame(&mut engine, &layout, [255, 255, 255, 255]);
    assert!(!first.quads.is_empty());
    assert!(!first.pages_dirty.is_empty());
    let rasterized = first.stats.rasterized;
    assert!(rasterized > 0);
    assert!(
        cache
            .atlas()
            .page_rgba(0)
            .unwrap()
            .iter()
            .any(|byte| *byte != 0)
    );

    for page in &first.pages_dirty {
        cache.atlas_mut().mark_clean(*page);
    }
    let second = cache.frame(&mut engine, &layout, [255, 255, 255, 255]);
    assert_eq!(second.quads, first.quads);
    assert!(second.pages_dirty.is_empty());
    assert_eq!(second.stats.rasterized, rasterized);
    assert!(second.stats.cache_hits > first.stats.cache_hits);
}

#[test]
fn dpi_scale_changes_raster_size_and_layout() {
    let mut engine = engine();
    let small = engine
        .layout(
            "Scale",
            &TextStyle::new(16.0).with_family("DejaVu Sans"),
            None,
        )
        .unwrap();
    let large = engine
        .layout(
            "Scale",
            &TextStyle::new(32.0).with_family("DejaVu Sans"),
            None,
        )
        .unwrap();
    assert!(large.width() > small.width() * 1.8);
    assert!(large.height() > small.height() * 1.8);
    let mut cache = GlyphCache::new(AtlasConfig::default()).unwrap();
    let small_frame = cache.frame(&mut engine, &small, [255; 4]);
    let large_frame = cache.frame(&mut engine, &large, [255; 4]);
    let small_height = small_frame
        .quads
        .iter()
        .map(|quad| quad.destination[3])
        .fold(0.0f32, f32::max);
    let large_height = large_frame
        .quads
        .iter()
        .map(|quad| quad.destination[3])
        .fold(0.0f32, f32::max);
    assert!(large_height > small_height);
}

#[test]
fn empty_and_invalid_layouts_are_explicit() {
    let mut engine = engine();
    let empty = engine
        .layout("", &TextStyle::new(16.0).with_family("DejaVu Sans"), None)
        .unwrap();
    assert_eq!(empty.width(), 0.0);
    assert_eq!(empty.char_count(), 0);
    assert_eq!(empty.caret(0).x, 0.0);
    assert!(
        engine
            .layout("x", &TextStyle::new(0.0).with_family("DejaVu Sans"), None)
            .is_err()
    );
    assert!(
        engine
            .layout(
                "x",
                &TextStyle::new(16.0).with_family("DejaVu Sans"),
                Some(f32::NAN)
            )
            .is_err()
    );
}

#[test]
fn alignment_reflow_and_missing_glyphs_are_bounded() {
    let mut engine = engine();
    let centered = engine
        .layout(
            "short\nlonger line",
            &TextStyle::new(16.0)
                .with_family("DejaVu Sans")
                .with_align(TextAlign::Center),
            Some(240.0),
        )
        .unwrap();
    let first = centered
        .glyphs()
        .iter()
        .find(|glyph| glyph.line_top == 0.0)
        .unwrap();
    assert!(first.x > 0.0, "centered line should not start at x=0");

    let wide = engine
        .layout(
            "one two three four five six",
            &TextStyle::new(16.0)
                .with_family("DejaVu Sans")
                .with_wrap(TextWrap::Word),
            Some(320.0),
        )
        .unwrap();
    let narrow = engine
        .layout(
            "one two three four five six",
            &TextStyle::new(16.0)
                .with_family("DejaVu Sans")
                .with_wrap(TextWrap::Word),
            Some(90.0),
        )
        .unwrap();
    assert!(narrow.lines().len() > wide.lines().len());
    assert!(narrow.height() > wide.height());

    let missing = engine
        .layout(
            "A\u{10FFFF}B",
            &TextStyle::new(16.0).with_family("DejaVu Sans"),
            None,
        )
        .unwrap();
    assert!(missing.width().is_finite());
    assert!(missing.height().is_finite());
    assert!(!missing.glyphs().is_empty());
}

#[test]
fn right_to_left_caret_and_selection_follow_visual_order() {
    let mut engine = engine();
    let layout = engine
        .layout(
            "مرحبا",
            &TextStyle::new(20.0).with_family("DejaVu Sans"),
            None,
        )
        .unwrap();
    assert!(layout.glyphs().iter().all(|glyph| glyph.rtl));
    let start = layout.caret(0);
    let end = layout.caret(layout.char_count());
    assert!(
        start.x > end.x,
        "RTL caret start {} should be right of end {}",
        start.x,
        end.x
    );
    assert_eq!(layout.hit(start.x - 1.0, start.top + 2.0), 0);
    let rects = layout.selection_rects(0, layout.char_count());
    assert!(!rects.is_empty());
    assert!(rects.iter().all(|rect| rect.width > 0.0));
}

#[test]
fn advanced_shaping_forms_ligature_clusters_and_kerning() {
    let mut engine = engine();
    let layout = engine
        .layout(
            "office AVA",
            &TextStyle::new(24.0).with_family("DejaVu Sans"),
            None,
        )
        .unwrap();
    assert!(
        layout
            .glyphs()
            .iter()
            .any(|glyph| glyph.char_range.end - glyph.char_range.start > 1),
        "expected a multi-character shaped cluster/ligature"
    );
    let av = engine
        .layout("AV", &TextStyle::new(24.0).with_family("DejaVu Sans"), None)
        .unwrap();
    let a = engine
        .layout("A", &TextStyle::new(24.0).with_family("DejaVu Sans"), None)
        .unwrap();
    let v = engine
        .layout("V", &TextStyle::new(24.0).with_family("DejaVu Sans"), None)
        .unwrap();
    assert!(
        av.width() < a.width() + v.width(),
        "kerning should reduce the AV advance"
    );
}

use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]
    #[test]
    fn arbitrary_unicode_layouts_stay_finite_and_bounded(
        text in ".{0,160}",
        width in 40.0f32..600.0,
        size in 8.0f32..48.0,
    ) {
        let mut engine = engine();
        let style = TextStyle::new(size)
            .with_family("DejaVu Sans")
            .with_wrap(TextWrap::WordOrGlyph);
        let layout = engine.layout(&text, &style, Some(width)).unwrap();
        prop_assert!(layout.width().is_finite() && layout.width() >= 0.0);
        prop_assert!(layout.height().is_finite() && layout.height() > 0.0);
        for glyph in layout.glyphs() {
            prop_assert!(glyph.x.is_finite() && glyph.baseline.is_finite());
            prop_assert!(glyph.width.is_finite() && glyph.width >= 0.0);
        }
        let caret = layout.caret(layout.char_count());
        prop_assert!(caret.x.is_finite() && caret.top.is_finite());
    }
}
