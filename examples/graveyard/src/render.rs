//! The graveyard scene: geometry, lights, volumetric fog and post.
use crate::world::{HALF, Kind, Piece, World};
use quasar_core::noise::{Fractal, Noise};
use quasar_frame::{Gpu, GraphicsResult, RenderTarget, texture_2d};
use quasar_render_3d::{
    Attenuation, ColorGrade, CookiePattern, Light, LightClusters, LightCookie, Lighting,
    LitCamera3D, LitMaterial, Mat4, Mesh, MeshData, PassOptions, PostChain, PostSettings,
    SceneTarget, ShadingModel, SurfaceDraw3D, SurfaceMaterial, SurfaceRenderState3D, Tonemap,
    VignetteSettings, VolumetricFog, VolumetricFogSettings,
};
use vulkanic_gal::{CommandBuffer, Texture, TextureView};

/// Night sky (linear). The volumetric fog fills the low sky near the
/// horizon, so the edge of the world dissolves into lit fog.
pub const SKY: [f32; 3] = [0.003, 0.004, 0.008];
const MOON: [f32; 3] = [0.55, 0.65, 0.95];
const MOON_INTENSITY: f32 = 0.25;

/// The graveyard fog: a dense layer on the ground, thinning by head height,
/// broken into drifting banks, lit by the moon, the lanterns and the
/// flashlight.
pub fn fog() -> VolumetricFogSettings {
    VolumetricFogSettings {
        albedo: [0.85, 0.88, 0.95],
        density: 0.3,
        base: 0.0,
        falloff: 0.85,
        haze: 0.012,
        noise_frequency: 0.16,
        noise_strength: 0.85,
        wind: [0.9, 0.0, 0.3],
        anisotropy: 0.4,
        ambient: [0.006, 0.008, 0.013],
        steps: 48,
        max_distance: 70.0,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Cube,
    Sphere,
    Cylinder,
    Cone,
    Pyramid,
    Ground,
}

#[derive(Clone, Copy, PartialEq)]
enum Surface {
    Stone([f32; 3]),
    Wood,
    Iron,
    Ground,
    Path,
    Glow,
    Shadow,
}

struct Part {
    shape: Shape,
    model: Mat4,
    surface: Surface,
}

const LANTERN_HEIGHT: f32 = 2.3;

fn place(piece: &Piece) -> Mat4 {
    Mat4::translation(piece.at[0], 0.0, piece.at[1])
        * Mat4::rotation_y(piece.yaw)
        * Mat4::rotation_x(piece.tilt)
        * Mat4::scale(piece.scale, piece.scale, piece.scale)
}

/// A unit shape stretched to `size` with its centre at `at` (local space).
fn local(at: [f32; 3], size: [f32; 3]) -> Mat4 {
    Mat4::translation(at[0], at[1], at[2]) * Mat4::scale(size[0], size[1], size[2])
}

fn parts(pieces: &[Piece]) -> Vec<Part> {
    let mut out = Vec::new();
    let stone = |shade: f32| Surface::Stone([0.42 * shade, 0.43 * shade, 0.45 * shade]);
    let mut push = |shape, model, surface| {
        out.push(Part {
            shape,
            model,
            surface,
        })
    };
    // Ground, the gravel path and the walls' fence.
    // Ground in 8 m tiles so its texture keeps its detail.
    let reach = HALF + 32.0;
    let mut gx = -reach;
    while gx < reach {
        let mut gz = -reach;
        while gz < reach {
            push(
                Shape::Ground,
                local([gx + 4.0, 0.0, gz + 4.0], [8.0, 1.0, 8.0]),
                Surface::Ground,
            );
            gz += 8.0;
        }
        gx += 8.0;
    }
    // The gravel path, in 2 m flags so its texture tiles.
    let mut z = -HALF + 3.0;
    while z < HALF - 1.0 {
        push(
            Shape::Cube,
            local([0.0, 0.01, z + 1.0], [2.0, 0.02, 1.96]),
            Surface::Path,
        );
        z += 2.0;
    }
    let mut post = 0;
    let mut x = -HALF;
    while x <= HALF + 0.01 {
        for (px, pz) in [(x, -HALF), (x, HALF), (-HALF, x), (HALF, x)] {
            post += 1;
            push(
                Shape::Cylinder,
                local([px, 0.85, pz], [0.09, 1.7, 0.09]),
                Surface::Iron,
            );
            push(
                Shape::Cone,
                local([px, 1.7, pz], [0.12, 0.25, 0.12]),
                Surface::Iron,
            );
        }
        x += 2.0;
    }
    let _ = post;
    for height in [0.45, 1.4] {
        for (at, size) in [
            ([0.0, height, -HALF], [2.0 * HALF, 0.05, 0.05]),
            ([0.0, height, HALF], [2.0 * HALF, 0.05, 0.05]),
            ([-HALF, height, 0.0], [0.05, 0.05, 2.0 * HALF]),
            ([HALF, height, 0.0], [0.05, 0.05, 2.0 * HALF]),
        ] {
            push(Shape::Cube, local(at, size), Surface::Iron);
        }
    }
    for piece in pieces {
        let m = place(piece);
        let shade = 0.75 + (piece.at[0] * 3.1 + piece.at[1] * 1.7).sin().abs() * 0.45;
        match piece.kind {
            Kind::Headstone => {
                push(
                    Shape::Cube,
                    m * local([0.0, 0.5, 0.0], [0.8, 1.0, 0.16]),
                    stone(shade),
                );
                push(
                    Shape::Sphere,
                    m * local([0.0, 1.0, 0.0], [0.8, 0.45, 0.16]),
                    stone(shade),
                );
                // A low earth mound in front.
                push(
                    Shape::Sphere,
                    Mat4::translation(piece.at[0], -0.05, piece.at[1] + 0.9)
                        * Mat4::rotation_y(piece.yaw)
                        * Mat4::scale(0.8, 0.25, 1.6),
                    Surface::Ground,
                );
            }
            Kind::Cross => {
                push(
                    Shape::Cube,
                    m * local([0.0, 0.1, 0.0], [0.5, 0.2, 0.3]),
                    stone(shade),
                );
                push(
                    Shape::Cube,
                    m * local([0.0, 0.85, 0.0], [0.13, 1.5, 0.13]),
                    stone(shade),
                );
                push(
                    Shape::Cube,
                    m * local([0.0, 1.15, 0.0], [0.75, 0.13, 0.13]),
                    stone(shade),
                );
            }
            Kind::Slab => {
                push(
                    Shape::Cube,
                    m * local([0.0, 0.14, 0.0], [1.0, 0.28, 2.1]),
                    stone(shade * 0.9),
                );
                push(
                    Shape::Cube,
                    m * local([0.0, 0.5, -0.95], [0.7, 0.75, 0.14]),
                    stone(shade),
                );
            }
            Kind::Obelisk => {
                push(
                    Shape::Cube,
                    m * local([0.0, 0.25, 0.0], [0.8, 0.5, 0.8]),
                    stone(shade),
                );
                push(
                    Shape::Pyramid,
                    m * local([0.0, 0.5, 0.0], [0.45, 2.6, 0.45]),
                    stone(shade),
                );
            }
            Kind::Tree => {
                push(
                    Shape::Cone,
                    m * local([0.0, 0.0, 0.0], [0.45, 4.5, 0.45]),
                    Surface::Wood,
                );
                for (i, height) in [1.8f32, 2.4, 3.0, 3.4].into_iter().enumerate() {
                    let turn = i as f32 * 2.1;
                    let branch = m
                        * Mat4::translation(0.0, height, 0.0)
                        * Mat4::rotation_y(turn)
                        * Mat4::rotation_x(0.95 - i as f32 * 0.12)
                        * local([0.0, 0.8, 0.0], [0.08, 1.6, 0.08]);
                    push(
                        Shape::Cone,
                        branch * local([0.0, -0.5, 0.0], [1.0, 1.0, 1.0]),
                        Surface::Wood,
                    );
                }
            }
            Kind::Lantern => {
                push(
                    Shape::Cylinder,
                    m * local([0.0, 1.05, 0.0], [0.1, 2.1, 0.1]),
                    Surface::Iron,
                );
                push(
                    Shape::Cube,
                    m * local([0.0, LANTERN_HEIGHT, 0.0], [0.34, 0.42, 0.34]),
                    Surface::Iron,
                );
                push(
                    Shape::Cube,
                    m * local([0.0, LANTERN_HEIGHT, 0.0], [0.36, 0.3, 0.26]),
                    Surface::Glow,
                );
                push(
                    Shape::Pyramid,
                    m * local([0.0, LANTERN_HEIGHT + 0.21, 0.0], [0.5, 0.25, 0.5]),
                    Surface::Iron,
                );
            }
            Kind::Mausoleum => {
                let dark = stone(0.7);
                push(
                    Shape::Cube,
                    m * local([0.0, 0.15, 0.4], [6.0, 0.3, 5.4]),
                    dark,
                );
                push(
                    Shape::Cube,
                    m * local([0.0, 1.8, 0.0], [5.0, 3.0, 4.2]),
                    stone(0.85),
                );
                push(
                    Shape::Pyramid,
                    m * local([0.0, 3.3, 0.0], [6.2, 1.6, 5.0]),
                    dark,
                );
                push(
                    Shape::Cube,
                    m * local([0.0, 1.4, 2.12], [1.3, 2.2, 0.05]),
                    Surface::Shadow,
                );
                for x in [-2.2f32, 2.2] {
                    push(
                        Shape::Cylinder,
                        m * local([x, 1.7, 2.35], [0.45, 2.8, 0.45]),
                        stone(1.0),
                    );
                }
            }
        }
    }
    out
}

fn lantern_lights(pieces: &[Piece]) -> Vec<Light> {
    pieces
        .iter()
        .filter(|p| p.kind == Kind::Lantern)
        .map(|p| Light::Point {
            position: [p.at[0], LANTERN_HEIGHT, p.at[1]],
            color: [1.0, 0.6, 0.26],
            intensity: 1.4,
            range: 7.0,
            attenuation: Attenuation::Smooth,
        })
        .collect()
}

fn texture(
    gpu: &Gpu,
    label: &str,
    size: u32,
    pixels: &[u8],
) -> GraphicsResult<(Texture, TextureView)> {
    use vulkanic_gal::{TexelCopyBufferLayout, TextureFormat, TextureUsages};
    let texture = texture_2d(
        &gpu.device,
        label,
        size,
        size,
        TextureFormat::Rgba8Unorm,
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
    )?;
    gpu.queue.write_texture(
        texture.as_image_copy(),
        pixels,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size * 4),
            rows_per_image: Some(size),
        },
        texture.size(),
    );
    let view = texture.create_view(&Default::default());
    Ok((texture, view))
}

/// Linear-light texels from a colour function over `[0, 1)²`.
fn paint(size: u32, colour: impl Fn(f32, f32) -> [f32; 3]) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let c = colour(x as f32 / size as f32, y as f32 / size as f32);
            pixels.extend(c.map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8));
            pixels.push(255);
        }
    }
    pixels
}

/// Smooth value noise over `[0, 1)²` that repeats every unit (a lattice of
/// `period` cells with wrapped corners).
fn tiled_noise(u: f32, v: f32, period: u32, seed: u32) -> f32 {
    let (x, y) = (u * period as f32, v * period as f32);
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - ix as f32, y - iy as f32);
    let p = period as i32;
    let corner = |dx: i32, dy: i32| {
        let h = quasar_core::noise::hash2(seed, (ix + dx).rem_euclid(p), (iy + dy).rem_euclid(p));
        quasar_core::noise::unit_f32(h)
    };
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let top = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * sx;
    let bottom = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * sx;
    top + (bottom - top) * sy
}

pub struct Graphics {
    surfaces: SurfaceRenderState3D,
    fog: VolumetricFog,
    clock: f32,
    post: PostChain,
    scene: Option<SceneTarget>,
    meshes: Vec<(Shape, Mesh)>,
    parts: Vec<Part>,
    lanterns: Vec<Light>,
    _textures: Vec<Texture>,
    stone: TextureView,
    ground: TextureView,
}

impl Graphics {
    pub fn new(gpu: &Gpu, world: &World) -> GraphicsResult<Self> {
        let mut surfaces = SurfaceRenderState3D::new(gpu, 16, 0)?;
        surfaces.set_light_clusters(LightClusters {
            min_lights: 4,
            cell_size: 6.0,
            dims: [10, 3, 10],
        })?;
        let noise = Noise::new(7);
        let stone_pixels = paint(64, |u, v| {
            let n = noise.value_fbm2(u * 16.0, v * 16.0, Fractal::new(1.0, 4));
            let moss = noise.derive(1).value2(u * 9.0, v * 9.0);
            let grey = 0.55 + (n - 0.5) * 0.5;
            if moss > 0.72 {
                [grey * 0.55, grey * 0.7, grey * 0.45]
            } else {
                [grey; 3]
            }
        });
        // Tileable ground: periodic noise, so 8 m tiles meet seamlessly.
        let ground_pixels = paint(256, |u, v| {
            let n = (0..4)
                .map(|o| tiled_noise(u, v, 24 << o, 20 + o) / (1 << o) as f32)
                .sum::<f32>()
                / 1.875;
            let patch = tiled_noise(u, v, 3, 9);
            let grass = [0.09 + n * 0.08, 0.13 + n * 0.1, 0.07 + n * 0.05];
            let dirt = [0.12 + n * 0.08, 0.1 + n * 0.06, 0.08 + n * 0.04];
            let t = ((patch - 0.35) * 3.0).clamp(0.0, 1.0);
            std::array::from_fn(|i| grass[i] + (dirt[i] - grass[i]) * t)
        });
        let (stone_texture, stone) = texture(gpu, "graveyard stone", 64, &stone_pixels)?;
        let (ground_texture, ground) = texture(gpu, "graveyard ground", 256, &ground_pixels)?;
        let meshes = [
            (Shape::Cube, MeshData::cuboid([0.5; 3])),
            (Shape::Sphere, MeshData::sphere(0.5, 12, 18)),
            (Shape::Cylinder, MeshData::cylinder(0.5, 0.5, 12)),
            (Shape::Cone, MeshData::cone(0.5, 1.0, 10)),
            (
                Shape::Pyramid,
                MeshData::cone(0.72, 1.0, 4)
                    .transformed(Mat4::rotation_y(std::f32::consts::FRAC_PI_4)),
            ),
            (Shape::Ground, MeshData::plane(0.5, 0.5, 1)),
        ]
        .into_iter()
        .map(|(shape, data)| Ok((shape, data.upload(gpu)?)))
        .collect::<GraphicsResult<Vec<_>>>()?;
        Ok(Self {
            surfaces,
            fog: VolumetricFog::new(gpu)?,
            clock: 0.0,
            post: PostChain::new(gpu)?,
            scene: None,
            meshes,
            parts: parts(&world.pieces),
            lanterns: lantern_lights(&world.pieces),
            _textures: vec![stone_texture, ground_texture],
            stone,
            ground,
        })
    }

    fn material(stone: &TextureView, ground: &TextureView, surface: Surface) -> SurfaceMaterial {
        let view = if matches!(surface, Surface::Ground) {
            ground
        } else {
            stone
        };
        let mut lit = LitMaterial::textured(view);
        lit.shading = ShadingModel::Diffuse;
        lit.specular = [0.0; 3];
        lit.base.tint = match surface {
            Surface::Stone(rgb) => [rgb[0], rgb[1], rgb[2], 1.0],
            Surface::Wood => [0.09, 0.075, 0.06, 1.0],
            Surface::Iron => [0.05, 0.05, 0.055, 1.0],
            Surface::Ground => [1.0; 4],
            Surface::Path => [0.32, 0.3, 0.28, 1.0],
            Surface::Shadow => [0.01, 0.01, 0.012, 1.0],
            Surface::Glow => [0.0, 0.0, 0.0, 1.0],
        };
        if surface == Surface::Glow {
            lit.shading = ShadingModel::Unlit;
            lit.emissive = [4.0, 2.4, 0.9];
        }
        lit.into()
    }

    /// Advances the fog drift (so a screenshot shows settled banks).
    pub fn prewarm(&mut self, seconds: f32) {
        self.clock += seconds;
    }

    fn camera(world: &World, target: &RenderTarget) -> (Mat4, Mat4, [f32; 3]) {
        let eye = world.eye();
        let f = world.forward();
        let view = Mat4::look_at(
            eye,
            [eye[0] + f[0], eye[1] + f[1], eye[2] + f[2]],
            [0.0, 1.0, 0.0],
        )
        .unwrap_or(Mat4::IDENTITY);
        let aspect = target.width as f32 / target.height.max(1) as f32;
        let projection =
            Mat4::perspective(1.05, aspect.max(0.1), 0.05, 140.0).unwrap_or(Mat4::IDENTITY);
        (view, projection, eye)
    }

    /// Records one frame into `target`.
    pub fn frame(
        &mut self,
        gpu: &Gpu,
        target: &RenderTarget,
        world: &World,
        dt: f32,
    ) -> GraphicsResult<Vec<CommandBuffer>> {
        let (view, projection, eye) = Self::camera(world, target);
        let scene = match self.scene.take() {
            Some(mut scene) => {
                scene.resize(gpu, target.width, target.height)?;
                scene
            }
            None => SceneTarget::new(gpu, target.width, target.height)?,
        };
        let result = self.record(gpu, target, world, dt, &scene, view, projection, eye);
        self.scene = Some(scene);
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn record(
        &mut self,
        gpu: &Gpu,
        target: &RenderTarget,
        world: &World,
        dt: f32,
        scene: &SceneTarget,
        view: Mat4,
        projection: Mat4,
        eye: [f32; 3],
    ) -> GraphicsResult<Vec<CommandBuffer>> {
        let mut lights = vec![Light::Directional {
            direction: [0.35, -0.6, 0.45],
            color: MOON,
            intensity: MOON_INTENSITY,
        }];
        lights.extend(self.lanterns.iter().copied());
        if world.flashlight {
            let f = world.forward();
            lights.push(Light::Spot {
                position: [eye[0], eye[1] - 0.2, eye[2]],
                direction: f,
                color: [0.92, 0.95, 1.0],
                intensity: 0.7,
                range: 24.0,
                inner_angle: 0.16,
                outer_angle: 0.4,
                attenuation: Attenuation::Smooth,
                cookie: LightCookie::new(CookiePattern::Rings, 3, 0.3),
            });
        }
        let (meshes, stone, ground) = (&self.meshes, &self.stone, &self.ground);
        let draws: Vec<SurfaceDraw3D<'_>> = self
            .parts
            .iter()
            .map(|part| SurfaceDraw3D {
                mesh: &meshes
                    .iter()
                    .find(|(s, _)| *s == part.shape)
                    .expect("mesh")
                    .1,
                model: part.model,
                material: Self::material(stone, ground, part.surface),
            })
            .collect();
        let camera = LitCamera3D {
            view_projection: projection * view,
            position: eye,
        };
        let opaque = self
            .surfaces
            .prepare(
                gpu,
                scene.target(),
                camera,
                Lighting {
                    ambient: [0.03, 0.035, 0.05],
                    lights: &lights,
                    output_color_space: quasar_render_3d::ColorSpace::Linear,
                },
                &draws,
                None,
                PassOptions {
                    clear_color: Some([SKY[0], SKY[1], SKY[2], 1.0]),
                    clear_depth: true,
                },
            )?
            .commands;
        drop(draws);
        self.clock += dt;
        let lighting = Lighting {
            ambient: [0.0; 3],
            lights: &lights,
            output_color_space: quasar_render_3d::ColorSpace::Linear,
        };
        let fog = self
            .fog
            .prepare(gpu, scene, camera, lighting, &fog(), self.clock)?;
        let post = PostSettings {
            exposure: 1.0,
            tonemap: Tonemap::Aces,
            vignette: VignetteSettings {
                strength: 0.5,
                radius: 0.45,
                softness: 0.55,
            },
            grade: ColorGrade {
                saturation: 0.7,
                contrast: 1.08,
                gain: [0.95, 1.0, 1.08],
                lift: [0.0, 0.0, 0.005],
            },
            ..PostSettings::default()
        };
        let composite = self.post.prepare_scene(gpu, scene, target, &post)?;
        Ok(vec![opaque, fog, composite])
    }
}
