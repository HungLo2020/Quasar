use crate::*;
#[test]
fn camera_transform_clip_and_order_have_expected_pixels() {
    let textures = Textures::default();
    let mut canvas = Canvas::new(16, 16).unwrap();
    canvas.rect(Rect::new(0.0, 0.0, 16.0, 16.0), [0, 0, 255, 255]);
    canvas.set_camera(Camera2D {
        center: [4.0, 4.0],
        zoom: 2.0,
        rotation: 0.0,
    });
    canvas.set_clip(Some(Rect::new(4.0, 4.0, 4.0, 4.0)));
    canvas.rect(Rect::new(2.0, 2.0, 4.0, 4.0), [255, 0, 0, 128]);
    let pixels = reference::rasterize(&canvas, &textures).unwrap();
    assert_eq!(
        &pixels[(5 * 16 + 5) * 4..(5 * 16 + 5) * 4 + 4],
        &[128, 0, 127, 255]
    );
    assert_eq!(
        &pixels[(3 * 16 + 3) * 4..(3 * 16 + 3) * 4 + 4],
        &[0, 0, 255, 255]
    );
}
#[test]
fn removed_textures_invalid_regions_and_non_finite_geometry_are_rejected() {
    let mut textures = Textures::default();
    let id = textures.insert(Image::new(1, 1, vec![255; 4]).unwrap());
    let mut canvas = Canvas::new(8, 8).unwrap();
    canvas.sprite(Sprite::new(id, Rect::new(0.0, 0.0, 2.0, 2.0)));
    assert!(canvas.validate(&textures).is_ok());
    textures.remove(id);
    assert!(canvas.validate(&textures).is_err());
    let mut canvas = Canvas::new(8, 8).unwrap();
    let mut sprite = Sprite::new(TextureId::WHITE, Rect::new(0.0, 0.0, 2.0, 2.0));
    sprite.source = Some(Rect::new(0.0, 0.0, 2.0, 2.0));
    canvas.sprite(sprite);
    assert!(canvas.validate(&textures).is_err());
    let mut canvas = Canvas::new(8, 8).unwrap();
    canvas.rect(Rect::new(f32::NAN, 0.0, 1.0, 1.0), Color::WHITE);
    assert!(canvas.validate(&textures).is_err());
}
#[test]
fn reference_supports_asymmetric_atlases_and_text() {
    let mut textures = Textures::default();
    let id = textures.insert(
        Image::new(
            2,
            2,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
            ],
        )
        .unwrap(),
    );
    let mut canvas = Canvas::new(16, 16).unwrap();
    canvas.sprite(Sprite::new(id, Rect::new(0.0, 0.0, 8.0, 8.0)));
    canvas.text([8.0, 8.0], "A", 1.0, Color::WHITE);
    let pixels = reference::rasterize(&canvas, &textures).unwrap();
    assert_eq!(&pixels[0..4], &[255, 0, 0, 255]);
    assert_eq!(
        &pixels[(7 * 16 + 7) * 4..(7 * 16 + 7) * 4 + 4],
        &[255, 255, 0, 255]
    );
    assert!(
        pixels[(8 * 16 + 8) * 4..]
            .chunks_exact(4)
            .any(|pixel| pixel == [255; 4])
    );
}
#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires Vulkan; run cargo test -p quasar-render-2d -- --include-ignored"]
fn gpu_pixels_match_reference_for_sprites_blending_camera_rotation_and_clips() {
    let mut renderer = Renderer2D::headless().unwrap();
    let mut textures = Textures::default();
    let id = textures.insert(
        Image::new(
            2,
            2,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
            ],
        )
        .unwrap(),
    );
    let mut canvas = Canvas::new(64, 64).unwrap();
    canvas.clear = Color([0, 0, 255, 255]);
    canvas.rect(Rect::new(8.0, 8.0, 20.0, 20.0), [255, 0, 0, 255]);
    let mut sprite = Sprite::new(id, Rect::new(16.0, 16.0, 16.0, 16.0));
    sprite.source = Some(Rect::new(1.0, 0.0, 1.0, 1.0));
    canvas.sprite(sprite);
    canvas.rect(Rect::new(12.0, 12.0, 12.0, 12.0), [0, 255, 0, 128]);
    canvas.set_transform(Transform2D::rotation(0.4).then(Transform2D::translation(38.0, 8.0)));
    canvas.rect(Rect::new(-4.0, -4.0, 8.0, 12.0), [255, 255, 0, 255]);
    canvas.set_transform(Transform2D::IDENTITY);
    canvas.set_clip(Some(Rect::new(44.0, 10.0, 4.0, 16.0)));
    canvas.rect(Rect::new(40.0, 4.0, 16.0, 32.0), [0, 255, 255, 255]);
    canvas.set_clip(None);
    canvas.sprite(Sprite::new(id, Rect::new(36.0, 40.0, 16.0, 16.0)));
    canvas.text([2.0, 42.0], "A2", 1.0, Color::WHITE);
    let expected = reference::rasterize(&canvas, &textures).unwrap();
    let pixels = renderer.render_to_image(&canvas, &textures).unwrap();
    let different: Vec<_> = pixels
        .iter()
        .zip(&expected)
        .enumerate()
        .filter(|(_, (a, b))| a.abs_diff(**b) > 1)
        .collect();
    assert!(
        different.is_empty(),
        "{} channels differ; first {:?}",
        different.len(),
        different.first()
    );
    let stats = renderer.stats();
    assert!(stats.batches < stats.quads);
    assert!(stats.texture_upload_bytes > 0);
    renderer.render_to_image(&canvas, &textures).unwrap();
    assert_eq!(renderer.stats().texture_upload_bytes, 0);
    textures.remove(id);
    assert!(renderer.render_to_image(&canvas, &textures).is_err());
    let mut empty = Canvas::new(4, 4).unwrap();
    empty.clear = Color([7, 8, 9, 255]);
    assert_eq!(
        renderer.render_to_image(&empty, &textures).unwrap(),
        [7, 8, 9, 255].repeat(16)
    );
}
#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires Vulkan; run cargo test -p quasar-render-2d -- --include-ignored"]
fn gpu_batches_many_quads_and_grows_buffers_without_losing_resources() {
    let mut renderer = Renderer2D::headless().unwrap();
    let textures = Textures::default();
    for count in [1, 1000, 2000, 3] {
        let mut canvas = Canvas::new(16, 16).unwrap();
        for _ in 0..count {
            canvas.rect(Rect::new(0.0, 0.0, 8.0, 8.0), [64, 128, 255, 255]);
        }
        let pixels = renderer.render_to_image(&canvas, &textures).unwrap();
        assert_eq!(&pixels[0..4], &[64, 128, 255, 255]);
        assert_eq!(renderer.stats().batches, 1);
        assert_eq!(renderer.stats().quads, count);
    }
}
