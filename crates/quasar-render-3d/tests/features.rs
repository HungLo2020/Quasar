//! Clustered lights, mesh fog, vertex colours, surface transparency and
//! crowd skinning.
mod support;

use quasar_frame::{Gpu, OffscreenTarget};
use quasar_render_3d::*;
use support::upload_color;

const SIZE: u32 = 64;

fn quad(gpu: &Gpu, half: f32, z: f32) -> Mesh {
    Mesh::upload(gpu, &quad_vertices(half, 0., z), &[0, 1, 2, 0, 2, 3]).unwrap()
}

fn quad_vertices(half: f32, x: f32, z: f32) -> Vec<Vertex> {
    [[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]]
        .map(|[u, v]| Vertex {
            position: [u * half + x, v * half, z],
            normal: [0., 0., 1.],
            uv: [0.5; 2],
        })
        .to_vec()
}

fn camera() -> LitCamera3D {
    LitCamera3D {
        view_projection: Mat4::perspective(1., 1., 0.1, 100.).unwrap()
            * Mat4::translation(0., 0., -3.),
        position: [0., 0., 3.],
    }
}

fn unlit(view: &vulkanic_gal::TextureView, rgba: [f32; 4]) -> SurfaceMaterial {
    let mut material = LitMaterial::textured(view);
    material.shading = ShadingModel::Unlit;
    material.specular = [0.; 3];
    material.base.tint = rgba;
    material.into()
}

fn pixel(rgba: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    rgba[i..i + 4].try_into().unwrap()
}

fn near(a: [u8; 4], b: [u8; 4], tolerance: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tolerance)
}

fn lights(count: usize) -> Vec<Light> {
    let mut lights = vec![Light::Directional {
        direction: [0.2, -1., -0.3],
        color: [0.2, 0.2, 0.25],
        intensity: 1.,
    }];
    let mut seed = 0x9e37_79b9u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32
    };
    for _ in 0..count {
        lights.push(Light::Point {
            position: [next() * 40. - 20., next() * 4. - 2., next() * 6. - 3.],
            color: [next(), next(), next()],
            intensity: 2.,
            range: 1. + next() * 5.,
            attenuation: Attenuation::Smooth,
        });
    }
    lights
}

#[test]
fn clustered_cells_list_every_light_that_reaches_them() {
    let lights = lights(120);
    let clusters = LightClusters {
        min_lights: 1,
        cell_size: 2.,
        dims: [16, 8, 16],
    };
    let grid = cluster_lights(&lights, [0.3, 0.2, -0.1], &clusters)
        .unwrap()
        .expect("enough lights to cluster");
    assert_eq!(grid.globals, vec![0], "directional lights shade everywhere");
    let mut checked = 0;
    for step in 0..4000 {
        let t = step as f32;
        let point = [
            (t * 0.37).sin() * 15.,
            (t * 0.11).cos() * 7.,
            (t * 0.23).sin() * 15.,
        ];
        let Some(listed) = grid.lights_at(point) else {
            continue;
        };
        checked += 1;
        for (index, light) in lights.iter().enumerate() {
            if let Light::Point {
                position, range, ..
            } = light
            {
                let d2: f32 = (0..3).map(|a| (point[a] - position[a]).powi(2)).sum();
                if d2 < range * range {
                    assert!(
                        listed.contains(&(index as u32)),
                        "light {index} reaches {point:?} but is not listed"
                    );
                }
            }
        }
    }
    assert!(checked > 1000, "most sample points fall inside the grid");
    let total: usize = grid.cells.iter().map(|c| c.1 as usize).sum();
    assert!(
        total < grid.cells.len() * lights.len() / 4,
        "cells list only nearby lights"
    );
    // Below the threshold every fragment shades every light instead.
    assert!(
        cluster_lights(&lights[..3], [0.; 3], &LightClusters::default())
            .unwrap()
            .is_none()
    );
    assert!(
        cluster_lights(&lights, [0.; 3], &LightClusters::DISABLED)
            .unwrap()
            .is_none()
    );
    let invalid = LightClusters {
        cell_size: 0.,
        ..LightClusters::default()
    };
    assert!(cluster_lights(&lights, [0.; 3], &invalid).is_err());
}

#[test]
#[ignore = "requires Vulkan"]
fn clustered_lighting_matches_shading_every_light() {
    let gpu = Gpu::headless().unwrap();
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE).unwrap();
    let (_texture, white) = upload_color(&gpu, [255; 4]).unwrap();
    let floor = quad(&gpu, 30., -1.);
    let lights = lights(240);
    let mut material: SurfaceMaterial = LitMaterial::textured(&white).into();
    material.lit.shading = ShadingModel::Diffuse;
    material.receives_shadows = false;
    let mut renderer = SurfaceRenderState3D::new(&gpu, 256, 0).unwrap();
    let mut render = |clusters: LightClusters| {
        renderer.set_light_clusters(clusters).unwrap();
        target
            .render(&gpu, |gpu, target| {
                Ok(vec![
                    renderer
                        .prepare(
                            gpu,
                            target,
                            camera(),
                            Lighting {
                                ambient: [0.02; 3],
                                lights: &lights,
                                output_color_space: ColorSpace::Srgb,
                            },
                            &[SurfaceDraw3D {
                                mesh: &floor,
                                model: Mat4::IDENTITY,
                                material: material.clone(),
                            }],
                            None,
                            PassOptions::default(),
                        )?
                        .commands,
                ])
            })
            .unwrap()
            .rgba
    };
    let every = render(LightClusters::DISABLED);
    let clustered = render(LightClusters {
        min_lights: 1,
        cell_size: 1.5,
        dims: [32, 8, 16],
    });
    let distinct: std::collections::BTreeSet<_> = every.as_chunks::<4>().0.iter().collect();
    assert!(distinct.len() > 50, "many lights shade the floor");
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (a, b) = (pixel(&every, x, y), pixel(&clustered, x, y));
            assert!(near(a, b, 1), "({x},{y}): {a:?} vs clustered {b:?}");
        }
    }
}

#[test]
#[ignore = "requires Vulkan"]
fn mesh_fog_fades_opaque_and_transparent_draws_with_distance() {
    let gpu = Gpu::headless().unwrap();
    let scene = SceneTarget::new(&gpu, SIZE, SIZE).unwrap();
    let output = OffscreenTarget::new(&gpu, SIZE, SIZE).unwrap();
    let mut post = PostChain::new(&gpu).unwrap();
    let (_texture, white) = upload_color(&gpu, [255; 4]).unwrap();
    // A full-screen quad 7 m from the eye: halfway through 4..10 m fog.
    let wall = quad(&gpu, 20., -4.);
    let mut surfaces = SurfaceRenderState3D::new(&gpu, 1, 0).unwrap();
    let fog = Fog::distance([0., 0., 1.], 4., 10.);
    surfaces.set_fog(Some(fog)).unwrap();
    assert!(
        surfaces
            .set_fog(Some(Fog {
                start: 5.,
                end: 5.,
                ..fog
            }))
            .is_err()
    );
    let settings = PostSettings::passthrough();
    let mut render = |surfaces: &mut SurfaceRenderState3D, transparent: bool| {
        output
            .render(&gpu, |gpu, target| {
                let draw = SurfaceDraw3D {
                    mesh: &wall,
                    model: Mat4::IDENTITY,
                    material: unlit(&white, [1., 0., 0., 1.]),
                };
                let mut commands = vec![
                    surfaces
                        .prepare(
                            gpu,
                            scene.target(),
                            camera(),
                            Lighting::default(),
                            if transparent {
                                &[]
                            } else {
                                std::slice::from_ref(&draw)
                            },
                            None,
                            PassOptions {
                                clear_color: Some([0., 0., 0., 1.]),
                                clear_depth: true,
                            },
                        )?
                        .commands,
                ];
                if transparent {
                    commands.push(
                        surfaces
                            .prepare_transparent(
                                gpu,
                                scene.target(),
                                camera(),
                                Lighting::default(),
                                &[SurfaceTransparentDraw3D::new(
                                    &wall,
                                    Mat4::IDENTITY,
                                    unlit(&white, [1., 0., 0., 1.]),
                                    BlendMode::Alpha,
                                )],
                                None,
                                PassOptions::load(),
                            )?
                            .commands,
                    );
                }
                commands.push(post.prepare_scene(gpu, &scene, target, &settings)?);
                Ok(commands)
            })
            .unwrap()
            .rgba
    };
    // Linear 0.5/0.5 red/blue, encoded once by the post chain.
    let half = 188;
    for transparent in [false, true] {
        let rgba = render(&mut surfaces, transparent);
        let center = pixel(&rgba, SIZE / 2, SIZE / 2);
        assert!(
            near(center, [half, 0, half, 255], 3),
            "transparent={transparent}: {center:?}"
        );
    }
    surfaces.set_fog(None).unwrap();
    assert!(near(
        pixel(&render(&mut surfaces, false), SIZE / 2, SIZE / 2),
        [255, 0, 0, 255],
        1
    ));
}

#[test]
#[ignore = "requires Vulkan"]
fn vertex_colours_multiply_into_every_pass() {
    let gpu = Gpu::headless().unwrap();
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE).unwrap();
    let (_texture, white) = upload_color(&gpu, [255; 4]).unwrap();
    // Left half red, right half green: two quads in one coloured mesh.
    let mut vertices = quad_vertices(0.5, -0.5, 0.);
    vertices.extend(quad_vertices(0.5, 0.5, 0.));
    let red = [1., 0., 0., 1.];
    let green = [0., 1., 0., 1.];
    let colors = [[red; 4], [green; 4]].concat();
    let indices = [0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7];
    let mesh = Mesh::upload_colored(&gpu, &vertices, &colors, &indices).unwrap();
    assert!(Mesh::upload_colored(&gpu, &vertices, &colors[..3], &indices).is_err());
    let mut surfaces = SurfaceRenderState3D::new(&gpu, 1, 0).unwrap();
    let mut legacy = RenderState3D::new(&gpu).unwrap();
    let surface = target
        .render(&gpu, |gpu, target| {
            Ok(vec![
                surfaces
                    .prepare(
                        gpu,
                        target,
                        camera(),
                        Lighting {
                            output_color_space: ColorSpace::Srgb,
                            ..Lighting::default()
                        },
                        &[SurfaceDraw3D {
                            mesh: &mesh,
                            model: Mat4::IDENTITY,
                            material: unlit(&white, [1., 1., 1., 1.]),
                        }],
                        None,
                        PassOptions::default(),
                    )?
                    .commands,
            ])
        })
        .unwrap()
        .rgba;
    let mut material = Material::textured(&white);
    material.tint = [1., 1., 1., 1.];
    let plain = target
        .render(&gpu, |gpu, target| {
            Ok(vec![
                legacy
                    .prepare(
                        gpu,
                        target,
                        Camera3D {
                            view_projection: camera().view_projection,
                            light_direction: [0., 0., 1.],
                        },
                        &[Draw3D {
                            mesh: &mesh,
                            model: Mat4::IDENTITY,
                            material: material.clone(),
                        }],
                        PassOptions::default(),
                    )?
                    .commands,
            ])
        })
        .unwrap()
        .rgba;
    let (left, right) = (SIZE / 2 - 6, SIZE / 2 + 6);
    assert!(near(pixel(&surface, left, SIZE / 2), [255, 0, 0, 255], 1));
    assert!(near(pixel(&surface, right, SIZE / 2), [0, 255, 0, 255], 1));
    let l = pixel(&plain, left, SIZE / 2);
    let r = pixel(&plain, right, SIZE / 2);
    assert!(
        l[0] > 200 && l[1] == 0 && r[1] > 200 && r[0] == 0,
        "{l:?} {r:?}"
    );
}

#[test]
#[ignore = "requires Vulkan"]
fn surface_transparency_blends_back_to_front_over_the_opaque_pass() {
    let gpu = Gpu::headless().unwrap();
    let scene = SceneTarget::new(&gpu, SIZE, SIZE).unwrap();
    let output = OffscreenTarget::new(&gpu, SIZE, SIZE).unwrap();
    let mut post = PostChain::new(&gpu).unwrap();
    let (_texture, white) = upload_color(&gpu, [255; 4]).unwrap();
    let (_normal_texture, flat) = upload_color(&gpu, [128, 128, 255, 255]).unwrap();
    let back = quad(&gpu, 20., -2.);
    let near_glass = quad(&gpu, 20., 1.);
    let far_glass = quad(&gpu, 20., 0.);
    let mut surfaces = SurfaceRenderState3D::new(&gpu, 1, 0).unwrap();
    let settings = PostSettings::passthrough();
    let mut glass = unlit(&white, [0., 0., 1., 0.5]);
    glass.normal_map = Some(NormalMap::new(&flat));
    let rgba = output
        .render(&gpu, |gpu, target| {
            Ok(vec![
                surfaces
                    .prepare(
                        gpu,
                        scene.target(),
                        camera(),
                        Lighting::default(),
                        &[SurfaceDraw3D {
                            mesh: &back,
                            model: Mat4::IDENTITY,
                            material: unlit(&white, [1., 0., 0., 1.]),
                        }],
                        None,
                        PassOptions {
                            clear_color: Some([0., 0., 0., 1.]),
                            clear_depth: true,
                        },
                    )?
                    .commands,
                // Submitted near-first: the pass must sort them.
                surfaces
                    .prepare_transparent(
                        gpu,
                        scene.target(),
                        camera(),
                        Lighting::default(),
                        &[
                            SurfaceTransparentDraw3D::new(
                                &near_glass,
                                Mat4::IDENTITY,
                                unlit(&white, [0., 1., 0., 0.5]),
                                BlendMode::Alpha,
                            ),
                            SurfaceTransparentDraw3D::new(
                                &far_glass,
                                Mat4::IDENTITY,
                                glass.clone(),
                                BlendMode::Alpha,
                            ),
                        ],
                        None,
                        PassOptions::load(),
                    )?
                    .commands,
                post.prepare_scene(gpu, &scene, target, &settings)?,
            ])
        })
        .unwrap()
        .rgba;
    // red -> blue at 50% -> green at 50%: linear (0.25, 0.5, 0.25).
    let center = pixel(&rgba, SIZE / 2, SIZE / 2);
    assert!(near(center, [137, 188, 137, 255], 3), "{center:?}");
    assert!(
        surfaces
            .prepare_transparent(
                &gpu,
                scene.target(),
                camera(),
                Lighting {
                    output_color_space: ColorSpace::Srgb,
                    ..Lighting::default()
                },
                &[],
                None,
                PassOptions::load(),
            )
            .is_err(),
        "blending needs linear output"
    );
}

#[test]
#[ignore = "requires Vulkan"]
fn crowds_skin_many_instances_in_one_dispatch_and_one_draw() {
    let gpu = Gpu::headless().unwrap();
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE).unwrap();
    let (_texture, white) = upload_color(&gpu, [255; 4]).unwrap();
    let vertices = quad_vertices(0.2, 0., 0.);
    let influences = [SkinInfluence::single(0); 4];
    let colors = [[1., 1., 0., 1.]; 4];
    let mut crowd = SkinnedCrowd::upload(
        &gpu,
        &vertices,
        Some(&colors),
        &influences,
        &[0, 1, 2, 0, 2, 3],
        1,
        4,
    )
    .unwrap();
    assert_eq!(crowd.capacity(), 4);
    let offsets = [-0.9f32, 0., 0.9];
    let palettes: Vec<[Mat4; 1]> = offsets
        .iter()
        .map(|x| [Mat4::translation(*x, 0., 0.)])
        .collect();
    let refs: Vec<&[Mat4]> = palettes.iter().map(|p| p.as_slice()).collect();
    let mut pass = SkinningPass::new(&gpu).unwrap();
    let commands = pass
        .prepare_crowds(&gpu, &mut [(&mut crowd, refs.as_slice())])
        .unwrap();
    gpu.queue.submit([commands]);
    for (slot, palette) in palettes.iter().enumerate() {
        let gpu_vertices = crowd.read_vertices(&gpu, slot).unwrap();
        let cpu = skin_vertices(&vertices, &influences, palette).unwrap();
        for (a, b) in gpu_vertices.iter().zip(&cpu) {
            assert!(
                a.position
                    .iter()
                    .zip(b.position)
                    .all(|(x, y)| (x - y).abs() < 1e-5),
                "slot {slot}"
            );
        }
    }
    // The fourth slot was not posed and stays in the bind pose.
    assert_eq!(
        crowd.read_vertices(&gpu, 3).unwrap()[0].position,
        vertices[0].position
    );
    let mut surfaces = SurfaceRenderState3D::with_submission(
        &gpu,
        1,
        0,
        SubmissionConfig {
            instancing: true,
            ..SubmissionConfig::default()
        },
    )
    .unwrap();
    let draws: Vec<_> = (0..3)
        .map(|slot| SurfaceDraw3D {
            mesh: crowd.instance(slot),
            model: Mat4::IDENTITY,
            material: unlit(&white, [1., 1., 1., 1.]),
        })
        .collect();
    let rgba = target
        .render(&gpu, |gpu, target| {
            Ok(vec![
                surfaces
                    .prepare(
                        gpu,
                        target,
                        camera(),
                        Lighting {
                            output_color_space: ColorSpace::Srgb,
                            ..Lighting::default()
                        },
                        &draws,
                        None,
                        PassOptions {
                            clear_color: Some([0., 0., 0., 1.]),
                            clear_depth: true,
                        },
                    )?
                    .commands,
            ])
        })
        .unwrap()
        .rgba;
    assert_eq!(
        surfaces.work_stats().draws,
        1,
        "one instanced draw for the crowd"
    );
    assert_eq!(surfaces.work_stats().visible, 3);
    // Three yellow (vertex colour) quads at the posed positions.
    let projected = |x: f32| {
        camera()
            .project(
                [x, 0., 0.],
                quasar_math::Viewport::new(SIZE as f32, SIZE as f32),
            )
            .unwrap()
    };
    for x in offsets {
        let p = projected(x);
        let colour = pixel(&rgba, p[0] as u32, p[1] as u32);
        assert!(
            near(colour, [255, 255, 0, 255], 1),
            "quad at {x}: {colour:?}"
        );
    }
    let gap = projected(0.45);
    assert_eq!(pixel(&rgba, gap[0] as u32, gap[1] as u32), [0, 0, 0, 255]);
    // Too many palettes for the slots is rejected.
    let five: Vec<&[Mat4]> = (0..5).map(|_| refs[0]).collect();
    assert!(
        pass.prepare_crowds(&gpu, &mut [(&mut crowd, five.as_slice())])
            .is_err()
    );
}

fn opaque_scene(
    gpu: &Gpu,
    surfaces: &mut SurfaceRenderState3D,
    scene: &SceneTarget,
    draws: &[SurfaceDraw3D<'_>],
) -> vulkanic_gal::CommandBuffer {
    surfaces
        .prepare(
            gpu,
            scene.target(),
            camera(),
            Lighting::default(),
            draws,
            None,
            PassOptions {
                clear_color: Some([0., 0., 0., 1.]),
                clear_depth: true,
            },
        )
        .unwrap()
        .commands
}

#[test]
#[ignore = "requires Vulkan"]
fn particles_integrate_on_the_gpu_and_draw_while_alive() {
    let gpu = Gpu::headless().unwrap();
    let scene = SceneTarget::new(&gpu, SIZE, SIZE).unwrap();
    let output = OffscreenTarget::new(&gpu, SIZE, SIZE).unwrap();
    let mut post = PostChain::new(&gpu).unwrap();
    let settings = ParticleSettings {
        gravity: [0., -2., 0.],
        drag: 0.25,
        blend: BlendMode::Additive,
    };
    let mut particles = ParticleSystem::new(&gpu, 64, settings, 7).unwrap();
    let burst = ParticleBurst {
        count: 40,
        speed: [0.2, 0.4],
        life: [0.5, 0.5],
        size: [0.3, 0.3],
        color_start: [1., 1., 1., 1.],
        color_end: [1., 1., 1., 1.],
        ..ParticleBurst::default()
    };
    particles.emit(&burst).unwrap();
    assert!(
        particles
            .emit(&ParticleBurst {
                life: [0., 1.],
                ..burst
            })
            .is_err()
    );
    // Same seed, same bursts: identical particles.
    let mut twin = ParticleSystem::new(&gpu, 64, settings, 7).unwrap();
    twin.emit(&burst).unwrap();
    gpu.queue.submit([
        particles.step(&gpu, 0.).unwrap(),
        twin.step(&gpu, 0.).unwrap(),
    ]);
    let start = particles.read_particles(&gpu).unwrap();
    assert_eq!(start, twin.read_particles(&gpu).unwrap());
    assert_eq!(start.iter().filter(|p| p.alive()).count(), 40);
    // GPU integration matches the CPU reference step.
    let mut expected = start.clone();
    for _ in 0..5 {
        gpu.queue.submit([particles.step(&gpu, 0.05).unwrap()]);
        for p in &mut expected {
            p.step(&settings, 0.05);
        }
    }
    for (gpu_p, cpu_p) in particles
        .read_particles(&gpu)
        .unwrap()
        .iter()
        .zip(&expected)
    {
        for a in 0..3 {
            assert!((gpu_p.position[a] - cpu_p.position[a]).abs() < 1e-5);
        }
        assert!((gpu_p.age - cpu_p.age).abs() < 1e-6);
    }
    assert_eq!(particles.live_count(), 40);
    let view = Mat4::translation(0., 0., -3.);
    let projection = Mat4::perspective(1., 1., 0.1, 100.).unwrap();
    let cam = ParticleCamera::from_view(view, projection);
    let mut render = |particles: &mut ParticleSystem, dt: f32| {
        output
            .render(&gpu, |gpu, target| {
                Ok(vec![
                    scene_clear(gpu, &scene),
                    particles.prepare(gpu, scene.target(), cam, dt)?,
                    post.prepare_scene(gpu, &scene, target, &PostSettings::passthrough())?,
                ])
            })
            .unwrap()
            .rgba
    };
    let lit = render(&mut particles, 0.01);
    let bright = lit.as_chunks::<4>().0.iter().filter(|p| p[0] > 100).count();
    assert!(bright > 20, "live particles draw ({bright} px)");
    // `clear` removes live and queued particles at the next prepare.
    twin.emit(&burst).unwrap();
    twin.clear();
    assert_eq!(twin.live_count(), 0);
    let cleared = render(&mut twin, 0.0);
    assert!(
        cleared.as_chunks::<4>().0.iter().all(|p| p[0] == 0),
        "cleared particles vanish at once"
    );
    assert!(
        twin.read_particles(&gpu)
            .unwrap()
            .iter()
            .all(|p| !p.alive())
    );
    twin.emit(&burst).unwrap();
    let again = render(&mut twin, 0.01);
    assert!(
        again
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] > 100)
            .count()
            > 20,
        "bursts after a clear still draw"
    );
    // After their lifetime the slots stop drawing.
    let gone = render(&mut particles, 0.5);
    assert!(
        gone.as_chunks::<4>().0.iter().all(|p| p[0] == 0),
        "dead particles vanish"
    );
    assert_eq!(particles.live_count(), 0);
}

fn scene_clear(gpu: &Gpu, scene: &SceneTarget) -> vulkanic_gal::CommandBuffer {
    let mut encoder = gpu.encoder("clear");
    let depth = scene.depth_view();
    encoder.begin_render_pass(&vulkanic_gal::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(vulkanic_gal::RenderPassColorAttachment {
            view: scene.color_view(),
            depth_slice: None,
            resolve_target: None,
            ops: vulkanic_gal::Operations {
                load: vulkanic_gal::LoadOp::Clear(vulkanic_gal::Color::BLACK),
                store: vulkanic_gal::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(vulkanic_gal::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(vulkanic_gal::Operations {
                load: vulkanic_gal::LoadOp::Clear(1.),
                store: vulkanic_gal::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        ..Default::default()
    });
    encoder.finish()
}

#[test]
#[ignore = "requires Vulkan"]
fn decals_project_onto_scene_depth_inside_their_box_only() {
    let gpu = Gpu::headless().unwrap();
    let scene = SceneTarget::new(&gpu, SIZE, SIZE).unwrap();
    let output = OffscreenTarget::new(&gpu, SIZE, SIZE).unwrap();
    let mut post = PostChain::new(&gpu).unwrap();
    let (_white_texture, white) = upload_color(&gpu, [255; 4]).unwrap();
    let (_red_texture, red) = upload_color(&gpu, [255, 0, 0, 255]).unwrap();
    let wall = quad(&gpu, 20., 0.);
    let mut surfaces = SurfaceRenderState3D::new(&gpu, 1, 0).unwrap();
    let mut decals = DecalRenderer::new(&gpu).unwrap();
    // The wall faces +Z; rotate the box so its projection axis (Y) is Z.
    let onto_wall = Mat4::translation(0., 0., 0.)
        * Mat4::rotation_x(std::f32::consts::FRAC_PI_2)
        * Mat4::scale(1., 0.5, 1.);
    let floating = Mat4::translation(0., 0., 1.5) * Mat4::rotation_x(std::f32::consts::FRAC_PI_2);
    let mut render = |list: &[Decal<'_>]| {
        output
            .render(&gpu, |gpu, target| {
                Ok(vec![
                    opaque_scene(
                        gpu,
                        &mut surfaces,
                        &scene,
                        &[SurfaceDraw3D {
                            mesh: &wall,
                            model: Mat4::IDENTITY,
                            material: unlit(&white, [0.5, 0.5, 0.5, 1.]),
                        }],
                    ),
                    decals.prepare(
                        gpu,
                        &scene,
                        camera().view_projection,
                        list,
                        BlendMode::Alpha,
                    )?,
                    post.prepare_scene(gpu, &scene, target, &PostSettings::passthrough())?,
                ])
            })
            .unwrap()
            .rgba
    };
    let plain = render(&[]);
    let marked = render(&[Decal {
        fade: 0.,
        ..Decal::new(onto_wall, &red)
    }]);
    let center = pixel(&marked, SIZE / 2, SIZE / 2);
    assert!(
        near(center, [255, 0, 0, 255], 2),
        "decal on the wall: {center:?}"
    );
    let edge = pixel(&marked, 2, 2);
    assert_eq!(
        edge,
        pixel(&plain, 2, 2),
        "outside the box the wall is unchanged"
    );
    // A box with no surface inside it leaves the scene alone.
    let empty = render(&[Decal::new(floating, &red)]);
    assert_eq!(empty, plain);
    assert!(
        decals
            .prepare(
                &gpu,
                &scene,
                camera().view_projection,
                &[Decal::new(Mat4::scale(0., 1., 1.), &red)],
                BlendMode::Alpha
            )
            .is_err()
    );
}

#[test]
#[ignore = "requires Vulkan"]
fn outlines_ring_silhouettes_and_fill_parts_hidden_behind_walls() {
    let gpu = Gpu::headless().unwrap();
    let scene = SceneTarget::new(&gpu, SIZE, SIZE).unwrap();
    let output = OffscreenTarget::new(&gpu, SIZE, SIZE).unwrap();
    let mut post = PostChain::new(&gpu).unwrap();
    let (_white_texture, white) = upload_color(&gpu, [255; 4]).unwrap();
    // A wall covering the left half, in front of an "enemy" quad.
    let wall = Mesh::upload(
        &gpu,
        &[[-5., -5.], [0., -5.], [0., 5.], [-5., 5.]].map(|[x, y]| Vertex {
            position: [x, y, 1.],
            normal: [0., 0., 1.],
            uv: [0.5; 2],
        }),
        &[0, 1, 2, 0, 2, 3],
    )
    .unwrap();
    let enemy = quad(&gpu, 0.5, 0.);
    let mut surfaces = SurfaceRenderState3D::new(&gpu, 1, 0).unwrap();
    let mut outlines = OutlineRenderer::new(&gpu).unwrap();
    let style = OutlineStyle {
        outline: [0., 1., 0., 1.],
        occluded_outline: [0., 0., 1., 1.],
        occluded_fill: [1., 0., 1., 1.],
        width: 2.,
    };
    let rgba = output
        .render(&gpu, |gpu, target| {
            Ok(vec![
                opaque_scene(
                    gpu,
                    &mut surfaces,
                    &scene,
                    &[
                        SurfaceDraw3D {
                            mesh: &wall,
                            model: Mat4::IDENTITY,
                            material: unlit(&white, [0.2, 0.2, 0.2, 1.]),
                        },
                        SurfaceDraw3D {
                            mesh: &enemy,
                            model: Mat4::IDENTITY,
                            material: unlit(&white, [1., 0., 0., 1.]),
                        },
                    ],
                ),
                outlines.prepare(
                    gpu,
                    &scene,
                    camera().view_projection,
                    &[OutlineDraw {
                        mesh: &enemy,
                        model: Mat4::IDENTITY,
                        style: 0,
                    }],
                    &[style],
                )?,
                post.prepare_scene(gpu, &scene, target, &PostSettings::passthrough())?,
            ])
        })
        .unwrap()
        .rgba;
    let viewport = quasar_math::Viewport::new(SIZE as f32, SIZE as f32);
    let at = |x: f32, y: f32| {
        let p = camera().project([x, y, 0.], viewport).unwrap();
        pixel(&rgba, p[0] as u32, p[1] as u32)
    };
    assert_eq!(
        at(0.25, 0.),
        [255, 0, 0, 255],
        "visible part keeps its colour"
    );
    assert_eq!(
        at(-0.25, 0.),
        [255, 0, 255, 255],
        "hidden part gets the fill"
    );
    let right_edge = camera().project([0.5, 0., 0.], viewport).unwrap();
    let ring = pixel(&rgba, right_edge[0] as u32 + 1, right_edge[1] as u32);
    assert_eq!(ring, [0, 255, 0, 255], "visible silhouette ring");
    let left_edge = camera().project([-0.5, 0., 0.], viewport).unwrap();
    let hidden_ring = pixel(&rgba, left_edge[0] as u32 - 2, left_edge[1] as u32);
    assert_eq!(hidden_ring, [0, 0, 255, 255], "occluded silhouette ring");
    assert_eq!(
        pixel(&rgba, SIZE - 2, 2),
        [0, 0, 0, 255],
        "far pixels untouched"
    );
    assert_eq!(outlines.draws(), 2);
    assert!(
        outlines
            .prepare(
                &gpu,
                &scene,
                camera().view_projection,
                &[OutlineDraw {
                    mesh: &enemy,
                    model: Mat4::IDENTITY,
                    style: 3
                }],
                &[style]
            )
            .is_err()
    );
}
