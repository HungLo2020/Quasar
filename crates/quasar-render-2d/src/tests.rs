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
            .as_chunks::<4>()
            .0
            .contains(&[255; 4])
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

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires Vulkan; run cargo test -p quasar-render-2d -- --include-ignored"]
fn dynamic_framebuffer_updates_reuse_texture_and_match_pixels() {
    let mut renderer = Renderer2D::headless().unwrap();
    let mut textures = Textures::default();
    let dynamic = renderer
        .create_dynamic_texture(&mut textures, 2, 2)
        .unwrap();
    let mut canvas = Canvas::new(2, 2).unwrap();
    canvas.sprite(Sprite::new(dynamic.id(), Rect::new(0.0, 0.0, 2.0, 2.0)));
    let first = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];
    renderer.update_dynamic_texture(dynamic, &first).unwrap();
    assert_eq!(renderer.render_to_image(&canvas, &textures).unwrap(), first);
    assert_eq!(renderer.stats().texture_upload_bytes, first.len());
    assert_eq!(renderer.dynamic_texture_count(), 1);

    let second = [1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255];
    renderer.update_dynamic_texture(dynamic, &second).unwrap();
    assert_eq!(
        renderer.render_to_image(&canvas, &textures).unwrap(),
        second
    );
    assert_eq!(renderer.stats().texture_upload_bytes, second.len());
    assert_eq!(renderer.dynamic_texture_count(), 1);

    renderer
        .destroy_dynamic_texture(&mut textures, dynamic)
        .unwrap();
    assert_eq!(renderer.dynamic_texture_count(), 0);
    assert!(renderer.update_dynamic_texture(dynamic, &first).is_err());
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires Vulkan; run cargo test -p quasar-render-2d -- --include-ignored"]
fn dynamic_texture_requires_staged_pixels_before_draw() {
    let mut renderer = Renderer2D::headless().unwrap();
    let mut textures = Textures::default();
    let dynamic = renderer
        .create_dynamic_texture(&mut textures, 2, 2)
        .unwrap();
    let mut canvas = Canvas::new(2, 2).unwrap();
    canvas.sprite(Sprite::new(dynamic.id(), Rect::new(0.0, 0.0, 2.0, 2.0)));
    assert!(renderer.render_to_image(&canvas, &textures).is_err());
    renderer
        .update_dynamic_texture(dynamic, &[9, 8, 7, 255].repeat(4))
        .unwrap();
    assert!(renderer.render_to_image(&canvas, &textures).is_ok());
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires Vulkan; run cargo test -p quasar-render-2d -- --include-ignored"]
fn dynamic_texture_uses_nearest_and_linear_sampling() {
    let mut renderer = Renderer2D::headless().unwrap();
    let mut textures = Textures::default();
    let dynamic = renderer
        .create_dynamic_texture(&mut textures, 2, 2)
        .unwrap();
    let checker = [
        0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255,
    ];
    let mut nearest_canvas = Canvas::new(8, 8).unwrap();
    let mut sprite = Sprite::new(dynamic.id(), Rect::new(0.0, 0.0, 8.0, 8.0));
    sprite.filter = Some(Filter::Nearest);
    nearest_canvas.sprite(sprite);
    renderer.update_dynamic_texture(dynamic, &checker).unwrap();
    let nearest = renderer
        .render_to_image(&nearest_canvas, &textures)
        .unwrap();

    let mut linear_canvas = Canvas::new(8, 8).unwrap();
    let mut sprite = Sprite::new(dynamic.id(), Rect::new(0.0, 0.0, 8.0, 8.0));
    sprite.filter = Some(Filter::Linear);
    linear_canvas.sprite(sprite);
    renderer.update_dynamic_texture(dynamic, &checker).unwrap();
    let linear = renderer.render_to_image(&linear_canvas, &textures).unwrap();

    let center = (4 * 8 + 4) * 4;
    assert_ne!(
        &nearest[center..center + 4],
        &linear[center..center + 4],
        "linear sampling should blend the checkerboard"
    );
    assert_eq!(&nearest[0..4], &checker[0..4]);
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires Vulkan; run cargo test -p quasar-render-2d -- --include-ignored"]
fn dynamic_and_static_textures_compose_in_one_draw_list() {
    let mut renderer = Renderer2D::headless().unwrap();
    let mut textures = Textures::default();
    let dynamic = renderer
        .create_dynamic_texture(&mut textures, 2, 2)
        .unwrap();
    let dynamic_two = renderer
        .create_dynamic_texture(&mut textures, 2, 2)
        .unwrap();
    let static_id = textures.insert(Image::new(2, 2, [0, 0, 255, 255].repeat(4)).unwrap());
    let mut canvas = Canvas::new(6, 2).unwrap();
    canvas.sprite(Sprite::new(dynamic.id(), Rect::new(0.0, 0.0, 2.0, 2.0)));
    canvas.sprite(Sprite::new(dynamic_two.id(), Rect::new(2.0, 0.0, 2.0, 2.0)));
    canvas.sprite(Sprite::new(static_id, Rect::new(4.0, 0.0, 2.0, 2.0)));
    let dynamic_pixels = [
        255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
    ];
    let second_pixels = [
        0, 255, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255,
    ];
    renderer
        .update_dynamic_texture(dynamic, &dynamic_pixels)
        .unwrap();
    renderer
        .update_dynamic_texture(dynamic_two, &second_pixels)
        .unwrap();
    let pixels = renderer.render_to_image(&canvas, &textures).unwrap();
    assert_eq!(&pixels[0..4], &[255, 0, 0, 255]);
    assert_eq!(&pixels[(2 * 4)..(2 * 4) + 4], &[0, 255, 0, 255]);
    assert_eq!(&pixels[(4 * 4)..(4 * 4) + 4], &[0, 0, 255, 255]);
    assert_eq!(
        renderer.stats().texture_upload_bytes,
        48,
        "two 16-byte dynamic uploads plus the first static image upload"
    );
}

#[cfg(all(feature = "gpu", feature = "vulkan"))]
#[test]
#[ignore = "requires Vulkan; run cargo test -p quasar-render-2d -- --include-ignored"]
fn out_of_order_prepared_dynamic_frames_keep_their_own_pixels() {
    use vulkanic_gal::*;

    let mut gal = VulkanicGal::create(BackendChoice::Vulkan, "dynamic-order-test").unwrap();
    let mut state = RenderState2D::new(&mut gal).unwrap();
    let mut textures = Textures::default();
    let dynamic = state
        .create_dynamic_texture(&mut gal, &mut textures, 2, 2)
        .unwrap();
    let mut canvas = Canvas::new(2, 2).unwrap();
    canvas.sprite(Sprite::new(dynamic.id(), Rect::new(0.0, 0.0, 2.0, 2.0)));

    let (texture_a, view_a, target_a, readback_a) = test_target(&mut gal, 2);
    let (texture_b, view_b, target_b, readback_b) = test_target(&mut gal, 2);

    let first = [
        255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
    ];
    state.update_dynamic_texture(dynamic, &first).unwrap();
    let mut prepared_a = state
        .prepare(
            &mut gal,
            &canvas,
            &textures,
            target_a,
            DisplayOptions::default(),
            LoadOp::Clear,
        )
        .unwrap();

    let second = [
        0, 255, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255,
    ];
    state.update_dynamic_texture(dynamic, &second).unwrap();
    let mut prepared_b = state
        .prepare(
            &mut gal,
            &canvas,
            &textures,
            target_b,
            DisplayOptions::default(),
            LoadOp::Clear,
        )
        .unwrap();

    append_readback(&mut prepared_a.commands, texture_a, readback_a, 2);
    append_readback(&mut prepared_b.commands, texture_b, readback_b, 2);

    // B was prepared second but is submitted first; each list carries its own
    // framebuffer snapshot, so A must still read the first pixels.
    let token_b = gal
        .submit(SubmissionBatch {
            label: "dynamic-order-b".into(),
            command_lists: vec![prepared_b.commands],
        })
        .unwrap();
    let token_a = gal
        .submit(SubmissionBatch {
            label: "dynamic-order-a".into(),
            command_lists: vec![prepared_a.commands],
        })
        .unwrap();
    assert!(token_a.submission > token_b.submission);
    gal.retire_through(token_a.submission).unwrap();

    let reads = gal.completed_host_reads();
    let read = |buffer| {
        reads
            .iter()
            .find(|read| read.buffer == buffer)
            .unwrap()
            .bytes
            .clone()
    };
    assert_eq!(read(readback_b), second);
    assert_eq!(read(readback_a), first);

    state
        .destroy_dynamic_texture(&mut gal, &mut textures, dynamic)
        .unwrap();
    state.release(&mut gal).unwrap();
    for (texture, view, target, readback) in [
        (texture_a, view_a, target_a, readback_a),
        (texture_b, view_b, target_b, readback_b),
    ] {
        gal.destroy(readback).unwrap();
        gal.destroy(target).unwrap();
        gal.destroy(view).unwrap();
        gal.destroy(texture).unwrap();
    }
}

#[cfg(feature = "gpu")]
#[cfg(feature = "vulkan")]
fn test_target(
    gal: &mut vulkanic_gal::VulkanicGal,
    size: u32,
) -> (
    vulkanic_gal::Handle,
    vulkanic_gal::Handle,
    vulkanic_gal::Handle,
    vulkanic_gal::Handle,
) {
    use vulkanic_gal::*;
    let extent = Extent3d {
        width: size,
        height: size,
        depth: 1,
    };
    let texture = gal
        .create_texture(TextureDesc {
            label: "dynamic-test-target".into(),
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            extent,
            mip_levels: 1,
            array_layers: 1,
            usages: vec![TextureUsage::ColorAttachment, TextureUsage::TransferSrc],
        })
        .unwrap();
    let view = gal
        .create_texture_view(TextureViewDesc {
            label: "dynamic-test-view".into(),
            texture,
            format: TextureFormat::Rgba8Unorm,
            base_mip: 0,
            mip_count: 1,
            base_layer: 0,
            layer_count: 1,
        })
        .unwrap();
    let target = gal
        .create_render_target(RenderTargetDesc {
            label: "dynamic-test-target".into(),
            color_views: vec![view],
            depth_stencil_view: None,
            extent,
        })
        .unwrap();
    let readback = gal
        .create_buffer(BufferDesc {
            label: "dynamic-test-readback".into(),
            size: u64::from(size) * u64::from(size) * 4,
            memory: MemoryDomain::Readback,
            usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
        })
        .unwrap();
    (texture, view, target, readback)
}

#[cfg(feature = "gpu")]
#[cfg(feature = "vulkan")]
fn append_readback(
    list: &mut vulkanic_gal::CommandList,
    texture: vulkanic_gal::Handle,
    readback: vulkanic_gal::Handle,
    size: u32,
) {
    use vulkanic_gal::*;
    let extent = Extent3d {
        width: size,
        height: size,
        depth: 1,
    };
    let barrier = |resource, before, after| {
        CommandOp::Barrier(ResourceBarrier {
            resource,
            subresources: None,
            before,
            after,
            src_queue: QueueClass::Graphics,
            dst_queue: QueueClass::Graphics,
        })
    };
    list.operations.extend([
        barrier(
            texture,
            TextureUsageState::ColorAttachment,
            TextureUsageState::TransferSrc,
        ),
        CommandOp::CopyTextureToBuffer(BufferImageCopyRegion {
            buffer: readback,
            buffer_offset: 0,
            bytes_per_row: size * 4,
            rows_per_image: size,
            texture,
            texture_mip: 0,
            texture_layer: 0,
            texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
            extent,
        }),
        barrier(
            readback,
            TextureUsageState::TransferDst,
            TextureUsageState::TransferSrc,
        ),
        CommandOp::HostReadBuffer {
            buffer: readback,
            offset: 0,
            size: u64::from(size) * u64::from(size) * 4,
        },
    ]);
}

#[test]
fn framebuffer_viewport_scales_and_letterboxes() {
    let fit = Viewport::calculate((640, 400), (1000, 600), DisplayOptions::default()).unwrap();
    assert_eq!(
        (fit.x, fit.y, fit.width, fit.height),
        (20, 0, 960, 600),
        "fit must preserve aspect and center bars"
    );
    let bars = Viewport::calculate((640, 400), (800, 600), DisplayOptions::default()).unwrap();
    assert_eq!(
        (bars.x, bars.y, bars.width, bars.height),
        (0, 50, 800, 500),
        "vertical letterbox bars must be centered"
    );
    let integer = Viewport::calculate(
        (640, 400),
        (1000, 600),
        DisplayOptions {
            scaling: Scaling::Integer,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        (integer.x, integer.y, integer.width, integer.height),
        (180, 100, 640, 400),
        "integer scaling must use whole-pixel multiples"
    );
    let fallback = Viewport::calculate(
        (640, 400),
        (320, 200),
        DisplayOptions {
            scaling: Scaling::Integer,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        (fallback.x, fallback.y, fallback.width, fallback.height),
        (0, 0, 320, 200),
        "integer mode must fall back to fit when the window is smaller"
    );
}

#[cfg(feature = "png")]
#[test]
fn png_write_round_trips_rgba() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("framebuffer.png");
    let pixels = [1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 128, 10, 11, 12, 255];
    write_png(&path, 2, 2, &pixels).unwrap();
    let image = Image::load_png(&path).unwrap();
    assert_eq!(image.rgba(), &pixels);
    assert!(write_png(&path, 2, 2, &pixels[..4]).is_err());
}

#[test]
fn sprite_transforms_match_all_eight_asymmetric_orientations() {
    let mut textures = Textures::default();
    let colors = [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 0, 255],
    ];
    let id = textures
        .insert(Image::new(2, 2, colors.into_iter().flatten().collect::<Vec<_>>()).unwrap());
    // Source A B / C D. Transpose first, then H/V in destination coordinates.
    let expected = [
        [0, 1, 2, 3],
        [1, 0, 3, 2],
        [2, 3, 0, 1],
        [3, 2, 1, 0],
        [0, 2, 1, 3],
        [2, 0, 3, 1],
        [1, 3, 0, 2],
        [3, 1, 2, 0],
    ];
    for (bits, pose) in expected.iter().enumerate() {
        let mut canvas = Canvas::new(2, 2).unwrap();
        canvas.sprite_transformed(
            Sprite::new(id, Rect::new(0., 0., 2., 2.)),
            SpriteTransform {
                flip_x: bits & 1 != 0,
                flip_y: bits & 2 != 0,
                transpose: bits & 4 != 0,
            },
        );
        let image = reference::rasterize(&canvas, &textures).unwrap();
        for (pixel, source) in image.as_chunks::<4>().0.iter().zip(*pose) {
            assert_eq!(*pixel, colors[source], "pose {bits}");
        }
    }
}
#[test]
fn atlases_and_animation_boundaries_are_checked() {
    let atlas = SpriteAtlas::grid(TextureId::WHITE, [10, 10], [3, 3], 1, 2).unwrap();
    assert_eq!(atlas.len(), 4);
    assert_eq!(atlas.region(3).unwrap(), Rect::new(6., 6., 3., 3.));
    assert!(atlas.region(4).is_err());
    assert!(SpriteAtlas::grid(TextureId::WHITE, [9, 9], [3, 3], 1, 2).is_err());
    let clip = AnimationClip::new(vec![
        AnimationFrame {
            frame: 4,
            duration_ms: 100,
        },
        AnimationFrame {
            frame: 9,
            duration_ms: 200,
        },
    ])
    .unwrap();
    for (time, frame) in [
        (0, 4),
        (99, 4),
        (100, 9),
        (299, 9),
        (300, 4),
        (u64::MAX, (if u64::MAX % 300 < 100 { 4 } else { 9 })),
    ] {
        assert_eq!(clip.frame_at(time), frame);
    }
    assert!(AnimationClip::new(vec![]).is_err());
    assert!(
        AnimationClip::new(vec![AnimationFrame {
            frame: 0,
            duration_ms: 0
        }])
        .is_err()
    );
}
#[cfg(feature = "png")]
#[test]
fn png_decode_expands_color_types_and_rejects_corruption() {
    for (kind, depth, pixels, expected) in [
        (
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            vec![12, 34, 56],
            vec![12, 34, 56, 255],
        ),
        (
            png::ColorType::Grayscale,
            png::BitDepth::Eight,
            vec![42],
            vec![42, 42, 42, 255],
        ),
        (
            png::ColorType::GrayscaleAlpha,
            png::BitDepth::Eight,
            vec![42, 100],
            vec![42, 42, 42, 100],
        ),
        (
            png::ColorType::Rgba,
            png::BitDepth::Sixteen,
            vec![12, 0, 34, 0, 56, 0, 128, 0],
            vec![12, 34, 56, 128],
        ),
    ] {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
            encoder.set_color(kind);
            encoder.set_depth(depth);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        }
        let image = Image::decode_png(std::io::Cursor::new(&bytes)).unwrap();
        assert_eq!(image.rgba(), expected);
        assert!(Image::decode_png(std::io::Cursor::new(&bytes[..bytes.len() / 2])).is_err());
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Indexed);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_palette(vec![10, 20, 30]);
        encoder.set_trns(vec![90]);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[0])
            .unwrap();
    }
    assert_eq!(
        Image::decode_png(std::io::Cursor::new(bytes))
            .unwrap()
            .rgba(),
        [10, 20, 30, 90]
    );
    assert!(Image::decode_png(std::io::Cursor::new(b"invalid")).is_err());
}

#[cfg(all(feature = "gpu", feature = "vulkan"))]
#[test]
#[ignore = "native GPU palette generations and stencil reservation"]
fn prepared_palette_generations_and_masks_survive_reverse_submission() {
    use vulkanic_gal::*;
    let mut gal = VulkanicGal::create(BackendChoice::Vulkan, "palette generations").unwrap();
    let mut state = RenderState2D::new(&mut gal).unwrap();
    let mut textures = Textures::default();
    let id = textures.insert(IndexedImage::new(2, 2, [1; 4]).unwrap().image().clone());
    let pa = textures.insert(
        Palette::new(vec![Color::BLACK, Color([255, 0, 0, 255])])
            .unwrap()
            .image(),
    );
    let pb = textures.insert(
        Palette::new(vec![Color::BLACK, Color([0, 0, 255, 255])])
            .unwrap()
            .image(),
    );
    let mut canvas = Canvas::new(2, 2).unwrap();
    canvas.set_pixel_state(PixelState {
        reference: 4,
        write_mask: 4,
        ..Default::default()
    });
    canvas.set_palette(Some(PaletteBinding {
        texture: pa,
        transparent_index: Some(0),
    }));
    canvas.sprite(Sprite::new(id, Rect::new(0., 0., 2., 2.)));
    let (ta, va, rta, ra) = test_target(&mut gal, 2);
    let (tb, vb, rtb, rb) = test_target(&mut gal, 2);
    let mut a = state
        .prepare(
            &mut gal,
            &canvas,
            &textures,
            rta,
            DisplayOptions::default(),
            LoadOp::Clear,
        )
        .unwrap();
    assert_eq!(a.stats.texture_upload_bytes, 16 + 1024);
    textures.remove(pa);
    let mut canvas_b = Canvas::new(2, 2).unwrap();
    canvas_b.set_pixel_state(canvas.pixel_state());
    canvas_b.set_palette(Some(PaletteBinding {
        texture: pb,
        transparent_index: Some(0),
    }));
    canvas_b.sprite(Sprite::new(id, Rect::new(0., 0., 2., 2.)));
    let mut b = state
        .prepare(
            &mut gal,
            &canvas_b,
            &textures,
            rtb,
            DisplayOptions::default(),
            LoadOp::Clear,
        )
        .unwrap();
    assert_eq!(
        b.stats.texture_upload_bytes, 1024,
        "palette swap must not upload indices"
    );
    assert!(state.release(&mut gal).is_err());
    assert!(state.release_targets(&mut gal).is_err());
    append_readback(&mut a.commands, ta, ra, 2);
    append_readback(&mut b.commands, tb, rb, 2);
    for commands in [b.commands, a.commands] {
        let token = gal
            .submit(SubmissionBatch {
                label: "palette reverse".into(),
                command_lists: vec![commands],
            })
            .unwrap();
        gal.retire_through(token.submission).unwrap();
    }
    let reads = gal.completed_host_reads();
    assert_eq!(
        reads.iter().find(|r| r.buffer == ra).unwrap().bytes,
        [255, 0, 0, 255].repeat(4)
    );
    assert_eq!(
        reads.iter().find(|r| r.buffer == rb).unwrap().bytes,
        [0, 0, 255, 255].repeat(4)
    );
    // Submitted generations are retired, mask state clears on the next prepare,
    // and repeated preparation on the same target reuses all cached GPU resources.
    let mut again = Canvas::new(2, 2).unwrap();
    again.set_palette(Some(PaletteBinding {
        texture: pb,
        transparent_index: Some(0),
    }));
    again.set_pixel_state(PixelState {
        read_mask: 4,
        reference: 0,
        ..Default::default()
    });
    again.sprite(Sprite::new(id, Rect::new(0., 0., 2., 2.)));
    let first = state
        .prepare(
            &mut gal,
            &again,
            &textures,
            rtb,
            DisplayOptions::default(),
            LoadOp::Clear,
        )
        .unwrap();
    assert_eq!(first.stats.texture_upload_bytes, 0);
    let token = gal
        .submit(SubmissionBatch {
            label: "mask cleared".into(),
            command_lists: vec![first.commands],
        })
        .unwrap();
    gal.retire_through(token.submission).unwrap();
    let creates = gal.metrics().resource_creates;
    let warm = state
        .prepare(
            &mut gal,
            &again,
            &textures,
            rtb,
            DisplayOptions::default(),
            LoadOp::Clear,
        )
        .unwrap();
    assert_eq!(
        creates,
        gal.metrics().resource_creates,
        "warm preparation must reuse targets, bindings and buffers"
    );
    drop(warm);
    state.release(&mut gal).unwrap();
    for h in [ra, rb, rta, rtb, va, vb, ta, tb] {
        gal.retire(h).unwrap();
    }
    gal.retire_through(gal.latest_submission_id()).unwrap();
    assert_eq!(
        gal.metrics().resource_creates,
        gal.metrics().resource_destroys
    );
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "native GPU indexed palette animation reuses dynamic uploads"]
fn dynamic_palette_updates_only_the_small_palette_texture() {
    let mut r = Renderer2D::headless().unwrap();
    let mut t = Textures::default();
    let index = t.insert(
        IndexedImage::new(256, 128, vec![1; 256 * 128])
            .unwrap()
            .image()
            .clone(),
    );
    let dynamic = r.create_dynamic_texture(&mut t, 256, 1).unwrap();
    let mut c = Canvas::new(4, 4).unwrap();
    c.set_palette(Some(PaletteBinding {
        texture: dynamic.id(),
        transparent_index: None,
    }));
    c.sprite(Sprite::new(index, Rect::new(0., 0., 4., 4.)));
    for (frame, color) in [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]]
        .into_iter()
        .enumerate()
    {
        let p = Palette::new(vec![Color::BLACK, Color(color)])
            .unwrap()
            .image();
        r.update_dynamic_texture(dynamic, p.rgba()).unwrap();
        assert_eq!(r.render_to_image(&c, &t).unwrap(), color.repeat(16));
        assert_eq!(
            r.stats().texture_upload_bytes,
            1024 + if frame == 0 { 256 * 128 * 4 } else { 0 }
        );
    }
    r.render_to_image(&c, &t).unwrap();
    assert_eq!(r.stats().texture_upload_bytes, 0);
    r.destroy_dynamic_texture(&mut t, dynamic).unwrap();
    assert!(r.render_to_image(&c, &t).is_err());
}
