#![cfg(any(feature = "vulkan", feature = "wgpu"))]
mod support;
use quasar_frame::OffscreenTarget;
use quasar_render_3d::*;
use vulkanic_gal::*;
fn plane(gal: &mut VulkanicGal) -> Mesh {
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
    let mut gal = VulkanicGal::create(
        if cfg!(feature = "vulkan") {
            BackendChoice::Vulkan
        } else {
            BackendChoice::Wgpu
        },
        "surface tests",
    )
    .unwrap();
    let mut renderer = SurfaceRenderState3D::new(&mut gal, 4, 2).unwrap();
    let mut shadows = ShadowRenderState3D::new(
        &mut gal,
        ShadowConfig {
            resolution: 128,
            ..ShadowConfig::default()
        },
    )
    .unwrap();
    let mut mesh = plane(&mut gal);
    let (texture, view) = support::upload_color(&mut gal, [255; 4]).unwrap();
    let (normal_texture, normal_view) =
        support::upload_color(&mut gal, [255, 128, 128, 255]).unwrap();
    let mut target = OffscreenTarget::new(&mut gal, 128, 128).unwrap();
    let mut lit = LitMaterial::textured(view);
    lit.shading = ShadingModel::Diffuse;
    let mut material = SurfaceMaterial::from(lit);
    let lights = [Light::Directional {
        direction: [-0.5, 0., -1.],
        color: [1.; 3],
        intensity: 1.,
    }];
    let mut caster = ShadowCaster {
        mesh: &mesh,
        model: Mat4::translation(0., 0.4, 1.) * Mat4::scale(0.3, 0.3, 1.),
        texture_view: view,
        alpha_cutoff: 0.,
        opacity: 1.,
    };
    let light_view = Mat4::orthographic(-2., 2., -2., 2., 0.1, 10.).unwrap()
        * Mat4::look_at([2., 0., 4.], [0., 0., 0.], [0., 1., 0.]).unwrap();
    let receiver_model = std::cell::Cell::new(Mat4::scale(2., 2., 1.));
    let mut draw = |gal: &mut VulkanicGal,
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
                        material,
                    }],
                    prepared.as_ref().map(PreparedShadows::binding),
                    PassOptions::default(),
                )?;
                let mut commands: Vec<_> = prepared
                    .as_ref()
                    .map(|s| s.commands.clone())
                    .into_iter()
                    .collect();
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
        material,
        &[],
        &lights,
    );
    let shadowed = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material,
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
    let (cutout_texture, cutout_view) =
        support::upload_color(&mut gal, [255, 255, 255, 0]).unwrap();
    let mut filtered =
        |settings: ShadowSettings, material: SurfaceMaterial, caster: &ShadowCaster<'_>| {
            draw(
                &mut gal,
                &mut renderer,
                &mut shadows,
                material,
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
            material,
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
        material,
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
        filtered(ShadowSettings::default(), material, &caster)
    );
    material.receives_shadows = true;
    // Cutouts use the albedo alpha and material opacity in the depth-only pass.
    caster.alpha_cutoff = 0.5;
    caster.opacity = 0.1;
    assert_eq!(
        plain,
        filtered(ShadowSettings::default(), material, &caster)
    );
    caster.opacity = 1.;
    caster.texture_view = cutout_view;
    assert_eq!(
        plain,
        filtered(ShadowSettings::default(), material, &caster)
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
        material,
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
        material,
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
        material,
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
    material.normal_map = Some(NormalMap::new(normal_view));
    let mapped = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material,
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
            material,
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
        material,
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
        material,
        &[],
        &lights,
    );
    assert_eq!(pixel(&mapped, 64, 64), pixel(&scaled, 64, 64));
    receiver_model.set(Mat4::scale(-2., 2., 1.));
    let reflected = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material,
        &[],
        &lights,
    );
    assert_eq!(
        pixel(&mirrored, 64, 64),
        pixel(&reflected, 64, 64),
        "mirrored world transform must preserve normal handedness"
    );
    receiver_model.set(Mat4::scale(2., 2., 1.));
    let (green_texture, green_view) =
        support::upload_color(&mut gal, [128, 255, 128, 255]).unwrap();
    material.normal_map = Some(NormalMap::new(green_view));
    let angled = [Light::Directional {
        direction: [0., 0.5, -1.],
        color: [1.; 3],
        intensity: 1.,
    }];
    let up = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material,
        &[],
        &angled,
    );
    material.normal_map.as_mut().unwrap().flip_y = true;
    let down = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material,
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
    let spot_plain = draw(&mut gal, &mut renderer, &mut shadows, material, &[], &spot);
    let spot_shadow = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material,
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
    let (shared_texture, shared_view) =
        support::upload_pixels(&mut gal, 2, 1, vec![255, 128, 128, 255, 128, 128, 255, 255])
            .unwrap();
    material.lit.base.texture_view = shared_view;
    material.normal_map = None;
    draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material,
        &[],
        &lights,
    );
    material.normal_map = Some(NormalMap::new(shared_view));
    let reused = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material,
        &[],
        &lights,
    );
    renderer.clear_bindings(&mut gal).unwrap();
    let fresh = draw(
        &mut gal,
        &mut renderer,
        &mut shadows,
        material,
        &[],
        &lights,
    );
    assert_eq!(
        reused, fresh,
        "normal-map filtering must not depend on cache history"
    );
    renderer.release(&mut gal).unwrap();
    shadows.release(&mut gal).unwrap();
    mesh.release(&mut gal).unwrap();
    target.release(&mut gal).unwrap();
    for h in [
        shared_view,
        shared_texture,
        cutout_view,
        cutout_texture,
        green_view,
        green_texture,
        normal_view,
        normal_texture,
        view,
        texture,
    ] {
        gal.retire(h).unwrap();
    }
    gal.retire_through(gal.latest_submission_id()).unwrap();
    assert_eq!(
        gal.metrics().resource_creates,
        gal.metrics().resource_destroys
    );
}

#[test]
#[ignore = "requires Vulkan"]
fn shadow_slots_preserve_reordered_frames_and_consumer_reservations() {
    let mut gal = VulkanicGal::create(
        if cfg!(feature = "vulkan") {
            BackendChoice::Vulkan
        } else {
            BackendChoice::Wgpu
        },
        "shadow lifetime tests",
    )
    .unwrap();
    let mut renderer = SurfaceRenderState3D::new(&mut gal, 1, 1).unwrap();
    let mut shadows = ShadowRenderState3D::new(
        &mut gal,
        ShadowConfig {
            resolution: 64,
            max_maps: 1,
            ..ShadowConfig::default()
        },
    )
    .unwrap();
    let mut mesh = plane(&mut gal);
    let (texture, view) = support::upload_color(&mut gal, [255; 4]).unwrap();
    let mut target = OffscreenTarget::new(&mut gal, 128, 128).unwrap();
    let lights = [Light::Directional {
        direction: [0., 0., -1.],
        color: [1.; 3],
        intensity: 1.,
    }];
    let mut lit = LitMaterial::textured(view);
    lit.shading = ShadingModel::Diffuse;
    let draws = [SurfaceDraw3D {
        mesh: &mesh,
        model: Mat4::scale(2., 2., 1.),
        material: lit.into(),
    }];
    let casters = [ShadowCaster {
        mesh: &mesh,
        model: Mat4::translation(0., 0., 1.) * Mat4::scale(0.4, 0.4, 1.),
        texture_view: view,
        alpha_cutoff: 0.,
        opacity: 1.,
    }];
    let mut pending = vec![];
    for strength in [0., 1., 0.5] {
        let maps = shadows
            .prepare(
                &mut gal,
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
                &mut gal,
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
    let atlas_views: std::collections::BTreeSet<_> = pending
        .iter()
        .map(|p| p.0.binding().texture_view())
        .collect();
    assert_eq!(
        atlas_views.len(),
        3,
        "outstanding frames need isolated depth arrays"
    );
    assert!(shadows.prepare(&mut gal, &[]).is_err());
    assert!(shadows.resize(&mut gal, 128).is_err());
    assert!(shadows.release(&mut gal).is_err());
    for (minimum, maximum) in [(100, 170), (0, 30), (240, 255)] {
        let (maps, receiver) = pending.pop().unwrap();
        let image = target
            .render(&mut gal, |_, _| {
                Ok(vec![maps.commands.clone(), receiver.commands.clone()])
            })
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
        let maps = shadows.prepare(&mut gal, &[]).unwrap();
        receivers.push(
            renderer
                .prepare(
                    &mut gal,
                    target.target(),
                    camera(),
                    Lighting::default(),
                    &draws,
                    Some(maps.binding()),
                    PassOptions::default(),
                )
                .unwrap(),
        );
        gal.submit(SubmissionBatch {
            label: "shadow producer only".into(),
            command_lists: vec![maps.commands.clone()],
        })
        .unwrap();
    }
    gal.retire_through(gal.latest_submission_id()).unwrap();
    assert!(shadows.prepare(&mut gal, &[]).is_err());
    assert!(shadows.resize(&mut gal, 128).is_err());
    receivers.clear();
    // A retained binding also reserves its generation for custom GAL consumers.
    let retained: Vec<_> = (0..3)
        .map(|_| shadows.prepare(&mut gal, &[]).unwrap().binding().clone())
        .collect();
    assert!(shadows.prepare(&mut gal, &[]).is_err());
    drop(retained);
    renderer.clear_bindings(&mut gal).unwrap();
    shadows.resize(&mut gal, 128).unwrap();
    // Warm a generation, then reuse it without descriptor/resource churn.
    for _ in 0..2 {
        let maps = shadows.prepare(&mut gal, &[]).unwrap();
        target
            .render(&mut gal, |gal, t| {
                let receiver = renderer.prepare(
                    gal,
                    t,
                    camera(),
                    Lighting::default(),
                    &draws,
                    Some(maps.binding()),
                    PassOptions::default(),
                )?;
                Ok(vec![maps.commands.clone(), receiver.commands])
            })
            .unwrap();
    }
    let before = gal.metrics().resource_creates;
    for _ in 0..4 {
        let maps = shadows.prepare(&mut gal, &[]).unwrap();
        target
            .render(&mut gal, |gal, t| {
                Ok(vec![
                    maps.commands.clone(),
                    renderer
                        .prepare(
                            gal,
                            t,
                            camera(),
                            Lighting::default(),
                            &draws,
                            Some(maps.binding()),
                            PassOptions::default(),
                        )?
                        .commands,
                ])
            })
            .unwrap();
    }
    assert_eq!(before, gal.metrics().resource_creates);
    renderer.release(&mut gal).unwrap();
    shadows.release(&mut gal).unwrap();
    shadows.release(&mut gal).unwrap();
    mesh.release(&mut gal).unwrap();
    target.release(&mut gal).unwrap();
    gal.retire(view).unwrap();
    gal.retire(texture).unwrap();
    gal.retire_through(gal.latest_submission_id()).unwrap();
    assert_eq!(
        gal.metrics().resource_creates,
        gal.metrics().resource_destroys
    );
}

#[test]
#[ignore = "requires Vulkan"]
fn shadow_and_surface_validation_rejects_invalid_data_before_recording() {
    let mut gal = VulkanicGal::create(
        if cfg!(feature = "vulkan") {
            BackendChoice::Vulkan
        } else {
            BackendChoice::Wgpu
        },
        "shadow validation",
    )
    .unwrap();
    let before = gal.metrics().resource_creates;
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
        assert!(ShadowRenderState3D::new(&mut gal, config).is_err());
    }
    assert_eq!(before, gal.metrics().resource_creates);
    let mut shadows = ShadowRenderState3D::new(
        &mut gal,
        ShadowConfig {
            resolution: 64,
            max_maps: 2,
            max_casters: 1,
            ..ShadowConfig::default()
        },
    )
    .unwrap();
    let mut renderer = SurfaceRenderState3D::new(&mut gal, 1, 1).unwrap();
    let mut mesh = plane(&mut gal);
    let (texture, view) = support::upload_color(&mut gal, [255; 4]).unwrap();
    let target = OffscreenTarget::new(&mut gal, 128, 128).unwrap();
    let mut map = ShadowView {
        light_index: 0,
        view_projection: camera().view_projection,
        settings: ShadowSettings::default(),
        casters: &[],
    };
    for bad in [f32::NAN, -1., 2.] {
        map.settings.strength = bad;
        assert!(
            shadows
                .prepare(&mut gal, std::slice::from_ref(&map))
                .is_err()
        );
    }
    map.settings = ShadowSettings::default();
    assert!(
        shadows
            .prepare(&mut gal, &[ShadowView { ..map }, ShadowView { ..map }])
            .is_err()
    );
    let casters = [
        ShadowCaster {
            mesh: &mesh,
            model: Mat4::IDENTITY,
            texture_view: view,
            alpha_cutoff: 0.,
            opacity: 1.,
        },
        ShadowCaster {
            mesh: &mesh,
            model: Mat4::IDENTITY,
            texture_view: view,
            alpha_cutoff: 0.,
            opacity: 1.,
        },
    ];
    assert!(
        shadows
            .prepare(
                &mut gal,
                &[ShadowView {
                    casters: &casters,
                    ..map
                }]
            )
            .is_err()
    );
    let maps = shadows.prepare(&mut gal, &[map]).unwrap();
    let mut material = SurfaceMaterial::from(LitMaterial::textured(view));
    let mut prepare = |material| {
        renderer.prepare(
            &mut gal,
            target.target(),
            camera(),
            Lighting::default(),
            &[SurfaceDraw3D {
                mesh: &mesh,
                model: Mat4::IDENTITY,
                material,
            }],
            Some(maps.binding()),
            PassOptions::default(),
        )
    };
    assert!(
        prepare(material).is_err(),
        "shadow light index must reference supplied directional/spot light"
    );
    material.receives_shadows = false;
    material.normal_map = Some(NormalMap {
        strength: f32::NAN,
        ..NormalMap::new(view)
    });
    assert!(prepare(material).is_err());
    material.normal_map = Some(NormalMap {
        uv_scale: [0., 1.],
        ..NormalMap::new(view)
    });
    assert!(prepare(material).is_err());
    let mut normals_only = SurfaceRenderState3D::new(&mut gal, 0, 0).unwrap();
    let mut unshadowed = SurfaceMaterial::from(LitMaterial::textured(view));
    unshadowed.normal_map = Some(NormalMap::new(view));
    let prepared = normals_only
        .prepare(
            &mut gal,
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
    normals_only.release(&mut gal).unwrap();
    drop(maps);
    renderer.release(&mut gal).unwrap();
    shadows.release(&mut gal).unwrap();
    mesh.release(&mut gal).unwrap();
    target.release(&mut gal).unwrap();
    gal.retire(view).unwrap();
    gal.retire(texture).unwrap();
    gal.retire_through(gal.latest_submission_id()).unwrap();
    assert_eq!(
        gal.metrics().resource_creates,
        gal.metrics().resource_destroys
    );
}
