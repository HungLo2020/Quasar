use crate::*;
fn indexed(textures: &mut Textures, width: u32, height: u32, bytes: &[u8]) -> TextureId {
    textures.insert(
        IndexedImage::new(width, height, bytes)
            .unwrap()
            .image()
            .clone(),
    )
}
fn palette(textures: &mut Textures, colors: &[[u8; 4]]) -> TextureId {
    textures.insert(
        Palette::new(colors.iter().copied().map(Color).collect::<Vec<_>>())
            .unwrap()
            .image(),
    )
}
#[test]
fn indexed_validation_swaps_cycles_and_zero_are_explicit() {
    assert!(IndexedImage::new(0, 1, []).is_err());
    assert!(IndexedImage::new(2, 1, [1]).is_err());
    assert!(Palette::new(Vec::<Color>::new()).is_err());
    assert!(Palette::new(vec![Color::WHITE; 257]).is_err());
    let p = Palette::new(vec![
        Color([10, 0, 0, 255]),
        Color([20, 0, 0, 255]),
        Color([30, 0, 0, 255]),
    ])
    .unwrap();
    assert!(p.cycled(1..1, 1).is_err());
    assert!(p.cycled(0..4, 1).is_err());
    assert_eq!(p.cycled(1..3, u64::MAX).unwrap().colors()[1], p.colors()[2]);
    let mut textures = Textures::default();
    let id = indexed(&mut textures, 4, 1, &[0, 1, 2, 255]);
    let a = textures.insert(p.image());
    let b = textures.insert(p.cycled(0..3, 1).unwrap().image());
    let mut canvas = Canvas::new(4, 3).unwrap();
    for (y, palette, transparent_index) in [(0., a, None), (1., b, None), (2., a, Some(0))] {
        canvas.set_palette(Some(PaletteBinding {
            texture: palette,
            transparent_index,
        }));
        let mut s = Sprite::new(id, Rect::new(0., y, 4., 1.));
        s.filter = Some(Filter::Linear);
        canvas.sprite(s);
    }
    let out = reference::rasterize(&canvas, &textures).unwrap();
    assert_eq!(&out[..12], &[10, 0, 0, 255, 20, 0, 0, 255, 30, 0, 0, 255]);
    assert_eq!(&out[16..28], &[30, 0, 0, 255, 10, 0, 0, 255, 20, 0, 0, 255]);
    assert_eq!(&out[32..36], &[0, 0, 0, 255]);
    assert_eq!(&out[12..16], &[0, 0, 0, 255]); // Out of palette is transparent.
    textures.remove(a);
    assert!(canvas.validate(&textures).is_err());
    assert!(Priority2D::new(1, 1).is_err());
    assert!(Priority2D::new(3, 4).is_err());
}
fn priority_scene() -> (Canvas, Textures) {
    let mut textures = Textures::default();
    let bg = indexed(&mut textures, 4, 1, &[0, 1, 1, 1]);
    let high = indexed(&mut textures, 4, 1, &[1, 1, 0, 0]);
    let low = indexed(&mut textures, 4, 1, &[1, 1, 1, 0]);
    let bgp = palette(&mut textures, &[[12, 16, 20, 255], [0, 100, 0, 255]]);
    let red = palette(&mut textures, &[[0, 0, 0, 0], [255, 0, 0, 255]]);
    let blue = palette(&mut textures, &[[0, 0, 0, 0], [0, 0, 255, 255]]);
    let mut canvas = Canvas::new(4, 1).unwrap();
    let pri = Priority2D::default();
    canvas.set_palette(Some(PaletteBinding {
        texture: bgp,
        transparent_index: None,
    }));
    canvas.background_sprite(
        Sprite::new(bg, Rect::new(0., 0., 4., 1.)),
        SpriteTransform::default(),
        pri,
        Some(0),
    );
    canvas.set_palette(Some(PaletteBinding {
        texture: red,
        transparent_index: Some(0),
    }));
    canvas.priority_sprite(
        Sprite::new(high, Rect::new(0., 0., 4., 1.)),
        SpriteTransform::default(),
        pri,
        true,
    );
    canvas.set_palette(Some(PaletteBinding {
        texture: blue,
        transparent_index: Some(0),
    }));
    canvas.priority_sprite(
        Sprite::new(low, Rect::new(0., 0., 4., 1.)),
        SpriteTransform::default(),
        pri,
        false,
    );
    (canvas, textures)
}
#[test]
fn micromages_doorway_and_jackal_hidden_oam_sprite_claim_pixels() {
    let (canvas, textures) = priority_scene();
    // High sprite behind scenery blocks lower sprite at x=1. Its hole at x=2
    // allows the lower sprite through. Background index zero at x=0 is nonopaque.
    assert_eq!(
        reference::rasterize(&canvas, &textures).unwrap(),
        vec![
            255, 0, 0, 255, 0, 100, 0, 255, 0, 0, 255, 255, 0, 100, 0, 255
        ]
    );
}
fn conformance_scene(frame: u32) -> (Canvas, Textures) {
    let mut textures = Textures::default();
    let id = indexed(&mut textures, 3, 2, &[0, 1, 2, 3, 2, 1]);
    let bank = Palette::new(vec![
        Color([20, 30, 40, 255]),
        Color([200, 20, 30, 255]),
        Color([10, 180, 30, 255]),
        Color([20, 30, 200, 128]),
    ])
    .unwrap();
    let p = textures.insert(bank.cycled(1..4, frame as u64).unwrap().image());
    let mut c = Canvas::new(40, 32).unwrap();
    c.clear = Color([7, 9, 11, 255]);
    c.set_palette(Some(PaletteBinding {
        texture: p,
        transparent_index: None,
    }));
    for i in 0..8 {
        let s = Sprite::new(
            id,
            Rect::new((i % 4 * 9) as f32, (i / 4 * 9) as f32, 6., 6.),
        );
        c.sprite_transformed(
            s,
            SpriteTransform {
                transpose: i & 1 != 0,
                flip_x: i & 2 != 0,
                flip_y: i & 4 != 0,
            },
        );
    }
    // A mask-only write and matching/nonmatching reads, overwrite selected bits,
    // clipped coverage, and ordinary RGBA in the same pass.
    c.set_palette(None);
    c.set_clip(Some(Rect::new(2., 20., 24., 10.)));
    c.set_pixel_state(PixelState {
        reference: 0xa0,
        write_mask: 0xf0,
        color_write: false,
        alpha_min: 128,
        ..Default::default()
    });
    c.rect(Rect::new(0., 18., 20., 14.), [255, 255, 255, 128]);
    c.set_pixel_state(PixelState {
        read_mask: 0xf0,
        reference: 0xa0,
        ..Default::default()
    });
    c.rect(Rect::new(0., 20., 40., 10.), [255, 180, 10, 255]);
    c.set_pixel_state(PixelState {
        read_mask: 0x80,
        reference: 0,
        ..Default::default()
    });
    c.rect(Rect::new(0., 23., 40., 2.), [10, 255, 255, 255]);
    c.set_pixel_state(PixelState::default());
    c.set_clip(None);
    c.rect(Rect::new(30., 20., 5., 5.), [100, 40, 200, 128]);
    (c, textures)
}
#[test]
fn masks_are_pixel_level_clipped_and_reset_for_each_canvas() {
    let (c, t) = conformance_scene(0);
    let p = reference::rasterize(&c, &t).unwrap();
    let pixel = |x: u32, y: u32| &p[((y * 40 + x) * 4) as usize..((y * 40 + x) * 4 + 4) as usize];
    assert_eq!(pixel(3, 21), [255, 180, 10, 255]);
    assert_eq!(pixel(21, 23), [10, 255, 255, 255]);
    assert_eq!(pixel(3, 23), [255, 180, 10, 255]);
    assert_eq!(p, reference::rasterize(&c, &t).unwrap());
}
#[cfg(feature = "gpu")]
#[test]
#[ignore = "native GPU pixel parity; run explicitly"]
fn gpu_indexed_masks_priority_and_rgba_match_reference() {
    let mut r = Renderer2D::headless().unwrap();
    let (c, t) = priority_scene();
    assert_eq!(
        r.render_to_image(&c, &t).unwrap(),
        reference::rasterize(&c, &t).unwrap()
    );
    for frame in 0..6 {
        let (c, t) = conformance_scene(frame);
        let expected = reference::rasterize(&c, &t).unwrap();
        let actual = r.render_to_image(&c, &t).unwrap();
        let differences: Vec<_> = actual
            .iter()
            .zip(&expected)
            .enumerate()
            .filter(|(_, (a, b))| a.abs_diff(**b) > 1)
            .collect();
        assert!(
            differences.is_empty(),
            "frame={frame}, different={} first={:?}",
            differences.len(),
            differences.first()
        );
        r.render_to_image(&c, &t).unwrap();
        assert_eq!(r.stats().texture_upload_bytes, 0);
    }
}
#[cfg(feature = "assets")]
#[test]
fn mounted_png_assets_share_cache_and_pixel_storage_with_textures() {
    use quasar_assets::{AssetServer, MemorySource};
    use std::sync::Arc;
    let mut source = MemorySource::new("preloaded");
    source
        .insert(
            "atlas.png",
            include_bytes!("../../../examples/tilequest/assets/atlas.png").to_vec(),
        )
        .unwrap();
    let mut server = AssetServer::new();
    server.mount("", Arc::new(source)).unwrap();
    server.register(PngLoader);
    let a = server.load::<PngLoader>("atlas.png").unwrap();
    let b = server.load::<PngLoader>("atlas.png").unwrap();
    assert_eq!(a.rgba().as_ptr(), b.rgba().as_ptr());
    let mut textures = Textures::default();
    let id = textures.insert((*a).clone());
    assert_eq!(a.rgba().as_ptr(), textures.get(id).unwrap().rgba().as_ptr());
    assert_eq!(server.cache_stats().entries, 1);
}

fn all_indices_scene() -> (Canvas, Textures) {
    let mut textures = Textures::default();
    let bytes: Vec<u8> = (0..=255).collect();
    let id = indexed(&mut textures, 16, 16, &bytes);
    let colors: Vec<_> = (0..=255).map(|i| Color([i, 255 - i, i / 2, 255])).collect();
    let p = textures.insert(Palette::new(colors.clone()).unwrap().image());
    let expanded = textures
        .insert(Image::new(16, 16, colors.iter().flat_map(|c| c.0).collect::<Vec<_>>()).unwrap());
    let mut c = Canvas::new(32, 32).unwrap();
    c.sprite(Sprite::new(expanded, Rect::new(16., 0., 16., 16.)));
    c.set_palette(Some(PaletteBinding {
        texture: p,
        transparent_index: None,
    }));
    c.sprite(Sprite::new(id, Rect::new(0., 0., 16., 16.)));
    // Exercise the maximum encoded transparent/excluded index, after crop/flip.
    c.set_palette(Some(PaletteBinding {
        texture: p,
        transparent_index: Some(255),
    }));
    let mut s = Sprite::new(id, Rect::new(0., 16., 16., 16.));
    s.tint = Color([128, 200, 255, 128]);
    c.sprite_flipped(s, true, true);
    c.set_palette(Some(PaletteBinding {
        texture: p,
        transparent_index: None,
    }));
    c.set_pixel_state(PixelState {
        exclude_index: Some(255),
        ..Default::default()
    });
    s.destination.x = 16.;
    c.sprite_flipped(s, true, true);
    (c, textures)
}
#[test]
fn all_256_indices_match_expanded_rgba_and_max_index_coverage() {
    let (c, t) = all_indices_scene();
    let p = reference::rasterize(&c, &t).unwrap();
    for y in 0..32 {
        for x in 0..16 {
            let a = ((y * 32 + x) * 4) as usize;
            let b = a + 16 * 4;
            assert_eq!(&p[a..a + 4], &p[b..b + 4]);
        }
    }
    assert_eq!(&p[(16 * 32 * 4)..(16 * 32 * 4 + 4)], &[0, 0, 0, 255]);
}
#[cfg(feature = "gpu")]
#[test]
#[ignore = "native GPU full index range parity"]
fn gpu_all_256_indices_match_expanded_rgba_and_reference() {
    let (c, t) = all_indices_scene();
    let mut r = Renderer2D::headless().unwrap();
    let cpu = reference::rasterize(&c, &t).unwrap();
    let gpu = r.render_to_image(&c, &t).unwrap();
    assert!(gpu.iter().zip(&cpu).all(|(a, b)| a.abs_diff(*b) <= 1));
}
