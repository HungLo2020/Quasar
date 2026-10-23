use super::*;
#[test]
#[ignore = "requires Vulkan"]
fn composed_pixels_preserve_world_and_share_texture_with_depth() {
    let mut gal = VulkanicGal::create(BackendChoice::Vulkan, "composition pixels").unwrap();
    let mut demo = Demo::new(&mut gal).unwrap();
    let mut target = Offscreen::new(&mut gal, 320, 240).unwrap();
    let world = target.image(&mut gal, &mut demo, 0.5, false).unwrap();
    let image = target.image(&mut gal, &mut demo, 0.5, true).unwrap();
    let pixel =
        |data: &[u8], x: usize, y: usize| data[(y * 320 + x) * 4..(y * 320 + x) * 4 + 4].to_vec();
    assert_eq!(
        pixel(&world, 160, 145),
        pixel(&image, 160, 145),
        "HUD must preserve cube pixels"
    );
    assert_ne!(
        pixel(&world, 160, 145),
        pixel(&world, 310, 230),
        "cube must actually draw"
    );
    assert_ne!(
        pixel(&world, 30, 30),
        pixel(&image, 30, 30),
        "HUD must draw"
    );
    assert_eq!(
        pixel(&image, 233, 49),
        vec![225, 170, 75, 255],
        "HUD samples shared GPU image"
    );
    demo.release(&mut gal).unwrap();
    assert!(
        demo.two_d
            .register_texture(&mut gal, &mut demo.textures, target.view)
            .is_err(),
        "released state must not allocate shared-image resources"
    );
    target.release(&mut gal).unwrap();
    gal.retire_through(gal.latest_submission_id()).unwrap();
    assert_eq!(
        gal.metrics().resource_creates,
        gal.metrics().resource_destroys,
        "all renderer and target resources must retire"
    );
}
#[test]
#[ignore = "requires Vulkan"]
fn prepared_abandonment_clones_failure_and_interleaved_uploads_are_safe() {
    let mut gal = VulkanicGal::create(BackendChoice::Vulkan, "prepared lifetime").unwrap();
    let mut demo = Demo::new(&mut gal).unwrap();
    let target = Offscreen::new(&mut gal, 320, 240).unwrap();
    let first = demo.commands(&mut gal, target.target, 0., true).unwrap();
    let clone = first.clone();
    assert!(demo.two_d.release(&mut gal).is_err());
    assert!(demo.mesh.release(&mut gal).is_err());
    gal.retire_through(gal.latest_submission_id()).unwrap(); // Upload completion cannot free an unsubmitted slot.
    let second = demo.commands(&mut gal, target.target, 1., true).unwrap();
    let third = demo.commands(&mut gal, target.target, 2., true).unwrap();
    assert!(demo.commands(&mut gal, target.target, 3., true).is_err());
    drop(first);
    drop(second);
    drop(third);
    assert!(
        demo.mesh.release(&mut gal).is_err(),
        "cloned commands retain mesh"
    );
    drop(clone);
    let mut commands = demo.commands(&mut gal, target.target, 0., true).unwrap();
    commands.push(list(
        "invalid",
        vec![CommandOp::Draw {
            vertices: 3,
            instances: 1,
        }],
    ));
    assert!(
        gal.submit(SubmissionBatch {
            label: "intentional rejection".into(),
            command_lists: commands
        })
        .is_err()
    );
    // Rejected commands release reservations; a new valid frame still succeeds.
    let mut target = target;
    target.image(&mut gal, &mut demo, 0., true).unwrap();
    demo.release(&mut gal).unwrap();
    target.release(&mut gal).unwrap();
    gal.retire_through(gal.latest_submission_id()).unwrap();
    assert_eq!(
        gal.metrics().resource_creates,
        gal.metrics().resource_destroys
    );
}
#[test]
#[ignore = "requires Vulkan"]
fn many_outstanding_frames_reuse_bounded_slots_and_survive_target_replacement() {
    let mut gal = VulkanicGal::create(BackendChoice::Vulkan, "overlap and targets").unwrap();
    let mut demo = Demo::new(&mut gal).unwrap();
    for size in [(320, 240), (400, 300), (128, 128)] {
        let target = Offscreen::new(&mut gal, size.0, size.1).unwrap();
        for i in 0..16 {
            let (token, _) = submit_explicit(&mut gal, |gal| {
                let mut commands = vec![list(
                    "color-ready",
                    vec![barrier(
                        target.color,
                        if i == 0 {
                            TextureUsageState::Undefined
                        } else {
                            TextureUsageState::ColorAttachment
                        },
                        TextureUsageState::ColorAttachment,
                    )],
                )];
                commands.extend(demo.commands(gal, target.target, i as f32 * 0.1, true)?);
                Ok(commands)
            })
            .unwrap();
            if i == 15 {
                gal.retire_through(token.submission).unwrap();
            }
        }
        demo.release_targets(&mut gal).unwrap();
        target.release(&mut gal).unwrap();
        gal.retire_completed().unwrap();
    }
    demo.release(&mut gal).unwrap();
    gal.retire_through(gal.latest_submission_id()).unwrap();
    assert_eq!(
        gal.metrics().resource_creates,
        gal.metrics().resource_destroys
    );
}

#[test]
#[ignore = "requires Vulkan"]
fn extracted_frame_transaction_closes_scopes_on_error_and_panic() {
    let mut gal = VulkanicGal::create(BackendChoice::Vulkan, "scope unwind").unwrap();
    assert!(
        quasar_frame::record_and_submit(&mut gal, |_| Err(GalError::invalid_argument(
            "failed preparation"
        )))
        .is_err()
    );
    assert_eq!(gal.command_recording_deferred_destroy_count(), 0);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = quasar_frame::record_and_submit(&mut gal, |_| panic!("abandoned frame"));
    }));
    assert!(result.is_err());
    assert!(
        gal.finish_command_recording().is_err(),
        "unwind must close the outer scope"
    );
    let mut demo = Demo::new(&mut gal).unwrap();
    let target = Offscreen::new(&mut gal, 64, 64).unwrap();
    let (token, _) = quasar_frame::record_and_submit(&mut gal, |gal| {
        let mut commands = vec![list(
            "initial-color",
            vec![barrier(
                target.color,
                TextureUsageState::Undefined,
                TextureUsageState::ColorAttachment,
            )],
        )];
        commands.extend(demo.commands(gal, target.target, 0., true)?);
        Ok(commands)
    })
    .unwrap();
    gal.retire_through(token.submission).unwrap();
    demo.release(&mut gal).unwrap();
    target.release(&mut gal).unwrap();
    gal.retire_through(gal.latest_submission_id()).unwrap();
    assert_eq!(
        gal.metrics().resource_creates,
        gal.metrics().resource_destroys
    );
}

#[test]
#[ignore = "requires Vulkan"]
fn depth_test_rejects_far_geometry_drawn_after_near_geometry() {
    let mut gal = VulkanicGal::create(BackendChoice::Vulkan, "depth ordering").unwrap();
    let mut demo = Demo::new(&mut gal).unwrap();
    let target = Offscreen::new(&mut gal, 64, 64).unwrap();
    let vertices = [
        Vertex {
            position: [-0.8, -0.8, 0.],
            normal: [0., 0., 1.],
            uv: [0.05, 0.05],
        },
        Vertex {
            position: [0.8, -0.8, 0.],
            normal: [0., 0., 1.],
            uv: [0.05, 0.05],
        },
        Vertex {
            position: [0., 0.8, 0.],
            normal: [0., 0., 1.],
            uv: [0.05, 0.05],
        },
    ];
    let mut mesh = Mesh::upload(&mut gal, &vertices, &[0, 1, 2]).unwrap();
    let material = Material::textured(demo.image_view);
    let near = Material {
        tint: [1., 0., 0., 1.],
        ..material
    };
    let far = Material {
        tint: [0., 0., 1., 1.],
        ..material
    };
    let prepared = demo
        .three_d
        .prepare(
            &mut gal,
            target.target,
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
        )
        .unwrap();
    let token = gal
        .submit(SubmissionBatch {
            label: "depth-proof".into(),
            command_lists: vec![
                list(
                    "initial-color",
                    vec![barrier(
                        target.color,
                        TextureUsageState::Undefined,
                        TextureUsageState::ColorAttachment,
                    )],
                ),
                prepared.commands,
                list(
                    "readback",
                    vec![
                        barrier(
                            target.color,
                            TextureUsageState::ColorAttachment,
                            TextureUsageState::TransferSrc,
                        ),
                        CommandOp::CopyTextureToBuffer(copy(
                            target.readback,
                            target.color,
                            target.size,
                        )),
                        barrier(
                            target.readback,
                            TextureUsageState::TransferDst,
                            TextureUsageState::TransferSrc,
                        ),
                        CommandOp::HostReadBuffer {
                            buffer: target.readback,
                            offset: 0,
                            size: 64 * 64 * 4,
                        },
                    ],
                ),
            ],
        })
        .unwrap();
    gal.retire_through(token.submission).unwrap();
    let read = gal
        .completed_host_reads()
        .into_iter()
        .find(|r| r.buffer == target.readback)
        .unwrap();
    assert_eq!(
        &read.bytes[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 4],
        &[225, 0, 0, 255],
        "far blue draw must fail depth against near red draw"
    );
    demo.three_d.clear_bindings(&mut gal).unwrap();
    mesh.release(&mut gal).unwrap();
    demo.release(&mut gal).unwrap();
    target.release(&mut gal).unwrap();
    gal.retire_through(gal.latest_submission_id()).unwrap();
    assert_eq!(
        gal.metrics().resource_creates,
        gal.metrics().resource_destroys
    );
}

#[test]
#[ignore = "requires Vulkan"]
fn prepared_frames_keep_their_pixels_when_submitted_out_of_preparation_order() {
    let mut gal = VulkanicGal::create(BackendChoice::Vulkan, "out-of-order preparation").unwrap();
    let mut demo = Demo::new(&mut gal).unwrap();
    let mut targets = [
        Offscreen::new(&mut gal, 320, 240).unwrap(),
        Offscreen::new(&mut gal, 320, 240).unwrap(),
        Offscreen::new(&mut gal, 320, 240).unwrap(),
    ];
    let angles = [0., 0.7, 1.4];
    let mut prepared: Vec<Option<Vec<CommandList>>> = targets
        .iter()
        .zip(angles)
        .map(|(t, a)| Some(demo.commands(&mut gal, t.target, a, true).unwrap()))
        .collect();
    gal.retire_through(gal.latest_submission_id()).unwrap();
    for i in [2, 0, 1] {
        let t = &targets[i];
        let mut lists = vec![list(
            "first-color",
            vec![barrier(
                t.color,
                TextureUsageState::Undefined,
                TextureUsageState::ColorAttachment,
            )],
        )];
        lists.extend(prepared[i].take().unwrap());
        lists.push(list(
            "read-pending",
            vec![
                barrier(
                    t.color,
                    TextureUsageState::ColorAttachment,
                    TextureUsageState::TransferSrc,
                ),
                CommandOp::CopyTextureToBuffer(copy(t.readback, t.color, t.size)),
                barrier(
                    t.readback,
                    TextureUsageState::TransferDst,
                    TextureUsageState::TransferSrc,
                ),
                CommandOp::HostReadBuffer {
                    buffer: t.readback,
                    offset: 0,
                    size: 320 * 240 * 4,
                },
            ],
        ));
        let token = gal
            .submit(SubmissionBatch {
                label: "pending-frame".into(),
                command_lists: lists,
            })
            .unwrap();
        gal.retire_through(token.submission).unwrap();
        let actual = gal
            .completed_host_reads()
            .into_iter()
            .find(|r| r.buffer == t.readback && r.submission == token.submission)
            .unwrap()
            .bytes;
        targets[i].initialized = true;
        let expected = targets[i]
            .image(&mut gal, &mut demo, angles[i], true)
            .unwrap();
        assert_eq!(actual, expected, "prepared frame {i} was overwritten");
    }
    demo.release(&mut gal).unwrap();
    for target in targets {
        target.release(&mut gal).unwrap();
    }
    gal.retire_through(gal.latest_submission_id()).unwrap();
    assert_eq!(
        gal.metrics().resource_creates,
        gal.metrics().resource_destroys
    );
}

#[test]
#[ignore = "requires Vulkan"]
fn shared_image_registration_rejects_a_3d_view_even_at_depth_one() {
    let mut gal = VulkanicGal::create(BackendChoice::Vulkan, "shared image dimension").unwrap();
    let mut state = RenderState2D::new(&mut gal).unwrap();
    let mut textures = Textures::default();
    let image = gal
        .create_texture(TextureDesc {
            label: "depth-one-volume".into(),
            dimension: TextureDimension::D3,
            format: TextureFormat::Rgba8Unorm,
            extent: extent(8, 8),
            mip_levels: 1,
            array_layers: 1,
            usages: vec![TextureUsage::Sampled],
        })
        .unwrap();
    let view = gal
        .create_texture_view(TextureViewDesc {
            label: "volume-view".into(),
            texture: image,
            format: TextureFormat::Rgba8Unorm,
            base_mip: 0,
            mip_count: 1,
            base_layer: 0,
            layer_count: 1,
        })
        .unwrap();
    let before = gal.metrics().resource_creates;
    assert!(
        state
            .register_texture(&mut gal, &mut textures, view)
            .is_err()
    );
    assert_eq!(
        before,
        gal.metrics().resource_creates,
        "reject before making a sampler2D view"
    );
    state.release(&mut gal).unwrap();
    gal.retire(view).unwrap();
    gal.retire(image).unwrap();
    assert_eq!(
        gal.metrics().resource_creates,
        gal.metrics().resource_destroys
    );
}
