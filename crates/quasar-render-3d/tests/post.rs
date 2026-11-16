//! Linear HDR scene target, sorted transparent pass and post chain.
mod support;

use quasar_frame::{Gpu, OffscreenTarget, RenderTarget, read_rgba, texture_2d};
use quasar_render_3d::*;
use support::upload_color;
use vulkanic_gal::*;

const SIZE: u32 = 64;

fn plane(gpu: &Gpu) -> Mesh {
    let vertices =
        [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]].map(|position| Vertex {
            position,
            normal: [0., 0., 1.],
            uv: [0.5; 2],
        });
    Mesh::upload(gpu, &vertices, &[0, 1, 2, 0, 2, 3]).unwrap()
}

fn camera() -> LitCamera3D {
    LitCamera3D {
        view_projection: Mat4::perspective(1., 1., 0.1, 20.).unwrap()
            * Mat4::translation(0., 0., -3.),
        position: [0., 0., 3.],
    }
}

fn unlit(view: &TextureView, rgba: [f32; 4]) -> LitMaterial {
    let mut material = LitMaterial::textured(view);
    material.shading = ShadingModel::Unlit;
    material.specular = [0.; 3];
    material.base.tint = rgba;
    material
}

fn pixel(rgba: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    rgba[i..i + 4].try_into().unwrap()
}

fn srgb(linear: f32) -> f32 {
    if linear <= 0.003_130_8 {
        linear * 12.92 * 255.
    } else {
        (1.055 * linear.powf(1. / 2.4) - 0.055) * 255.
    }
}

fn near(value: u8, expected: f32, tolerance: f32) -> bool {
    (f32::from(value) - expected).abs() <= tolerance
}

struct Scene {
    gpu: Gpu,
    lit: LitRenderState3D,
    post: PostChain,
    scene: SceneTarget,
    output: OffscreenTarget,
}

/// Scene plus the shared quad mesh and white texture (kept outside so draws
/// can borrow them while the scene renders).
fn setup() -> (Scene, Mesh, Texture, TextureView) {
    let scene = Scene::new();
    let mesh = plane(&scene.gpu);
    let (texture, white) = upload_color(&scene.gpu, [255; 4]).unwrap();
    (scene, mesh, texture, white)
}

impl Scene {
    fn new() -> Self {
        let gpu = Gpu::headless().unwrap();
        Self {
            lit: LitRenderState3D::new(&gpu, 4).unwrap(),
            post: PostChain::new(&gpu).unwrap(),
            scene: SceneTarget::new(&gpu, SIZE, SIZE).unwrap(),
            output: OffscreenTarget::new(&gpu, SIZE, SIZE).unwrap(),
            gpu,
        }
    }

    /// Opaque background, transparent draws, then the post chain into the
    /// RGBA8 UNORM output, all on one device and in one submission.
    fn render(
        &mut self,
        opaque: &[LitDraw3D<'_>],
        transparent: &[TransparentDraw3D<'_>],
        settings: PostSettings,
    ) -> Vec<u8> {
        let Self {
            gpu,
            lit,
            post,
            scene,
            output,
            ..
        } = self;
        output
            .render(gpu, |gpu, target| {
                let mut commands = vec![
                    lit.prepare(
                        gpu,
                        scene.target(),
                        camera(),
                        Lighting::default(),
                        opaque,
                        PassOptions {
                            clear_color: Some([0., 0., 0., 1.]),
                            clear_depth: true,
                        },
                    )?
                    .commands,
                ];
                if !transparent.is_empty() {
                    commands.push(
                        lit.prepare_transparent(
                            gpu,
                            scene.target(),
                            camera(),
                            Lighting::default(),
                            transparent,
                            PassOptions::load(),
                        )?
                        .commands,
                    );
                }
                commands.push(post.prepare_scene(gpu, scene, target, &settings)?);
                Ok(commands)
            })
            .unwrap()
            .rgba
    }
}

#[test]
fn sorting_orders_farthest_first_and_keeps_ties_stable() {
    let vp = camera().view_projection;
    let bounds = ([-1., -1., 0.], [1., 1., 0.]);
    let items = [
        (Mat4::translation(0., 0., 0.5), bounds),
        (Mat4::translation(0., 0., -2.), bounds),
        (Mat4::translation(0., 0., -0.5), bounds),
        (Mat4::translation(0., 0., 10.), bounds), // behind the camera
        (Mat4::translation(1., 0., -0.5), bounds),
    ];
    assert_eq!(sort_back_to_front(vp, items), vec![1, 2, 4, 0, 3]);
    let ortho =
        Mat4::orthographic(-2., 2., -2., 2., 0.1, 20.).unwrap() * Mat4::translation(0., 0., -5.);
    assert_eq!(
        sort_back_to_front(ortho, [items[0], items[1]]),
        vec![1, 0],
        "orthographic depth sorts too"
    );
}

#[test]
fn post_settings_validate_ranges() {
    assert!(PostSettings::default().validate().is_ok());
    assert!(PostSettings::passthrough().validate().is_ok());
    let mut bad = PostSettings::default();
    bad.bloom.levels = 0;
    assert!(bad.validate().is_err());
    let mut bad = PostSettings::default();
    bad.vignette.strength = 1.5;
    assert!(bad.validate().is_err());
    let bad = PostSettings {
        exposure: f32::NAN,
        ..PostSettings::default()
    };
    assert!(bad.validate().is_err());
    let mut bad = PostSettings::default();
    bad.grade.gain = [1., -1., 1.];
    assert!(bad.validate().is_err());
    assert_eq!(PassOptions::load().clear_color, None);
    assert!(!PassOptions::load().clear_depth);
}

#[test]
#[ignore = "requires Vulkan"]
fn transparent_draws_blend_back_to_front_in_linear_space() {
    let (mut s, mesh, _texture, white) = setup();
    let red = unlit(&white, [1., 0., 0., 0.5]);
    let green = unlit(&white, [0., 1., 0., 0.5]);
    // Submitted near-first: only sorting puts green underneath red.
    let draws = [
        TransparentDraw3D::new(&mesh, Mat4::translation(0., 0., 0.5), red, BlendMode::Alpha),
        TransparentDraw3D::new(
            &mesh,
            Mat4::translation(0., 0., -0.5),
            green,
            BlendMode::Alpha,
        ),
    ];
    let rgba = s.render(&[], &draws, PostSettings::passthrough());
    let c = pixel(&rgba, 32, 32);
    // red*0.5 + (green*0.5 + black*0.5)*0.5 = (0.5, 0.25, 0), encoded once.
    assert!(
        near(c[0], srgb(0.5), 3.) && near(c[1], srgb(0.25), 3.) && c[2] < 3,
        "{c:?}"
    );

    // Additive draws sum regardless of order.
    let red = unlit(&white, [1., 0., 0., 1.]);
    let green = unlit(&white, [0., 1., 0., 1.]);
    let additive = [
        TransparentDraw3D::new(
            &mesh,
            Mat4::translation(0., 0., -0.5),
            red,
            BlendMode::Additive,
        ),
        TransparentDraw3D::new(
            &mesh,
            Mat4::translation(0., 0., 0.5),
            green,
            BlendMode::Additive,
        ),
    ];
    let c = pixel(
        &s.render(&[], &additive, PostSettings::passthrough()),
        32,
        32,
    );
    assert!(c[0] > 250 && c[1] > 250 && c[2] < 3, "{c:?}");

    // Depth testing still hides transparent work behind opaque surfaces.
    let wall = LitDraw3D {
        mesh: &mesh,
        model: Mat4::translation(0., 0., 0.2),
        material: unlit(&white, [0., 0., 1., 1.]),
    };
    let hidden = [TransparentDraw3D::new(
        &mesh,
        Mat4::translation(0., 0., -0.5),
        unlit(&white, [1., 0., 0., 1.]),
        BlendMode::Additive,
    )];
    let c = pixel(
        &s.render(&[wall], &hidden, PostSettings::passthrough()),
        32,
        32,
    );
    assert!(c[0] < 3 && c[2] > 250, "{c:?}");

    // In-shader sRGB output cannot be blended correctly and is rejected.
    let target = s.scene.target().clone();
    let result = s.lit.prepare_transparent(
        &s.gpu,
        &target,
        camera(),
        Lighting {
            output_color_space: ColorSpace::Srgb,
            ..Lighting::default()
        },
        &[],
        PassOptions::load(),
    );
    assert!(result.is_err());
}

#[test]
#[ignore = "requires Vulkan"]
fn the_chain_encodes_srgb_exactly_once_for_unorm_and_srgb_outputs() {
    let (mut s, mesh, _texture, white) = setup();
    let grey = [LitDraw3D {
        mesh: &mesh,
        model: Mat4::scale(3., 3., 1.),
        material: unlit(&white, [0.5, 0.5, 0.5, 1.]),
    }];
    let unorm = pixel(&s.render(&grey, &[], PostSettings::passthrough()), 32, 32);
    assert!(near(unorm[0], srgb(0.5), 2.), "{unorm:?}");

    // An sRGB-format target encodes in hardware; the raw bytes must match.
    let texture = texture_2d(
        &s.gpu.device,
        "srgb output",
        SIZE,
        SIZE,
        TextureFormat::Rgba8UnormSrgb,
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
    )
    .unwrap();
    let target = RenderTarget::from_texture(&texture);
    let commands = s
        .post
        .prepare_scene(&s.gpu, &s.scene, &target, &PostSettings::passthrough())
        .unwrap();
    s.gpu.queue.submit([commands]);
    let srgb_bytes = read_rgba(&s.gpu, &texture).unwrap();
    let c = pixel(&srgb_bytes, 32, 32);
    assert!(
        (i16::from(c[0]) - i16::from(unorm[0])).abs() <= 1,
        "{c:?} vs {unorm:?}"
    );

    // Forcing linear output writes the raw value.
    let linear = PostSettings {
        output: OutputEncoding::Linear,
        ..PostSettings::passthrough()
    };
    let c = pixel(&s.render(&grey, &[], linear), 32, 32);
    assert!(near(c[0], 127.5, 2.), "{c:?}");
}

#[test]
#[ignore = "requires Vulkan"]
fn tonemapping_exposure_and_grading_map_hdr_values() {
    let (mut s, mesh, _texture, white) = setup();
    let hot = [LitDraw3D {
        mesh: &mesh,
        model: Mat4::scale(3., 3., 1.),
        material: unlit(&white, [4., 4., 4., 1.]),
    }];
    let settings = |tonemap| PostSettings {
        tonemap,
        bloom: BloomSettings {
            enabled: false,
            ..Default::default()
        },
        ..PostSettings::default()
    };
    let clamp = pixel(&s.render(&hot, &[], settings(Tonemap::None)), 32, 32)[0];
    let aces = pixel(&s.render(&hot, &[], settings(Tonemap::Aces)), 32, 32)[0];
    let reinhard = pixel(&s.render(&hot, &[], settings(Tonemap::Reinhard)), 32, 32)[0];
    assert_eq!(clamp, 255);
    assert!(
        aces < 255 && aces > 230,
        "ACES keeps highlight detail: {aces}"
    );
    assert!(near(reinhard, srgb(0.8), 3.), "{reinhard}");
    let dark = PostSettings {
        exposure: 0.05,
        ..settings(Tonemap::Reinhard)
    };
    let dim = pixel(&s.render(&hot, &[], dark), 32, 32)[0];
    assert!(
        near(dim, srgb(0.2 / 1.2), 3.),
        "exposure applies before tonemap: {dim}"
    );
    let grey = PostSettings {
        grade: ColorGrade {
            saturation: 0.0,
            ..Default::default()
        },
        ..PostSettings::passthrough()
    };
    let colorful = [LitDraw3D {
        mesh: &mesh,
        model: Mat4::scale(3., 3., 1.),
        material: unlit(&white, [1., 0., 0., 1.]),
    }];
    let c = pixel(&s.render(&colorful, &[], grey), 32, 32);
    assert!(
        c[0] == c[1] && c[1] == c[2] && c[0] > 0,
        "desaturated: {c:?}"
    );
    let tinted = PostSettings {
        grade: ColorGrade {
            gain: [0., 1., 1.],
            ..Default::default()
        },
        ..PostSettings::passthrough()
    };
    assert_eq!(pixel(&s.render(&colorful, &[], tinted), 32, 32)[0], 0);
}

#[test]
#[ignore = "requires Vulkan"]
fn bloom_spreads_bright_pixels_and_vignette_darkens_corners() {
    let (mut s, mesh, _texture, white) = setup();
    let spark = [LitDraw3D {
        mesh: &mesh,
        model: Mat4::scale(0.05, 0.05, 1.),
        material: unlit(&white, [8., 8., 8., 1.]),
    }];
    let no_bloom = PostSettings {
        bloom: BloomSettings {
            enabled: false,
            ..Default::default()
        },
        tonemap: Tonemap::Reinhard,
        ..PostSettings::default()
    };
    let with_bloom = PostSettings {
        bloom: BloomSettings {
            intensity: 1.0,
            ..Default::default()
        },
        tonemap: Tonemap::Reinhard,
        ..PostSettings::default()
    };
    let plain = s.render(&spark, &[], no_bloom);
    let glow = s.render(&spark, &[], with_bloom);
    assert!(pixel(&plain, 32, 32)[0] > 200, "the spark itself is bright");
    assert_eq!(pixel(&plain, 40, 32)[0], 0, "no glow without bloom");
    assert!(
        pixel(&glow, 40, 32)[0] > 5,
        "bloom spreads: {:?}",
        pixel(&glow, 40, 32)
    );
    assert!(
        pixel(&glow, 40, 32)[0] > pixel(&glow, 60, 32)[0],
        "and falls off"
    );
    assert!(s.post.bloom_view().is_some());

    let grey = [LitDraw3D {
        mesh: &mesh,
        model: Mat4::scale(3., 3., 1.),
        material: unlit(&white, [0.5, 0.5, 0.5, 1.]),
    }];
    let vignette = PostSettings {
        vignette: VignetteSettings {
            strength: 0.8,
            ..Default::default()
        },
        ..PostSettings::passthrough()
    };
    let flat = s.render(&grey, &[], PostSettings::passthrough());
    let shaded = s.render(&grey, &[], vignette);
    assert_eq!(
        pixel(&shaded, 32, 32),
        pixel(&flat, 32, 32),
        "centre unchanged"
    );
    assert!(
        pixel(&shaded, 1, 1)[0] + 40 < pixel(&flat, 1, 1)[0],
        "corners darken"
    );

    // Resizing the scene reallocates bloom levels on the next frame.
    assert!(s.scene.resize(&s.gpu, 32, 32).unwrap());
    assert!(!s.scene.resize(&s.gpu, 32, 32).unwrap());
    let output = OffscreenTarget::new(&s.gpu, 32, 32).unwrap();
    let Scene {
        gpu, post, scene, ..
    } = &mut s;
    let small = output
        .render(gpu, |gpu, target| {
            Ok(vec![post.prepare_scene(gpu, scene, target, &with_bloom)?])
        })
        .unwrap();
    assert_eq!(small.rgba.len(), 32 * 32 * 4);
    post.release(gpu).unwrap();
}
