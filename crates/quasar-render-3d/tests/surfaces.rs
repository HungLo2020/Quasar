use quasar_frame::Gpu;

mod support;
use quasar_frame::OffscreenTarget;
use quasar_render_3d::*;
fn plane(gal: &Gpu) -> Mesh {
    let vertices = [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]]
        .into_iter()
        .zip([[0., 1.], [1., 1.], [1., 0.], [0., 0.]])
        .map(|(position, uv)| Vertex {
            position,
            normal: [0., 0., 1.],
            uv,
        })
        .collect::<Vec<_>>();
    Mesh::upload(gal, &vertices, &[0, 1, 2, 0, 2, 3]).unwrap()
}
fn camera() -> LitCamera3D {
    LitCamera3D {
        position: [0., 0., 4.],
        view_projection: Mat4::orthographic(-2., 2., -2., 2., 0.1, 10.).unwrap()
            * Mat4::translation(0., 0., -4.),
    }
}
fn pixel(image: &[u8], x: usize, y: usize) -> [u8; 4] {
    image[(y * 128 + x) * 4..(y * 128 + x) * 4 + 4]
        .try_into()
        .unwrap()
}
#[test]
#[ignore = "requires Vulkan"]
fn directional_and_spot_shadows_and_normal_maps_change_pixels() {
    surface_suite(SubmissionConfig::default());
}

#[test]
#[ignore = "requires GPU"]
fn instanced_surface_shader_preserves_all_lighting_and_material_cases() {
    surface_suite(SubmissionConfig {
        instancing: true,
        ..SubmissionConfig::default()
    });
}

fn surface_suite(submission: SubmissionConfig) {
    let mut gal = Gpu::headless().unwrap();
    let mut renderer = SurfaceRenderState3D::with_submission(&gal, 4, 2, submission).unwrap();
    let mut shadows = ShadowRenderState3D::with_submission(
        &gal,
        ShadowConfig {
            resolution: 128,
            ..ShadowConfig::default()
        },
        submission,
    )
    .unwrap();
    let mut mesh = plane(&gal);
    let (_texture, view) = support::upload_color(&gal, [255; 4]).unwrap();
    let (_normal_texture, normal_view) = support::upload_color(&gal, [255, 128, 128, 255]).unwrap();
    let target = OffscreenTarget::new(&gal, 128, 128).unwrap();
    let mut lit = LitMaterial::textured(&view);
    lit.shading = ShadingModel::Diffuse;
    let mut material = SurfaceMaterial::from(lit.clone());
    let lights = [Light::Directional {
        direction: [-0.5, 0., -1.],
        color: [1.; 3],
        intensity: 1.,
    }];
    let mut caster = ShadowCaster {
        mesh: &mesh,
        model: Mat4::translation(0., 0.4, 1.) * Mat4::scale(0.3, 0.3, 1.),
        texture_view: view.clone(),
        alpha_cutoff: 0.,
        opacity: 1.,
    };
    let light_view = Mat4::orthographic(-2., 2., -2., 2., 0.1, 10.).unwrap()
        * Mat4::look_at([2., 0., 4.], [0., 0., 0.], [0., 1., 0.]).unwrap();
    let receiver_model = std::cell::Cell::new(Mat4::scale(2., 2., 1.));
    let draw = |gal: &mut Gpu,
                renderer: &mut SurfaceRenderState3D,
                shadows: &mut ShadowRenderState3D,
                material: SurfaceMaterial,
                views: &[ShadowView<'_>],
                lights: &[Light]| {
        let prepared = (!views.is_empty()).then(|| shadows.prepare(gal, views).unwrap());
        let result = target
            .render(gal, |gal, target| {
                let world = renderer.prepare(
                    gal,
                    target,
                    camera(),
                    Lighting {
                        ambient: [0.02; 3],
                        lights,
                        ..Lighting::default()
                    },
                    &[SurfaceDraw3D {
                        mesh: &mesh,
                        model: receiver_model.get(),
                        material: material.clone(),
                    }],
                    prepared.as_ref().map(PreparedShadows::binding),
                    PassOptions::default(),
                )?;
                let mut commands: Vec<_> = prepared.into_iter().map(|s| s.commands).collect();
                commands.push(world.commands);
                Ok(commands)
            })
            .unwrap();
        result.rgba
    };
    let plain = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material.clone(),
        &[],
        &lights,
    );
    let shadowed = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material.clone(),
        &[ShadowView {
            light_index: 0,
            view_projection: light_view,
            settings: ShadowSettings {
                normal_bias: 0.,
                filter: ShadowFilter::Hard,
                ..ShadowSettings::default()
            },
            casters: std::slice::from_ref(&caster),
        }],
        &lights,
    );
    let dark: Vec<_> = (0..128)
        .flat_map(|y| (0..128).map(move |x| (x, y)))
        .filter(|(x, y)| pixel(&plain, *x, *y)[0] > 100 && pixel(&shadowed, *x, *y)[0] < 30)
        .collect();
    assert!(
        dark.len() > 100,
        "expected cast shadow; changed pixels {}",
        plain.iter().zip(&shadowed).filter(|(a, b)| a != b).count()
    );
    // Offset caster above center must shadow above center, not reflect the Y coordinate.
    assert!(
        dark.iter().map(|p| p.1).sum::<usize>() / dark.len() < 64,
        "shadow projection Y orientation"
    );
    let shadow_sample = dark[dark.len() / 2];
    let (_cutout_texture, cutout_view) = support::upload_color(&gal, [255, 255, 255, 0]).unwrap();
    let mut filtered =
        |settings: ShadowSettings, material: SurfaceMaterial, caster: &ShadowCaster<'_>| {
            draw(
                &mut gal,
                &mut renderer,
                &mut shadows,
                material.clone(),
                &[ShadowView {
                    light_index: 0,
                    view_projection: light_view,
                    settings,
                    casters: std::slice::from_ref(caster),
                }],
                &lights,
            )
        };
    for filter in [ShadowFilter::Pcf3, ShadowFilter::Pcf5] {
        let soft = filtered(
            ShadowSettings {
                normal_bias: 0.,
                filter,
                ..ShadowSettings::default()
            },
            material.clone(),
            &caster,
        );
        let intermediate = |image: &[u8]| {
            image
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[0] > 30 && p[0] < 200)
                .count()
        };
        assert!(
            intermediate(&soft) > intermediate(&shadowed),
            "PCF must smooth shadow edges"
        );
    }
    let half = filtered(
        ShadowSettings {
            normal_bias: 0.,
            strength: 0.5,
            filter: ShadowFilter::Hard,
            ..ShadowSettings::default()
        },
        material.clone(),
        &caster,
    );
    let value = pixel(&half, shadow_sample.0, shadow_sample.1)[0];
    assert!(
        value > 40 && value < 200,
        "partial shadow strength: {value}"
    );
    material.receives_shadows = false;
    assert_eq!(
        plain,
        filtered(ShadowSettings::default(), material.clone(), &caster)
    );
    material.receives_shadows = true;
    // Cutouts use the albedo alpha and material opacity in the depth-only pass.
    caster.alpha_cutoff = 0.5;
    caster.opacity = 0.1;
    assert_eq!(
        plain,
        filtered(ShadowSettings::default(), material.clone(), &caster)
    );
    caster.opacity = 1.;
    caster.texture_view = cutout_view;
    assert_eq!(
        plain,
        filtered(ShadowSettings::default(), material.clone(), &caster)
    );
    caster.texture_view = view;
    caster.alpha_cutoff = 0.;
    caster.opacity = 1.;
    caster.model = Mat4::translation(0., -0.6, 1.) * Mat4::scale(0.3, 0.3, 1.);
    let moved = filtered(
        ShadowSettings {
            normal_bias: 0.,
            filter: ShadowFilter::Hard,
            ..ShadowSettings::default()
        },
        material.clone(),
        &caster,
    );
    assert_ne!(moved, shadowed, "moving casters must update maps");
    assert!(pixel(&moved, shadow_sample.0, shadow_sample.1)[0] > 100);
    caster.model = Mat4::translation(0., 0.4, 1.) * Mat4::scale(0.3, 0.3, 1.);
    // Shadow only the associated light; a second array layer must be selected independently.
    let colors = [
        Light::Directional {
            direction: [-0.5, 0., -1.],
            color: [1., 0., 0.],
            intensity: 1.,
        },
        Light::Directional {
            direction: [-0.5, 0., -1.],
            color: [0., 1., 0.],
            intensity: 1.,
        },
    ];
    let settings = ShadowSettings {
        normal_bias: 0.,
        filter: ShadowFilter::Hard,
        ..ShadowSettings::default()
    };
    let mixed = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material.clone(),
        &[
            ShadowView {
                light_index: 0,
                view_projection: light_view,
                settings,
                casters: &[],
            },
            ShadowView {
                light_index: 1,
                view_projection: light_view,
                settings,
                casters: std::slice::from_ref(&caster),
            },
        ],
        &colors,
    );
    let c = pixel(&mixed, shadow_sample.0, shadow_sample.1);
    assert!(
        c[0] > 200 && c[1] < 30,
        "independent light/layer visibility: {c:?}"
    );
    material.lit.emissive = [0.1; 3];
    let emission = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material.clone(),
        &[ShadowView {
            light_index: 0,
            view_projection: light_view,
            settings,
            casters: std::slice::from_ref(&caster),
        }],
        &lights,
    );
    assert!(
        (pixel(&emission, shadow_sample.0, shadow_sample.1)[0] as i32
            - pixel(&shadowed, shadow_sample.0, shadow_sample.1)[0] as i32
            - 26)
            .abs()
            <= 1,
        "emission must survive shadowing"
    );
    material.lit.emissive = [0.; 3];
    material.normal_map = Some(NormalMap::new(&normal_view));
    let mapped = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material.clone(),
        &[],
        &lights,
    );
    assert_ne!(pixel(&plain, 64, 64), pixel(&mapped, 64, 64));
    material.normal_map.as_mut().unwrap().strength = 0.;
    assert_eq!(
        plain,
        draw(
            &mut gal,
            &mut renderer,
            &mut shadows,
            material.clone(),
            &[],
            &lights
        )
    );
    material.normal_map.as_mut().unwrap().strength = 1.;
    material.normal_map.as_mut().unwrap().uv_scale[0] = -1.;
    let mirrored = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material.clone(),
        &[],
        &lights,
    );
    assert!(
        pixel(&mapped, 64, 64)[0] > pixel(&mirrored, 64, 64)[0] + 50,
        "mirrored UV must flip the tangent"
    );
    // A non-uniform world scale preserves the same tangent-space response.
    material.normal_map.as_mut().unwrap().uv_scale[0] = 1.;
    receiver_model.set(Mat4::scale(1.5, 0.75, 0.5));
    let scaled = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material.clone(),
        &[],
        &lights,
    );
    assert_eq!(pixel(&mapped, 64, 64), pixel(&scaled, 64, 64));
    receiver_model.set(Mat4::scale(-2., 2., 1.));
    let reflected = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material.clone(),
        &[],
        &lights,
    );
    assert_eq!(
        pixel(&mirrored, 64, 64),
        pixel(&reflected, 64, 64),
        "mirrored world transform must preserve normal handedness"
    );
    receiver_model.set(Mat4::scale(2., 2., 1.));
    let (_green_texture, green_view) = support::upload_color(&gal, [128, 255, 128, 255]).unwrap();
    material.normal_map = Some(NormalMap::new(&green_view));
    let angled = [Light::Directional {
        direction: [0., 0.5, -1.],
        color: [1.; 3],
        intensity: 1.,
    }];
    let up = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material.clone(),
        &[],
        &angled,
    );
    material.normal_map.as_mut().unwrap().flip_y = true;
    let down = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material.clone(),
        &[],
        &angled,
    );
    assert_ne!(
        pixel(&up, 64, 64),
        pixel(&down, 64, 64),
        "authored Y convention must be selectable"
    );
    material.normal_map = None;
    let spot = [Light::Spot {
        position: [2., 0., 4.],
        direction: [-0.5, 0., -1.],
        color: [1.; 3],
        intensity: 1.,
        range: 12.,
        inner_angle: 0.5,
        outer_angle: 0.75,
        attenuation: Attenuation::Smooth,
    }];
    let vp = Mat4::perspective(1.5, 1., 0.1, 12.).unwrap()
        * Mat4::look_at([2., 0., 4.], [0., 0., 0.], [0., 1., 0.]).unwrap();
    let spot_plain = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material.clone(),
        &[],
        &spot,
    );
    let spot_shadow = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material.clone(),
        &[ShadowView {
            light_index: 0,
            view_projection: vp,
            settings: ShadowSettings {
                normal_bias: 0.,
                ..ShadowSettings::default()
            },
            casters: std::slice::from_ref(&caster),
        }],
        &spot,
    );
    assert!(
        spot_plain
            .iter()
            .zip(&spot_shadow)
            .filter(|(a, b)| **a > 100 && **b < 30)
            .count()
            > 100
    );
    // Enabling a normal map must be independent of earlier material bindings,
    // including a caller deliberately reusing one view for both inputs.
    let (_shared_texture, shared_view) =
        support::upload_pixels(&gal, 2, 1, vec![255, 128, 128, 255, 128, 128, 255, 255]).unwrap();
    material.lit.base.texture_view = shared_view.clone();
    material.normal_map = None;
    draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material.clone(),
        &[],
        &lights,
    );
    material.normal_map = Some(NormalMap::new(&shared_view));
    let reused = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material.clone(),
        &[],
        &lights,
    );
    renderer.clear_bindings(&gal).unwrap();
    let fresh = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material.clone(),
        &[],
        &lights,
    );
    assert_eq!(
        reused, fresh,
        "normal-map filtering must not depend on cache history"
    );
    renderer.release(&gal).unwrap();
    shadows.release(&gal).unwrap();
    mesh.release(&gal).unwrap();
    target.release(&gal).unwrap();

    gal.wait().unwrap();
}

#[test]
#[ignore = "requires Vulkan"]
fn shadow_slots_preserve_reordered_frames_and_consumer_reservations() {
    slot_suite(SubmissionConfig::default());
}

#[test]
#[ignore = "requires GPU"]
fn instanced_slots_preserve_reordered_frames_and_consumer_reservations() {
    slot_suite(SubmissionConfig {
        instancing: true,
        ..SubmissionConfig::default()
    });
}

fn slot_suite(submission: SubmissionConfig) {
    let gal = Gpu::headless().unwrap();
    let mut renderer = SurfaceRenderState3D::with_submission(&gal, 1, 1, submission).unwrap();
    let mut shadows = ShadowRenderState3D::with_submission(
        &gal,
        ShadowConfig {
            resolution: 64,
            max_maps: 1,
            ..ShadowConfig::default()
        },
        submission,
    )
    .unwrap();
    let mut mesh = plane(&gal);
    let (_texture, view) = support::upload_color(&gal, [255; 4]).unwrap();
    let target = OffscreenTarget::new(&gal, 128, 128).unwrap();
    let lights = [Light::Directional {
        direction: [0., 0., -1.],
        color: [1.; 3],
        intensity: 1.,
    }];
    let mut lit = LitMaterial::textured(&view);
    lit.shading = ShadingModel::Diffuse;
    let draws = [SurfaceDraw3D {
        mesh: &mesh,
        model: Mat4::scale(2., 2., 1.),
        material: lit.clone().into(),
    }];
    let casters = [ShadowCaster {
        mesh: &mesh,
        model: Mat4::translation(0., 0., 1.) * Mat4::scale(0.4, 0.4, 1.),
        texture_view: view.clone(),
        alpha_cutoff: 0.,
        opacity: 1.,
    }];
    let mut pending = vec![];
    for strength in [0., 1., 0.5] {
        let maps = shadows
            .prepare(
                &gal,
                &[ShadowView {
                    light_index: 0,
                    view_projection: camera().view_projection,
                    settings: ShadowSettings {
                        strength,
                        normal_bias: 0.,
                        filter: ShadowFilter::Hard,
                        ..ShadowSettings::default()
                    },
                    casters: &casters,
                }],
            )
            .unwrap();
        let receiver = renderer
            .prepare(
                &gal,
                target.target(),
                camera(),
                Lighting {
                    ambient: [0.02; 3],
                    lights: &lights,
                    ..Lighting::default()
                },
                &draws,
                Some(maps.binding()),
                PassOptions::default(),
            )
            .unwrap();
        pending.push((maps, receiver));
    }
    #[allow(
        clippy::mutable_key_type,
        reason = "Native texture-view identity is immutable."
    )]
    let atlas_views: std::collections::HashSet<_> = pending
        .iter()
        .map(|p| p.0.binding().texture_view().clone())
        .collect();
    assert_eq!(
        atlas_views.len(),
        3,
        "outstanding frames need isolated depth arrays"
    );
    assert!(shadows.prepare(&gal, &[]).is_ok());
    shadows.resize(&gal, 128).unwrap();
    // Native commands retain outstanding generations across reconfiguration.
    for (minimum, maximum) in [(100, 170), (0, 30), (240, 255)] {
        let (maps, receiver) = pending.pop().unwrap();
        let image = target
            .render(&gal, |_, _| Ok(vec![maps.commands, receiver.commands]))
            .unwrap();
        let center = pixel(&image.rgba, 64, 64)[0];
        assert!(
            (minimum..=maximum).contains(&center),
            "reordered shadow visibility {center}"
        );
    }
    // Producer submission does not free the atlas while unsubmitted receivers retain it.
    let mut receivers = vec![];
    for _ in 0..3 {
        let maps = shadows.prepare(&gal, &[]).unwrap();
        receivers.push(
            renderer
                .prepare(
                    &gal,
                    target.target(),
                    camera(),
                    Lighting::default(),
                    &draws,
                    Some(maps.binding()),
                    PassOptions::default(),
                )
                .unwrap(),
        );
        gal.queue.submit([maps.commands]);
    }

    assert!(shadows.prepare(&gal, &[]).is_ok());
    shadows.resize(&gal, 128).unwrap();
    receivers.clear();
    // A retained binding also reserves its generation for custom GAL consumers.
    let retained: Vec<_> = (0..3)
        .map(|_| shadows.prepare(&gal, &[]).unwrap().binding().clone())
        .collect();
    assert!(shadows.prepare(&gal, &[]).is_ok());
    drop(retained);
    renderer.clear_bindings(&gal).unwrap();
    shadows.resize(&gal, 128).unwrap();
    // Warm a generation, then reuse it without descriptor/resource churn.
    for _ in 0..2 {
        let maps = shadows.prepare(&gal, &[]).unwrap();
        target
            .render(&gal, |gal, t| {
                let receiver = renderer.prepare(
                    gal,
                    t,
                    camera(),
                    Lighting::default(),
                    &draws,
                    Some(maps.binding()),
                    PassOptions::default(),
                )?;
                Ok(vec![maps.commands, receiver.commands])
            })
            .unwrap();
    }
    for _ in 0..4 {
        let maps = shadows.prepare(&gal, &[]).unwrap();
        target
            .render(&gal, |gal, t| {
                let receiver = renderer
                    .prepare(
                        gal,
                        t,
                        camera(),
                        Lighting::default(),
                        &draws,
                        Some(maps.binding()),
                        PassOptions::default(),
                    )?
                    .commands;
                Ok(vec![maps.commands, receiver])
            })
            .unwrap();
    }
    renderer.release(&gal).unwrap();
    shadows.release(&gal).unwrap();
    shadows.release(&gal).unwrap();
    mesh.release(&gal).unwrap();
    target.release(&gal).unwrap();

    gal.wait().unwrap();
}

#[test]
#[ignore = "requires Vulkan"]
fn shadow_and_surface_validation_rejects_invalid_data_before_recording() {
    let gal = Gpu::headless().unwrap();
    for config in [
        ShadowConfig {
            resolution: 16,
            ..ShadowConfig::default()
        },
        ShadowConfig {
            max_maps: 0,
            ..ShadowConfig::default()
        },
        ShadowConfig {
            max_casters: 0,
            ..ShadowConfig::default()
        },
        ShadowConfig {
            max_depth_bytes: 1,
            ..ShadowConfig::default()
        },
    ] {
        assert!(ShadowRenderState3D::new(&gal, config).is_err());
    }
    // Explicitly supplied arrays obey the same base-level depth attachment contract.
    for (width, height, layers, mips, format, expected) in [
        (
            64,
            64,
            2,
            1,
            vulkanic_gal::TextureFormat::Depth32Float,
            true,
        ),
        (
            64,
            32,
            2,
            1,
            vulkanic_gal::TextureFormat::Depth32Float,
            false,
        ),
        (
            64,
            64,
            1,
            1,
            vulkanic_gal::TextureFormat::Depth32Float,
            false,
        ),
        (
            64,
            64,
            2,
            2,
            vulkanic_gal::TextureFormat::Depth32Float,
            false,
        ),
        (64, 64, 2, 1, vulkanic_gal::TextureFormat::Rgba8Unorm, false),
    ] {
        let texture = gal.device.create_texture(&vulkanic_gal::TextureDescriptor {
            label: Some("external shadow target validation"),
            size: vulkanic_gal::Extent3d {
                width,
                height,
                depth_or_array_layers: layers,
            },
            mip_level_count: mips,
            sample_count: 1,
            dimension: vulkanic_gal::TextureDimension::D2,
            format,
            usage: vulkanic_gal::TextureUsages::RENDER_ATTACHMENT
                | vulkanic_gal::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        assert_eq!(ShadowTarget::from_texture(&texture).is_ok(), expected);
    }
    let mut shadows = ShadowRenderState3D::new(
        &gal,
        ShadowConfig {
            resolution: 64,
            max_maps: 2,
            max_casters: 1,
            ..ShadowConfig::default()
        },
    )
    .unwrap();
    let mut renderer = SurfaceRenderState3D::new(&gal, 1, 1).unwrap();
    let mut mesh = plane(&gal);
    let (_texture, view) = support::upload_color(&gal, [255; 4]).unwrap();
    let target = OffscreenTarget::new(&gal, 128, 128).unwrap();
    let mut map = ShadowView {
        light_index: 0,
        view_projection: camera().view_projection,
        settings: ShadowSettings::default(),
        casters: &[],
    };
    for bad in [f32::NAN, -1., 2.] {
        map.settings.strength = bad;
        assert!(shadows.prepare(&gal, std::slice::from_ref(&map)).is_err());
    }
    map.settings = ShadowSettings::default();
    assert!(
        shadows
            .prepare(&gal, &[ShadowView { ..map }, ShadowView { ..map }])
            .is_err()
    );
    let casters = [
        ShadowCaster {
            mesh: &mesh,
            model: Mat4::IDENTITY,
            texture_view: view.clone(),
            alpha_cutoff: 0.,
            opacity: 1.,
        },
        ShadowCaster {
            mesh: &mesh,
            model: Mat4::IDENTITY,
            texture_view: view.clone(),
            alpha_cutoff: 0.,
            opacity: 1.,
        },
    ];
    assert!(
        shadows
            .prepare(
                &gal,
                &[ShadowView {
                    casters: &casters,
                    ..map
                }]
            )
            .is_err()
    );
    let maps = shadows.prepare(&gal, &[map]).unwrap();
    let mut material = SurfaceMaterial::from(LitMaterial::textured(&view));
    let mut prepare = |material: SurfaceMaterial| {
        renderer.prepare(
            &gal,
            target.target(),
            camera(),
            Lighting::default(),
            &[SurfaceDraw3D {
                mesh: &mesh,
                model: Mat4::IDENTITY,
                material: material.clone(),
            }],
            Some(maps.binding()),
            PassOptions::default(),
        )
    };
    assert!(
        prepare(material.clone()).is_err(),
        "shadow light index must reference supplied directional/spot light"
    );
    material.receives_shadows = false;
    material.normal_map = Some(NormalMap {
        strength: f32::NAN,
        ..NormalMap::new(&view)
    });
    assert!(prepare(material.clone()).is_err());
    material.normal_map = Some(NormalMap {
        uv_scale: [0., 1.],
        ..NormalMap::new(&view)
    });
    assert!(prepare(material.clone()).is_err());
    let mut normals_only = SurfaceRenderState3D::new(&gal, 0, 0).unwrap();
    let mut unshadowed = SurfaceMaterial::from(LitMaterial::textured(&view));
    unshadowed.normal_map = Some(NormalMap::new(&view));
    let prepared = normals_only
        .prepare(
            &gal,
            target.target(),
            camera(),
            Lighting::default(),
            &[SurfaceDraw3D {
                mesh: &mesh,
                model: Mat4::IDENTITY,
                material: unshadowed,
            }],
            None,
            PassOptions::default(),
        )
        .unwrap();
    drop(prepared);
    normals_only.release(&gal).unwrap();

    renderer.release(&gal).unwrap();
    shadows.release(&gal).unwrap();
    mesh.release(&gal).unwrap();
    target.release(&gal).unwrap();

    gal.wait().unwrap();
}

#[cfg(feature = "visibility")]
#[test]
#[ignore = "requires GPU"]
fn instanced_buffer_growth_and_culling_preserve_per_instance_materials() {
    let gal = Gpu::headless().unwrap();
    let mut mesh = plane(&gal);
    let (_texture, view) = support::upload_color(&gal, [255; 4]).unwrap();
    let mut reference = SurfaceRenderState3D::new(&gal, 0, 0).unwrap();
    let mut optimized = SurfaceRenderState3D::with_submission(
        &gal,
        0,
        0,
        SubmissionConfig {
            instancing: true,
            frustum_culling: true,
            ..SubmissionConfig::default()
        },
    )
    .unwrap();
    let target = OffscreenTarget::new(&gal, 128, 128).unwrap();
    let mut material = LitMaterial::textured(&view);
    material.shading = ShadingModel::Diffuse;
    let draws: Vec<_> = (0..20)
        .map(|i| {
            let mut lit = material.clone();
            lit.base.tint = [(i % 3) as f32 / 2., (i % 5) as f32 / 4., 1., 1.];
            SurfaceDraw3D {
                mesh: &mesh,
                model: Mat4::translation(
                    if i < 16 { (i % 4) as f32 - 1.5 } else { 50. },
                    (i / 4) as f32 - 1.5,
                    0.,
                ) * Mat4::scale(0.4, 0.4, 0.4),
                material: lit.clone().into(),
            }
        })
        .collect();
    let lighting = Lighting {
        ambient: [1.; 3],
        ..Lighting::default()
    };
    let mut pending = Vec::new();
    for _ in 0..3 {
        pending.push(
            optimized
                .prepare(
                    &gal,
                    target.target(),
                    camera(),
                    lighting,
                    &draws[..1],
                    None,
                    PassOptions::default(),
                )
                .unwrap(),
        );
    }
    drop(
        optimized
            .prepare(
                &gal,
                target.target(),
                camera(),
                lighting,
                &draws,
                None,
                PassOptions::default(),
            )
            .unwrap(),
    );
    optimized.clear_bindings(&gal).unwrap();
    pending.clear();
    let expected = target
        .render(&gal, |gal, t| {
            Ok(vec![
                reference
                    .prepare(
                        gal,
                        t,
                        camera(),
                        lighting,
                        &draws,
                        None,
                        PassOptions::default(),
                    )?
                    .commands,
            ])
        })
        .unwrap();
    let actual = target
        .render(&gal, |gal, t| {
            Ok(vec![
                optimized
                    .prepare(
                        gal,
                        t,
                        camera(),
                        lighting,
                        &draws,
                        None,
                        PassOptions::default(),
                    )?
                    .commands,
            ])
        })
        .unwrap();
    assert_eq!(actual.rgba, expected.rgba);
    assert_eq!(optimized.work_stats().visible, 16);
    assert_eq!(optimized.work_stats().culled, 4);
    assert_eq!(optimized.work_stats().draws, 1);
    assert_eq!(optimized.work_stats().instance_upload_bytes, 16 * 208);
    optimized.release(&gal).unwrap();
    reference.release(&gal).unwrap();
    mesh.release(&gal).unwrap();
    target.release(&gal).unwrap();

    gal.wait().unwrap();
}

#[cfg(feature = "visibility")]
#[test]
#[ignore = "requires GPU"]
fn camera_rejection_does_not_remove_an_offscreen_shadow_caster() {
    let gal = Gpu::headless().unwrap();
    let submission = SubmissionConfig {
        instancing: true,
        frustum_culling: true,
        ..SubmissionConfig::default()
    };
    let mut receiver = SurfaceRenderState3D::with_submission(&gal, 1, 1, submission).unwrap();
    let mut shadows = ShadowRenderState3D::with_submission(
        &gal,
        ShadowConfig {
            resolution: 128,
            max_maps: 1,
            ..ShadowConfig::default()
        },
        submission,
    )
    .unwrap();
    let mut mesh = plane(&gal);
    let (_texture, view) = support::upload_color(&gal, [255; 4]).unwrap();
    let mut material = LitMaterial::textured(&view);
    material.shading = ShadingModel::Diffuse;
    let draws = [
        SurfaceDraw3D {
            mesh: &mesh,
            model: Mat4::IDENTITY,
            material: material.clone().into(),
        },
        SurfaceDraw3D {
            mesh: &mesh,
            model: Mat4::translation(0.9, 0., 1.) * Mat4::scale(0.15, 0.15, 1.),
            material: material.clone().into(),
        },
    ];
    let caster = [draws[1].shadow_caster()];
    let camera = LitCamera3D {
        view_projection: Mat4::orthographic(-0.5, 0.5, -0.5, 0.5, 0.1, 10.).unwrap()
            * Mat4::translation(0., 0., -4.),
        ..camera()
    };
    let lights = [Light::Directional {
        direction: [-1., 0., -1.],
        color: [1.; 3],
        intensity: 1.,
    }];
    let lighting = Lighting {
        ambient: [0.02; 3],
        lights: &lights,
        ..Lighting::default()
    };
    let target = OffscreenTarget::new(&gal, 128, 128).unwrap();
    let plain = target
        .render(&gal, |gal, t| {
            Ok(vec![
                receiver
                    .prepare(
                        gal,
                        t,
                        camera,
                        lighting,
                        &draws,
                        None,
                        PassOptions::default(),
                    )?
                    .commands,
            ])
        })
        .unwrap();
    let maps = shadows
        .prepare(
            &gal,
            &[ShadowView {
                light_index: 0,
                view_projection: Mat4::orthographic(-2., 2., -2., 2., 0.1, 10.).unwrap()
                    * Mat4::look_at([4., 0., 4.], [0., 0., 0.], [0., 1., 0.]).unwrap(),
                casters: &caster,
                settings: ShadowSettings {
                    normal_bias: 0.,
                    filter: ShadowFilter::Hard,
                    ..ShadowSettings::default()
                },
            }],
        )
        .unwrap();
    let dark = target
        .render(&gal, |gal, t| {
            let world = receiver
                .prepare(
                    gal,
                    t,
                    camera,
                    lighting,
                    &draws,
                    Some(maps.binding()),
                    PassOptions::default(),
                )?
                .commands;
            Ok(vec![maps.commands, world])
        })
        .unwrap();
    assert_eq!(receiver.work_stats().visible, 1);
    assert_eq!(receiver.work_stats().culled, 1);
    assert_eq!(shadows.work_stats().visible, 1);
    assert!(
        pixel(&plain.rgba, 51, 64)[0] > 100 && pixel(&dark.rgba, 51, 64)[0] < 30,
        "offscreen caster must shadow visible receiver"
    );

    receiver.release(&gal).unwrap();
    shadows.release(&gal).unwrap();
    mesh.release(&gal).unwrap();
    target.release(&gal).unwrap();

    gal.wait().unwrap();
}

#[test]
#[ignore = "native reusable shadow target with reordered whole frames"]
fn explicit_shadow_target_reuse_keeps_whole_frame_snapshots_correct() {
    let gpu = Gpu::headless().unwrap();
    let mut shadows = ShadowRenderState3D::new(
        &gpu,
        ShadowConfig {
            resolution: 64,
            max_maps: 1,
            ..Default::default()
        },
    )
    .unwrap();
    let atlas = shadows.create_target(&gpu).unwrap();
    let mut surfaces = SurfaceRenderState3D::new(&gpu, 1, 1).unwrap();
    let mesh = plane(&gpu);
    let (_texture, view) = support::upload_color(&gpu, [255; 4]).unwrap();
    let mut lit = LitMaterial::textured(&view);
    lit.shading = ShadingModel::Diffuse;
    let lights = [Light::Directional {
        direction: [0., 0., -1.],
        color: [1.; 3],
        intensity: 1.,
    }];
    let casters = [ShadowCaster {
        mesh: &mesh,
        model: Mat4::translation(0., 0., 1.) * Mat4::scale(0.4, 0.4, 1.),
        texture_view: view,
        alpha_cutoff: 0.,
        opacity: 1.,
    }];
    let target = OffscreenTarget::new(&gpu, 128, 128).unwrap();
    let mut frames = Vec::new();
    for strength in [0., 1.] {
        let maps = shadows
            .prepare_into(
                &gpu,
                &[ShadowView {
                    light_index: 0,
                    view_projection: camera().view_projection,
                    settings: ShadowSettings {
                        strength,
                        normal_bias: 0.,
                        filter: ShadowFilter::Hard,
                        ..Default::default()
                    },
                    casters: &casters,
                }],
                &atlas,
            )
            .unwrap();
        let receiver = surfaces
            .prepare(
                &gpu,
                target.target(),
                camera(),
                Lighting {
                    ambient: [0.02; 3],
                    lights: &lights,
                    ..Default::default()
                },
                &[SurfaceDraw3D {
                    mesh: &mesh,
                    model: Mat4::scale(2., 2., 1.),
                    material: lit.clone().into(),
                }],
                Some(maps.binding()),
                PassOptions::default(),
            )
            .unwrap();
        frames.push(vec![maps.commands, receiver.commands]);
    }
    drop(atlas);
    drop(surfaces);
    drop(shadows);
    drop(mesh);
    gpu.queue.submit(frames.pop().unwrap());
    let dark = quasar_frame::read_rgba(&gpu, &target.texture).unwrap();
    assert!(pixel(&dark, 64, 64)[0] < 30);
    gpu.queue.submit(frames.pop().unwrap());
    let bright = quasar_frame::read_rgba(&gpu, &target.texture).unwrap();
    assert!(pixel(&bright, 64, 64)[0] > 240);
}
