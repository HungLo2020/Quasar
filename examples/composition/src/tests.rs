use super::*;
use quasar_frame::{error, read_rgba};
use vulkanic_gal::*;
fn pixel(data: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    data[(y * width + x) * 4..(y * width + x) * 4 + 4]
        .try_into()
        .unwrap()
}
#[test]
#[ignore = "native GPU composition"]
fn composed_pixels_preserve_world_and_share_texture_with_depth() {
    let gpu = Gpu::headless().unwrap();
    let mut demo = Demo::new(&gpu).unwrap();
    let mut target = Offscreen::new(&gpu, 320, 240).unwrap();
    let world = target.image(&gpu, &mut demo, 0.5, false).unwrap();
    let image = target.image(&gpu, &mut demo, 0.5, true).unwrap();
    assert_eq!(pixel(&world, 320, 160, 145), pixel(&image, 320, 160, 145));
    assert_ne!(pixel(&world, 320, 160, 145), pixel(&world, 320, 310, 230));
    assert_ne!(pixel(&world, 320, 30, 30), pixel(&image, 320, 30, 30));
    assert_eq!(pixel(&image, 320, 233, 49), [225, 170, 75, 255]);
    demo.release(&gpu).unwrap();
    assert!(
        demo.two_d
            .register_texture(&gpu, &mut demo.textures, &target.target.color)
            .is_err()
    );
    drop(demo);
    drop(target);
    gpu.wait().unwrap();
    let h = gpu.instance.generate_report().unwrap().hub;
    assert_eq!(
        h.buffers.num_kept_from_user
            + h.textures.num_kept_from_user
            + h.texture_views.num_kept_from_user,
        0
    );
}
#[test]
#[ignore = "native GPU framebuffer composition"]
fn dynamic_software_framebuffer_composes_with_world_and_hud() {
    let gpu = Gpu::headless().unwrap();
    let mut demo = Demo::new(&gpu).unwrap();
    let target = Offscreen::new(&gpu, 320, 240).unwrap();
    let dynamic = demo
        .two_d
        .create_dynamic_texture(&gpu, &mut demo.textures, 32, 32)
        .unwrap();
    let color = [20, 220, 90, 255];
    demo.two_d
        .update_dynamic_texture(dynamic, &color.repeat(1024))
        .unwrap();
    submit_explicit(&gpu, |gpu| {
        let mut commands = demo.commands(gpu, &target.target, 0.5, true)?;
        let mut overlay = Canvas::new(320, 240).unwrap();
        overlay.sprite(Sprite::new(dynamic.id(), Rect::new(8., 8., 32., 32.)));
        commands.push(
            demo.two_d
                .prepare(
                    gpu,
                    &overlay,
                    &demo.textures,
                    &target.target,
                    DisplayOptions::default(),
                    quasar_render_2d::LoadOp::Load,
                )?
                .commands,
        );
        Ok(commands)
    })
    .unwrap();
    let image = read_rgba(&gpu, target.texture()).unwrap();
    assert_eq!(pixel(&image, 320, 20, 20), color);
    assert_ne!(pixel(&image, 320, 160, 145), color);
}
#[test]
#[ignore = "native GPU ownership, abandonment and reverse submission"]
fn prepared_frames_keep_their_pixels_when_submitted_out_of_preparation_order() {
    let gpu = Gpu::headless().unwrap();
    let mut demo = Demo::new(&gpu).unwrap();
    let mut target = Offscreen::new(&gpu, 160, 120).unwrap();
    let first = target.image(&gpu, &mut demo, 0., false).unwrap();
    let second = target.image(&gpu, &mut demo, 1., false).unwrap();
    assert_ne!(first, second);
    let a = Offscreen::new(&gpu, 160, 120).unwrap();
    let b = Offscreen::new(&gpu, 160, 120).unwrap();
    let ca = demo.commands(&gpu, &a.target, 0., false).unwrap();
    let cb = demo.commands(&gpu, &b.target, 1., false).unwrap();
    for angle in 0..12 {
        drop(
            demo.commands(&gpu, &target.target, angle as f32, true)
                .unwrap(),
        );
    }
    // Dropping all application resource owners leaves encoded work valid.
    drop(demo);
    gpu.queue.submit(cb);
    gpu.queue.submit(ca);
    assert_eq!(read_rgba(&gpu, a.texture()).unwrap(), first);
    assert_eq!(read_rgba(&gpu, b.texture()).unwrap(), second);
}
#[test]
#[ignore = "native GPU failed preparation transaction"]
fn extracted_frame_transaction_recovers_after_error_and_panic() {
    let gpu = Gpu::headless().unwrap();
    let target = OffscreenTarget::new(&gpu, 32, 32).unwrap();
    assert!(
        target
            .render(&gpu, |_, _| Err(error("intentional")))
            .is_err()
    );
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = target.render(&gpu, |_, _| panic!("intentional"));
    }));
    assert!(panic.is_err());
    let mut state = RenderState2D::new(&gpu).unwrap();
    let mut c = Canvas::new(32, 32).unwrap();
    c.clear = quasar_render_2d::Color([5, 10, 15, 255]);
    let t = Textures::default();
    assert_eq!(
        target
            .render(&gpu, |gpu, target| Ok(vec![
                state
                    .prepare(
                        gpu,
                        &c,
                        &t,
                        target,
                        DisplayOptions::default(),
                        quasar_render_2d::LoadOp::Clear
                    )?
                    .commands
            ]))
            .unwrap()
            .rgba,
        [5, 10, 15, 255].repeat(1024)
    );
}
#[test]
#[ignore = "native GPU depth ordering"]
fn depth_test_rejects_far_geometry_drawn_after_near_geometry() {
    let gpu = Gpu::headless().unwrap();
    let mut demo = Demo::new(&gpu).unwrap();
    let target = OffscreenTarget::new(&gpu, 64, 64).unwrap();
    let v = [[-0.8, -0.8, 0.], [0.8, -0.8, 0.], [0., 0.8, 0.]].map(|position| Vertex {
        position,
        normal: [0., 0., 1.],
        uv: [0.05; 2],
    });
    let mesh = Mesh::upload(&gpu, &v, &[0, 1, 2]).unwrap();
    let material = Material::textured(&demo.image_view);
    let near = Material {
        tint: [1., 0., 0., 1.],
        ..material.clone()
    };
    let far = Material {
        tint: [0., 0., 1., 1.],
        ..material
    };
    let image = target
        .render(&gpu, |gpu, target| {
            Ok(vec![
                demo.three_d
                    .prepare(
                        gpu,
                        target,
                        Camera3D {
                            view_projection: Mat4::IDENTITY,
                            light_direction: [0., 0., 1.],
                        },
                        &[
                            Draw3D {
                                mesh: &mesh,
                                model: Mat4::translation(0., 0., 0.2),
                                material: near,
                            },
                            Draw3D {
                                mesh: &mesh,
                                model: Mat4::translation(0., 0., 0.8),
                                material: far,
                            },
                        ],
                        PassOptions::default(),
                    )?
                    .commands,
            ])
        })
        .unwrap();
    assert_eq!(pixel(&image.rgba, 64, 32, 32), [225, 0, 0, 255]);
}
#[test]
#[ignore = "native GPU view dimension validation"]
fn shared_image_registration_rejects_a_3d_view_even_at_depth_one() {
    let gpu = Gpu::headless().unwrap();
    let mut state = RenderState2D::new(&gpu).unwrap();
    let image = gpu.device.create_texture(&TextureDescriptor {
        label: None,
        size: extent(8, 8),
        dimension: TextureDimension::D3,
        format: TextureFormat::Rgba8Unorm,
        mip_level_count: 1,
        sample_count: 1,
        usage: TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = image.create_view(&Default::default());
    assert!(
        state
            .register_texture(&gpu, &mut Textures::default(), &view)
            .is_err()
    );
}
#[test]
#[ignore = "native GPU preloaded tilemap integration"]
fn tilemap_uses_shared_native_atlas_without_png_or_duplicate_uploads() {
    use quasar_render_2d::{Color, Filter, PixelState};
    use quasar_tilemap::{
        Map, Runtime, TileEdit,
        render::{LoadedTilesets, View},
    };
    let gpu = Gpu::headless().unwrap();
    let mut demo = Demo::new(&gpu).unwrap();
    let target = OffscreenTarget::new(&gpu, 8, 8).unwrap();
    let map:Map=serde_json::from_value(serde_json::json!({"quasar_version":1,"tile_width":4,"tile_height":4,"width":2,"height":2,"tilesets":[{"first_gid":1,"name":"preloaded","image":"never-open-this.png","image_width":8,"image_height":8,"tile_width":4,"tile_height":4,"columns":2,"tile_count":4}],"layers":[{"id":1,"name":"tiles","content":{"kind":"tiles","chunks":[{"x":0,"y":0,"width":2,"height":2,"cells":[1,2,3,4]}]}}]})).unwrap();
    let loaded =
        LoadedTilesets::from_textures(&map, &demo.textures, |_| Ok(demo.shared_id)).unwrap();
    let mut runtime = Runtime::new(map).unwrap();
    runtime
        .set_tiles(&[TileEdit {
            layer: 1,
            x: 0,
            y: 0,
            gid: 1,
        }])
        .unwrap();
    let mut c = Canvas::new(8, 8).unwrap();
    c.clear = Color::BLACK;
    loaded
        .draw_layer(
            &runtime,
            1,
            &mut c,
            View {
                x: 0.,
                y: 0.,
                width: 8,
                height: 8,
                zoom: 1.,
            },
            0,
            Filter::Nearest,
        )
        .unwrap();
    c.set_pixel_state(PixelState {
        reference: 4,
        write_mask: 4,
        color_write: false,
        alpha_min: 1,
        ..Default::default()
    });
    c.sprite(Sprite::new(demo.shared_id, Rect::new(0., 0., 8., 8.)));
    let p = demo
        .two_d
        .prepare(
            &gpu,
            &c,
            &demo.textures,
            target.target(),
            DisplayOptions::default(),
            quasar_render_2d::LoadOp::Clear,
        )
        .unwrap();
    assert_eq!(p.stats.texture_upload_bytes, 0);
    gpu.queue.submit([p.commands]);
    let image = read_rgba(&gpu, &target.texture).unwrap();
    for y in 0..8 {
        for x in 0..8 {
            assert_eq!(
                pixel(&image, 8, x, y),
                if (x / 2 + y / 2) % 2 == 0 {
                    [225, 170, 75, 255]
                } else {
                    [35, 135, 185, 255]
                }
            );
        }
    }
}
#[test]
#[ignore = "native GPU stencil overlay with caller depth"]
fn masked_overlay_composes_over_loaded_3d_world() {
    use quasar_render_2d::PixelState;
    let gpu = Gpu::headless().unwrap();
    let mut demo = Demo::new(&gpu).unwrap();
    let mut target = Offscreen::new(&gpu, 320, 240).unwrap();
    let world = target.image(&gpu, &mut demo, 0.5, false).unwrap();
    let mut c = Canvas::new(320, 240).unwrap();
    c.set_pixel_state(PixelState {
        reference: 8,
        write_mask: 8,
        color_write: false,
        alpha_min: 1,
        ..Default::default()
    });
    c.rect(Rect::new(12., 18., 16., 12.), [255; 4]);
    c.set_pixel_state(PixelState {
        reference: 8,
        read_mask: 8,
        ..Default::default()
    });
    c.rect(Rect::new(0., 0., 320., 240.), [120, 40, 220, 255]);
    let mut commands = demo.commands(&gpu, &target.target, 0.5, false).unwrap();
    commands.push(
        demo.two_d
            .prepare(
                &gpu,
                &c,
                &demo.textures,
                &target.target,
                DisplayOptions::default(),
                quasar_render_2d::LoadOp::Load,
            )
            .unwrap()
            .commands,
    );
    gpu.queue.submit(commands);
    let actual = read_rgba(&gpu, target.texture()).unwrap();
    for y in 0..240 {
        for x in 0..320 {
            assert_eq!(
                pixel(&actual, 320, x, y),
                if (12..28).contains(&x) && (18..30).contains(&y) {
                    [120, 40, 220, 255]
                } else {
                    pixel(&world, 320, x, y)
                }
            );
        }
    }
}

#[test]
#[ignore = "native GPU outstanding buffers and replacement targets"]
fn many_outstanding_frames_survive_target_replacement() {
    let gpu = Gpu::headless().unwrap();
    let mut demo = Demo::new(&gpu).unwrap();
    let mut pending = Vec::new();
    for (width, height) in [(128, 128), (320, 240), (400, 300)] {
        let target = Offscreen::new(&gpu, width, height).unwrap();
        for i in 0..16 {
            pending.extend(
                demo.commands(&gpu, &target.target, i as f32 * 0.1, true)
                    .unwrap(),
            );
        }
        drop(target);
    }
    drop(demo);
    gpu.queue.submit(pending);
    gpu.wait().unwrap();
    let h = gpu.instance.generate_report().unwrap().hub;
    assert_eq!(
        h.buffers.num_kept_from_user
            + h.textures.num_kept_from_user
            + h.texture_views.num_kept_from_user,
        0
    );
}
