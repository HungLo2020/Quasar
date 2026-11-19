//! Height fog, light scattering, falloff curves, spot cookies and toon
//! shading through the shared lighting module.
mod support;

use quasar_frame::{Gpu, OffscreenTarget};
use quasar_render_3d::*;
use support::upload_color;
use vulkanic_gal::TextureView;

const SIZE: u32 = 64;

struct Bench {
    gpu: Gpu,
    scene: SceneTarget,
    output: OffscreenTarget,
    post: PostChain,
    surfaces: SurfaceRenderState3D,
    _texture: vulkanic_gal::Texture,
    white: TextureView,
}

impl Bench {
    fn new() -> Self {
        let gpu = Gpu::headless().unwrap();
        let scene = SceneTarget::new(&gpu, SIZE, SIZE).unwrap();
        let output = OffscreenTarget::new(&gpu, SIZE, SIZE).unwrap();
        let post = PostChain::new(&gpu).unwrap();
        let surfaces = SurfaceRenderState3D::new(&gpu, 8, 0).unwrap();
        let (texture, white) = upload_color(&gpu, [255; 4]).unwrap();
        Self {
            gpu,
            scene,
            output,
            post,
            surfaces,
            _texture: texture,
            white,
        }
    }

    fn material(&self, shading: ShadingModel, rgb: [f32; 3]) -> SurfaceMaterial {
        let mut material = LitMaterial::textured(&self.white);
        material.shading = shading;
        material.specular = [0.; 3];
        material.base.tint = [rgb[0], rgb[1], rgb[2], 1.];
        material.into()
    }

    fn render(
        &mut self,
        camera: LitCamera3D,
        lights: &[Light],
        draws: &[SurfaceDraw3D<'_>],
    ) -> Vec<u8> {
        let settings = PostSettings::passthrough();
        let (scene, post, surfaces) = (&self.scene, &mut self.post, &mut self.surfaces);
        self.output
            .render(&self.gpu, |gpu, target| {
                Ok(vec![
                    surfaces
                        .prepare(
                            gpu,
                            scene.target(),
                            camera,
                            Lighting {
                                ambient: [0.; 3],
                                lights,
                                output_color_space: ColorSpace::Linear,
                            },
                            draws,
                            None,
                            PassOptions {
                                clear_color: Some([0., 0., 0., 1.]),
                                clear_depth: true,
                            },
                        )?
                        .commands,
                    post.prepare_scene(gpu, scene, target, &settings)?,
                ])
            })
            .unwrap()
            .rgba
    }
}

fn pixel(rgba: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    rgba[i..i + 4].try_into().unwrap()
}

/// A square in the plane `axis = offset` facing +axis.
fn square(gpu: &Gpu, center: [f32; 3], half: f32, normal: [f32; 3]) -> Mesh {
    let (u, v) = if normal[1].abs() > 0.5 {
        ([1., 0., 0.], [0., 0., -1.])
    } else {
        ([1., 0., 0.], [0., 1., 0.])
    };
    let corner = |a: f32, b: f32| Vertex {
        position: std::array::from_fn(|i| center[i] + (u[i] * a + v[i] * b) * half),
        normal,
        uv: [0.5; 2],
    };
    let vertices = [
        corner(-1., -1.),
        corner(1., -1.),
        corner(1., 1.),
        corner(-1., 1.),
    ];
    Mesh::upload(gpu, &vertices, &[0, 1, 2, 0, 2, 3]).unwrap()
}

fn front_camera() -> LitCamera3D {
    LitCamera3D {
        view_projection: Mat4::perspective(1., 1., 0.1, 100.).unwrap()
            * Mat4::look_at([0., 0., 3.], [0., 0., 0.], [0., 1., 0.]).unwrap(),
        position: [0., 0., 3.],
    }
}

#[test]
#[ignore = "requires Vulkan"]
fn height_fog_pools_on_low_ground_under_a_top_down_camera() {
    let mut bench = Bench::new();
    let camera = LitCamera3D {
        view_projection: Mat4::perspective(1., 1., 0.1, 100.).unwrap()
            * Mat4::look_at([0., 12., 0.], [0., 0., 0.], [0., 0., -1.]).unwrap(),
        position: [0., 12., 0.],
    };
    // Left: floor at height 0. Right: a block top at height 4.
    let floor = square(&bench.gpu, [-2., 0., 0.], 2., [0., 1., 0.]);
    let block = square(&bench.gpu, [2., 4., 0.], 2., [0., 1., 0.]);
    let white = bench.material(ShadingModel::Unlit, [1.; 3]);
    let draws = [
        SurfaceDraw3D {
            mesh: &floor,
            model: Mat4::IDENTITY,
            material: white.clone(),
        },
        SurfaceDraw3D {
            mesh: &block,
            model: Mat4::IDENTITY,
            material: white,
        },
    ];
    let (left, right) = ((SIZE / 4, SIZE / 2), (SIZE * 3 / 4, SIZE / 2));
    let clear = bench.render(camera, &[], &draws);
    assert_eq!(pixel(&clear, left.0, left.1)[0], 255);
    assert_eq!(pixel(&clear, right.0, right.1)[0], 255);
    // Distance fog alone barely separates them from straight above.
    let haze = HeightFog {
        base: 0.,
        density: 1.2,
        falloff: 1.2,
    };
    bench
        .surfaces
        .set_fog(Some(Fog::height([0., 0., 1.], haze)))
        .unwrap();
    let fogged = bench.render(camera, &[], &draws);
    let (low, high) = (
        pixel(&fogged, left.0, left.1),
        pixel(&fogged, right.0, right.1),
    );
    assert!(low[0] < 200, "the floor sits in haze: {low:?}");
    assert!(high[0] > 235, "the block top is above it: {high:?}");
    assert!(low[2] > 200 && high[2] > 200);
    // Invalid settings are rejected.
    let bad = HeightFog {
        density: -1.,
        ..haze
    };
    assert!(
        bench
            .surfaces
            .set_fog(Some(Fog::height([0.; 3], bad)))
            .is_err()
    );
}

#[test]
#[ignore = "requires Vulkan"]
fn scattering_makes_lights_glow_through_fog() {
    let mut bench = Bench::new();
    let wall = square(&bench.gpu, [0., 0., -4.], 20., [0., 0., 1.]);
    let black = bench.material(ShadingModel::Unlit, [0.; 3]);
    let draws = [SurfaceDraw3D {
        mesh: &wall,
        model: Mat4::IDENTITY,
        material: black,
    }];
    let lamp = [Light::Point {
        position: [0., 0., 0.],
        color: [1., 0.8, 0.5],
        intensity: 3.,
        range: 1.2,
        attenuation: Attenuation::Smooth,
    }];
    let fog = Fog::distance([0.; 3], 0., 30.);
    bench.surfaces.set_fog(Some(fog)).unwrap();
    let plain = bench.render(front_camera(), &lamp, &draws);
    assert_eq!(pixel(&plain, SIZE / 2, SIZE / 2), [0, 0, 0, 255]);
    bench
        .surfaces
        .set_fog(Some(fog.with_scattering(10.)))
        .unwrap();
    let lit = bench.render(front_camera(), &lamp, &draws);
    let center = pixel(&lit, SIZE / 2, SIZE / 2);
    let corner = pixel(&lit, 2, 2);
    assert!(
        center[0] > 60,
        "rays through the lamp's range glow: {center:?}"
    );
    assert!(center[0] > center[2], "in the lamp's colour: {center:?}");
    assert!(
        corner[0] < center[0] / 3,
        "rays far from it stay dark: {corner:?}"
    );
}

#[test]
#[ignore = "requires Vulkan"]
fn cookies_pattern_spot_pools_and_curves_shape_falloff() {
    let mut bench = Bench::new();
    let wall = square(&bench.gpu, [0., 0., -1.], 6., [0., 0., 1.]);
    let diffuse = bench.material(ShadingModel::Diffuse, [1.; 3]);
    let draws = [SurfaceDraw3D {
        mesh: &wall,
        model: Mat4::IDENTITY,
        material: diffuse,
    }];
    let spot = |cookie: LightCookie| Light::Spot {
        position: [0., 0., 1.5],
        direction: [0., 0., -1.],
        color: [1.; 3],
        intensity: 0.7,
        range: 6.,
        inner_angle: 0.45,
        outer_angle: 0.5,
        attenuation: Attenuation::Smooth,
        cookie,
    };
    // The pool's interior (the cone edge falls outside 18..46).
    let row = |rgba: &[u8]| -> Vec<u8> { (18..46).map(|x| pixel(rgba, x, SIZE / 2)[0]).collect() };
    let column =
        |rgba: &[u8]| -> Vec<u8> { (18..46).map(|y| pixel(rgba, SIZE / 2, y)[0]).collect() };
    let spread = |values: &[u8]| {
        i32::from(*values.iter().max().unwrap()) - i32::from(*values.iter().min().unwrap())
    };
    let plain = bench.render(front_camera(), &[spot(LightCookie::NONE)], &draws);
    let bars = LightCookie::new(CookiePattern::Bars, 3, 1.);
    let striped = bench.render(front_camera(), &[spot(bars)], &draws);
    let center = (SIZE / 2, SIZE / 2);
    assert!(
        pixel(&striped, center.0, center.1)[0].abs_diff(pixel(&plain, center.0, center.1)[0]) <= 2,
        "the pattern is 1 on the spot axis"
    );
    // Bars vary across the light's u axis (world X here), not along Y.
    assert!(spread(&row(&striped)) > spread(&row(&plain)) + 60);
    assert!(spread(&column(&striped)) <= spread(&column(&plain)) + 6);
    // Rotating the cookie a quarter turn swaps the stripes.
    let turned = bench.render(
        front_camera(),
        &[spot(bars.with_rotation(std::f32::consts::FRAC_PI_2))],
        &draws,
    );
    assert!(spread(&column(&turned)) > spread(&column(&plain)) + 60);
    assert!(spread(&row(&turned)) <= spread(&row(&plain)) + 6);
    assert!((LightCookie::NONE.sample([0.3, 0.2]) - 1.).abs() < 1e-6);
    assert!(bars.sample([1. / 3., 0.]) < 0.01);

    // Falloff curves: a high exponent hugs the light.
    let point = |attenuation| Light::Point {
        position: [0., 0., -0.5],
        color: [1.; 3],
        intensity: 1.,
        range: 3.,
        attenuation,
    };
    let at = (SIZE / 2 + 10, SIZE / 2);
    let soft = bench.render(
        front_camera(),
        &[point(Attenuation::Curve { exponent: 0.5 })],
        &draws,
    );
    let tight = bench.render(
        front_camera(),
        &[point(Attenuation::Curve { exponent: 4. })],
        &draws,
    );
    assert!(
        pixel(&soft, at.0, at.1)[0] > pixel(&tight, at.0, at.1)[0] + 40,
        "{:?} vs {:?}",
        pixel(&soft, at.0, at.1),
        pixel(&tight, at.0, at.1)
    );
    let invalid = [point(Attenuation::Curve { exponent: 0. })];
    assert!(
        encode_lighting(
            front_camera(),
            Lighting {
                lights: &invalid,
                ..Lighting::default()
            },
            4
        )
        .is_err()
    );
}

#[test]
#[ignore = "requires Vulkan"]
fn toon_shading_quantises_light_into_bands() {
    let mut bench = Bench::new();
    let wall = square(&bench.gpu, [0., 0., -1.], 6., [0., 0., 1.]);
    let lamp = [Light::Point {
        position: [-2.5, 0., -0.6],
        color: [1.; 3],
        intensity: 1.,
        range: 7.,
        attenuation: Attenuation::Smooth,
    }];
    let distinct = |rgba: &[u8]| {
        let mut values: Vec<u8> = (0..SIZE).map(|x| pixel(rgba, x, SIZE / 2)[0]).collect();
        values.sort_unstable();
        values.dedup();
        values.len()
    };
    for (model, check) in [(ShadingModel::Diffuse, true), (ShadingModel::Toon, false)] {
        let material = bench.material(model, [1.; 3]);
        let draws = [SurfaceDraw3D {
            mesh: &wall,
            model: Mat4::IDENTITY,
            material,
        }];
        bench
            .surfaces
            .set_toon(ToonSettings {
                bands: 3,
                softness: 0.,
                rim: 0.,
            })
            .unwrap();
        let rgba = bench.render(front_camera(), &lamp, &draws);
        let levels = distinct(&rgba);
        if check {
            assert!(levels > 12, "diffuse is a smooth gradient: {levels}");
        } else {
            assert!(levels <= 5, "toon has a few flat bands: {levels}");
            assert!(levels >= 2);
        }
    }
    assert!(
        bench
            .surfaces
            .set_toon(ToonSettings {
                bands: 0,
                ..ToonSettings::default()
            })
            .is_err()
    );
}

impl Bench {
    /// Renders `draws`, then `extra(scene, camera)` commands, then post.
    fn render_with(
        &mut self,
        camera: LitCamera3D,
        draws: &[SurfaceDraw3D<'_>],
        mut extra: impl FnMut(&Gpu, &SceneTarget) -> Vec<vulkanic_gal::CommandBuffer>,
    ) -> Vec<u8> {
        let settings = PostSettings::passthrough();
        let (scene, post, surfaces) = (&self.scene, &mut self.post, &mut self.surfaces);
        self.output
            .render(&self.gpu, |gpu, target| {
                let mut commands = vec![
                    surfaces
                        .prepare(
                            gpu,
                            scene.target(),
                            camera,
                            Lighting::default(),
                            draws,
                            None,
                            PassOptions {
                                clear_color: Some([0., 0., 0., 1.]),
                                clear_depth: true,
                            },
                        )?
                        .commands,
                ];
                commands.extend(extra(gpu, scene));
                commands.push(post.prepare_scene(gpu, scene, target, &settings)?);
                Ok(commands)
            })
            .unwrap()
            .rgba
    }
}

/// A ridge: two strips meeting at x = 0, raised 0.6 m towards the camera.
fn ridge(gpu: &Gpu) -> Mesh {
    let v = |x: f32, y: f32, z: f32, n: [f32; 3]| Vertex {
        position: [x, y, z],
        normal: n,
        uv: [0.5; 2],
    };
    let (left, right) = ([-0.5, 0., 1.], [0.5, 0., 1.]);
    let vertices = [
        v(-1.2, -1., -2., left),
        v(0., -1., -1.4, left),
        v(0., 1., -1.4, left),
        v(-1.2, 1., -2., left),
        v(0., -1., -1.4, right),
        v(1.2, -1., -2., right),
        v(1.2, 1., -2., right),
        v(0., 1., -1.4, right),
    ];
    Mesh::upload(gpu, &vertices, &[0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]).unwrap()
}

#[test]
#[ignore = "requires Vulkan"]
fn edges_ink_silhouettes_depth_steps_and_creases() {
    let mut bench = Bench::new();
    let mut effects = ScreenEffects::new(&bench.gpu).unwrap();
    let wall = square(&bench.gpu, [0., 0., -6.], 30., [0., 0., 1.]);
    let card = square(&bench.gpu, [-0.9, 0.9, -1.], 0.4, [0., 0., 1.]);
    let roof = ridge(&bench.gpu);
    let white = bench.material(ShadingModel::Unlit, [1.; 3]);
    let draws = [
        SurfaceDraw3D {
            mesh: &wall,
            model: Mat4::IDENTITY,
            material: white.clone(),
        },
        SurfaceDraw3D {
            mesh: &card,
            model: Mat4::IDENTITY,
            material: white.clone(),
        },
        SurfaceDraw3D {
            mesh: &roof,
            model: Mat4::IDENTITY,
            material: white,
        },
    ];
    let camera = front_camera();
    let settings = EdgeSettings::default();
    let rgba = bench.render_with(camera, &draws, |gpu, scene| {
        vec![
            effects
                .prepare_edges(gpu, scene, camera, &settings)
                .unwrap(),
        ]
    });
    let dark = |x: u32, y: u32| pixel(&rgba, x, y)[0] < 40;
    let row_dark = |y: u32, xs: std::ops::Range<u32>| xs.into_iter().any(|x| dark(x, y));
    // The ridge's crease (screen centre column) is inked; its flat strips
    // are not.
    assert!((26..38).any(|y| row_dark(y, 30..34)), "crease");
    assert!(!dark(SIZE / 2 - 8, SIZE / 2) && !dark(SIZE / 2 + 8, SIZE / 2));
    // The card's outline is a depth step against the wall behind.
    assert!(
        (0..SIZE).any(|x| (0..24).any(|y| dark(x, y))),
        "card outline"
    );
    // Open wall far from any feature stays clean.
    assert!(!dark(4, SIZE - 4));
    assert!(
        effects
            .prepare_edges(
                &bench.gpu,
                &bench.scene,
                camera,
                &EdgeSettings {
                    width: 0.,
                    ..settings
                },
            )
            .is_err()
    );
}

#[test]
#[ignore = "requires Vulkan"]
fn god_rays_stream_from_open_sky_past_silhouettes() {
    let mut bench = Bench::new();
    let mut effects = ScreenEffects::new(&bench.gpu).unwrap();
    // A black wall over the lower half; open sky (no geometry) above.
    let wall = square(&bench.gpu, [0., -2.6, -2.], 2.6, [0., 0., 1.]);
    let black = bench.material(ShadingModel::Unlit, [0.; 3]);
    let draws = [SurfaceDraw3D {
        mesh: &wall,
        model: Mat4::IDENTITY,
        material: black,
    }];
    let camera = front_camera();
    let rays = GodRays {
        source: GodRaySource::Position([0., 6., -10.]),
        intensity: 1.,
        ..GodRays::default()
    };
    let plain = bench.render_with(camera, &draws, |_, _| Vec::new());
    let lit = bench.render_with(camera, &draws, |gpu, scene| {
        vec![effects.prepare_god_rays(gpu, scene, camera, &rays).unwrap()]
    });
    let wall_y = (SIZE / 2..SIZE).find(|y| {
        pixel(&plain, SIZE / 2, *y)[3] == 255 && {
            // First wall row: below it the scene stays black without rays.
            true
        }
    });
    assert!(wall_y.is_some());
    let below_edge = pixel(&lit, SIZE / 2, SIZE / 2 + 4);
    let sky = pixel(&lit, SIZE / 2, 4);
    assert_eq!(pixel(&plain, SIZE / 2, SIZE / 2 + 4)[0], 0);
    assert!(sky[0] > 60, "open sky glows: {sky:?}");
    assert!(
        below_edge[0] > 10,
        "rays spill past the silhouette: {below_edge:?}"
    );
    // A light behind the camera adds nothing.
    let behind = GodRays {
        source: GodRaySource::Position([0., 0., 10.]),
        ..rays
    };
    let none = bench.render_with(camera, &draws, |gpu, scene| {
        vec![
            effects
                .prepare_god_rays(gpu, scene, camera, &behind)
                .unwrap(),
        ]
    });
    assert_eq!(none, plain);
}

#[test]
#[ignore = "requires Vulkan"]
fn soft_particles_fade_where_they_meet_geometry() {
    let mut bench = Bench::new();
    let wall = square(&bench.gpu, [0., 0., -1.], 10., [0., 0., 1.]);
    let black = bench.material(ShadingModel::Unlit, [0.; 3]);
    let draws = [SurfaceDraw3D {
        mesh: &wall,
        model: Mat4::IDENTITY,
        material: black,
    }];
    let settings = ParticleSettings {
        gravity: [0.; 3],
        drag: 0.,
        blend: BlendMode::Additive,
    };
    let view = Mat4::look_at([0., 0., 3.], [0., 0., 0.], [0., 1., 0.]).unwrap();
    let projection = Mat4::perspective(1., 1., 0.1, 100.).unwrap();
    let cam = ParticleCamera::from_view(view, projection);
    let camera = front_camera();
    let mut brightness = Vec::new();
    for soft in [0., 1.5] {
        let mut particles = ParticleSystem::new(&bench.gpu, 4, settings, 1).unwrap();
        particles.set_soft_distance(soft).unwrap();
        // One large puff just in front of the wall.
        particles
            .emit(&ParticleBurst {
                count: 1,
                position: [0., 0., -0.9],
                speed: [0., 0.],
                life: [5., 5.],
                size: [1.5, 1.5],
                color_start: [1., 1., 1., 1.],
                color_end: [1., 1., 1., 1.],
                ..ParticleBurst::default()
            })
            .unwrap();
        let rgba = bench.render_with(camera, &draws, |gpu, scene| {
            vec![particles.prepare(gpu, scene.target(), cam, 0.).unwrap()]
        });
        brightness.push(pixel(&rgba, SIZE / 2, SIZE / 2)[0]);
    }
    assert!(brightness[0] > 200, "hard: {brightness:?}");
    assert!(
        brightness[1] < brightness[0] / 2,
        "soft fades near the wall: {brightness:?}"
    );
    let mut particles = ParticleSystem::new(&bench.gpu, 4, settings, 1).unwrap();
    assert!(particles.set_soft_distance(-1.).is_err());
}

#[test]
#[ignore = "requires Vulkan"]
fn volumetric_fog_dims_the_scene_and_glows_around_lights() {
    let mut bench = Bench::new();
    let mut fog = VolumetricFog::new(&bench.gpu).unwrap();
    // A bright wall 6 m away, a lamp half way, uniform fog (no noise).
    let wall = square(&bench.gpu, [0., 0., -3.], 30., [0., 0., 1.]);
    let white = bench.material(ShadingModel::Unlit, [1.; 3]);
    let draws = [SurfaceDraw3D {
        mesh: &wall,
        model: Mat4::IDENTITY,
        material: white,
    }];
    let lamp = [Light::Point {
        position: [0., 0., 0.],
        color: [1., 0.7, 0.4],
        intensity: 4.,
        range: 2.,
        attenuation: Attenuation::Smooth,
    }];
    let settings = VolumetricFogSettings {
        albedo: [1.; 3],
        density: 0.25,
        base: 10.,
        falloff: 0.,
        haze: 0.,
        noise_strength: 0.,
        ambient: [0.; 3],
        anisotropy: 0.,
        steps: 48,
        ..VolumetricFogSettings::default()
    };
    let camera = front_camera();
    let clear = bench.render_with(camera, &draws, |_, _| Vec::new());
    let fogged = bench.render_with(camera, &draws, |gpu, scene| {
        vec![
            fog.prepare(
                gpu,
                scene,
                camera,
                Lighting {
                    lights: &lamp,
                    ..Lighting::default()
                },
                &settings,
                0.,
            )
            .unwrap(),
        ]
    });
    let edge = (2, 2);
    assert_eq!(pixel(&clear, edge.0, edge.1)[0], 255);
    // 6+ m of fog at 0.25/m: well under half the wall's light gets through.
    assert!(
        pixel(&fogged, edge.0, edge.1)[0] < 170,
        "{:?}",
        pixel(&fogged, edge.0, edge.1)
    );
    // Rays through the lamp's range pick up its warm light.
    let centre = pixel(&fogged, SIZE / 2, SIZE / 2);
    assert!(
        centre[0] > pixel(&fogged, edge.0, edge.1)[0] + 30,
        "{centre:?}"
    );
    assert!(centre[0] > centre[2], "warm halo: {centre:?}");
    // Noise breaks the fog into banks: the image varies across the screen.
    let banks = bench.render_with(camera, &draws, |gpu, scene| {
        vec![
            fog.prepare(
                gpu,
                scene,
                camera,
                Lighting::default(),
                &VolumetricFogSettings {
                    noise_strength: 1.,
                    noise_frequency: 0.6,
                    ..settings
                },
                3.,
            )
            .unwrap(),
        ]
    });
    let row: Vec<u8> = (0..SIZE).map(|x| pixel(&banks, x, SIZE / 2)[0]).collect();
    let spread = row.iter().max().unwrap() - row.iter().min().unwrap();
    assert!(spread > 25, "banks: {row:?}");
    assert!(
        fog.prepare(
            &bench.gpu,
            &bench.scene,
            camera,
            Lighting::default(),
            &VolumetricFogSettings {
                steps: 0,
                ..settings
            },
            0.
        )
        .is_err()
    );
}
