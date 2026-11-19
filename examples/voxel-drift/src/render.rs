//! Voxels (asteroid LOD nodes and ship grids) and meshes on one HDR scene,
//! composed in order by `quasar_render_voxel::HybridFrame`: opaque voxels,
//! the drone mesh, scorch decals on dug rock, dust and spark particles,
//! the drone's see-through outline, the post chain and the 2D HUD.
use crate::GeometryUpdate;
use crate::content::{TILE, mat, texture_layers};
use crate::world::{DriftWorld, Effect, Target};
use quasar_frame::{Gpu, GraphicsResult, RenderTarget, texture_2d};
use quasar_math::Vec3;
use quasar_profile::RendererStats;
use quasar_render_2d::{Canvas, Color, DisplayOptions, LoadOp, Rect, RenderState2D, Textures};
use quasar_render_3d::{
    BlendMode, ColorGrade, Decal, DecalRenderer, EdgeSettings, GodRaySource, GodRays, Light,
    LitCamera3D, LitMaterial, Mat4, Mesh, MeshData, OutlineDraw, OutlineRenderer, OutlineStyle,
    ParticleBurst, ParticleCamera, ParticleSettings, ParticleSystem, PostChain, PostSettings,
    SceneLook, SceneTarget, ScreenEffects, ShadingModel, SurfaceDraw3D, SurfaceMaterial,
    SurfaceRenderState3D, ToonSettings,
};
use quasar_render_voxel::{
    ChunkKey, ChunkPlacement, HybridFrame, MaterialDef, MaterialFilter, OverlayBox, Stage,
    VoxelCamera, VoxelMaterials, VoxelPassOptions, VoxelRenderer, VoxelRendererConfig,
    VoxelShading, VoxelStats,
};
use vulkanic_gal::{CommandBuffer, Texture, TextureView};

const SPACE: [f32; 3] = [0.004, 0.005, 0.012];
/// Direction the sunlight travels.
const SUN: [f32; 3] = [-0.55, -0.35, -0.75];
/// Scorch marks kept on the asteroid (oldest replaced first).
const MAX_DECALS: usize = 48;

pub struct Graphics {
    voxels: VoxelRenderer,
    surfaces: SurfaceRenderState3D,
    particles: ParticleSystem,
    /// Sun shafts past the asteroid's silhouette, and ink edges.
    screen: ScreenEffects,
    /// The scene's sun, fill, grade and style.
    pub look: SceneLook,
    decals: DecalRenderer,
    outlines: OutlineRenderer,
    post: PostChain,
    scene: Option<SceneTarget>,
    drone: Mesh,
    _white: Texture,
    white_view: TextureView,
    _scorch: Texture,
    scorch_view: TextureView,
    scorches: Vec<Vec3>,
    frame: u64,
    state_2d: RenderState2D,
    textures: Textures,
    stats: RendererStats,
    pub voxel_stats: VoxelStats,
    pending_submissions: Vec<u64>,
    pub prepare_micros: f64,
}

fn rgba_texture(
    gal: &Gpu,
    label: &str,
    size: u32,
    pixels: &[u8],
) -> GraphicsResult<(Texture, TextureView)> {
    use vulkanic_gal::{TexelCopyBufferLayout, TextureFormat, TextureUsages};
    let texture = texture_2d(
        &gal.device,
        label,
        size,
        size,
        TextureFormat::Rgba8Unorm,
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
    )?;
    gal.queue.write_texture(
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

/// A soft, slightly ragged dark ring: the burn left by the mining laser.
fn scorch_pixels(size: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let u = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let r = (u * u + v * v).sqrt();
            let angle = v.atan2(u);
            let ragged = 0.85 + 0.08 * (angle * 7.0).sin() + 0.05 * (angle * 13.0).cos();
            let alpha = ((ragged - r) / 0.25).clamp(0.0, 1.0) * 0.85;
            let glow = ((0.35 - r) / 0.35).clamp(0.0, 1.0);
            pixels.extend([
                (40.0 + 120.0 * glow) as u8,
                (22.0 + 40.0 * glow) as u8,
                12,
                (alpha * 255.0) as u8,
            ]);
        }
    }
    pixels
}

/// Where the survey drone is at frame `frame`: a slow orbit that passes
/// behind the asteroid, so its outline shows through the rock.
pub fn drone_position(frame: u64) -> Vec3 {
    let t = frame as f32 * 0.01;
    Vec3::new(t.cos() * 46.0, 18.0 + (t * 1.7).sin() * 6.0, t.sin() * 46.0)
}

fn materials(gal: &Gpu) -> GraphicsResult<VoxelMaterials> {
    let defs: Vec<MaterialDef> = (0..mat::COUNT)
        .map(|layer| {
            let mut def = MaterialDef::layer(layer);
            def.triplanar_scale = 0.18;
            match layer {
                mat::LIGHT => def.with_emissive([0.9, 0.85, 0.6]),
                mat::THRUSTER => def.with_emissive([0.25, 0.1, 0.02]),
                _ => def,
            }
        })
        .collect();
    VoxelMaterials::new(gal, TILE, &texture_layers(), &defs, MaterialFilter::Nearest)
}

impl Graphics {
    pub fn new(gal: &Gpu) -> GraphicsResult<Self> {
        let voxels = VoxelRenderer::new(gal, materials(gal)?, VoxelRendererConfig::default())?;
        let (white, white_view) = rgba_texture(gal, "drift white", 1, &[255; 4])?;
        let (scorch, scorch_view) = rgba_texture(gal, "drift scorch", 64, &scorch_pixels(64))?;
        let particles = ParticleSystem::new(
            gal,
            4096,
            ParticleSettings {
                gravity: [0.0; 3],
                drag: 0.8,
                blend: BlendMode::Additive,
            },
            0xd51f7,
        )?;
        // Soft dust: puffs fade where they meet rock instead of clipping.
        let mut particles = particles;
        particles.set_soft_distance(1.5)?;
        let mut drone = MeshData::sphere(1.2, 10, 16);
        drone.append(&MeshData::cuboid([2.2, 0.12, 0.5]));
        Ok(Self {
            voxels,
            surfaces: SurfaceRenderState3D::new(gal, 4, 0)?,
            particles,
            screen: ScreenEffects::new(gal)?,
            look: look(false),
            decals: DecalRenderer::new(gal)?,
            outlines: OutlineRenderer::new(gal)?,
            post: PostChain::new(gal)?,
            scene: None,
            drone: drone.upload(gal)?,
            _white: white,
            white_view,
            _scorch: scorch,
            scorch_view,
            scorches: Vec::new(),
            frame: 0,
            state_2d: RenderState2D::new(gal)?,
            textures: Textures::default(),
            stats: RendererStats::default(),
            voxel_stats: VoxelStats::default(),
            pending_submissions: Vec::new(),
            prepare_micros: 0.0,
        })
    }

    /// Switches between the standard and the cel-shaded look.
    pub fn set_toon(&mut self, toon: bool) {
        self.look = look(toon);
    }

    /// Removes every particle and scorch mark at once.
    pub fn clear_effects(&mut self) {
        self.particles.clear();
        self.scorches.clear();
    }

    /// Turns world events into particles and decals.
    fn spawn_effects(&mut self, world: &mut DriftWorld) -> GraphicsResult<()> {
        for effect in world.take_effects() {
            match effect {
                Effect::Dig { point, fill } => {
                    let color = if fill {
                        [0.5, 0.8, 1.0, 0.9]
                    } else {
                        [1.0, 0.7, 0.35, 0.9]
                    };
                    self.particles.emit(&ParticleBurst {
                        position: point.to_array(),
                        position_jitter: 2.5,
                        count: 160,
                        speed: [1.0, 6.0],
                        life: [0.6, 1.6],
                        size: [0.35, 0.08],
                        color_start: color,
                        color_end: [color[0] * 0.5, color[1] * 0.4, color[2] * 0.3, 0.0],
                        ..ParticleBurst::default()
                    })?;
                    if !fill {
                        if self.scorches.len() == MAX_DECALS {
                            self.scorches.remove(0);
                        }
                        self.scorches.push(point);
                    }
                }
                Effect::BlockRemoved { point } => {
                    self.particles.emit(&ParticleBurst {
                        position: point.to_array(),
                        position_jitter: 0.4,
                        count: 90,
                        speed: [3.0, 9.0],
                        life: [0.3, 0.8],
                        size: [0.12, 0.02],
                        color_start: [3.0, 2.2, 1.0, 1.0],
                        color_end: [2.0, 0.3, 0.05, 0.0],
                        ..ParticleBurst::default()
                    })?;
                }
            }
        }
        Ok(())
    }

    pub fn last_stats(&self) -> RendererStats {
        self.stats
    }

    pub fn sync(&mut self, gal: &Gpu, world: &mut DriftWorld) -> GraphicsResult<()> {
        self.voxels.retire_completed(gal);
        for update in world.take_updates() {
            match update {
                GeometryUpdate::Grid {
                    grid,
                    transform,
                    cell_size,
                } => self.voxels.set_grid(grid, transform, cell_size),
                GeometryUpdate::Upload {
                    grid,
                    chunk,
                    level,
                    origin,
                    scale,
                    mesh,
                } => {
                    let key = ChunkKey { grid, chunk, level };
                    self.voxels
                        .upload(gal, key, ChunkPlacement { origin, scale }, &mesh)?;
                }
                GeometryUpdate::Remove { grid, chunk, level } => {
                    self.voxels.remove(ChunkKey { grid, chunk, level });
                }
            }
        }
        Ok(())
    }

    pub fn submitted(&mut self, gal: &Gpu) {
        let pending = std::mem::take(&mut self.pending_submissions);
        quasar_render_voxel::HybridCommands::submitted(&pending, gal, &mut self.voxels);
    }

    pub fn finish(&mut self, gal: &Gpu) -> GraphicsResult<()> {
        self.pending_submissions.clear();
        self.voxels.wait_and_retire(gal)
    }

    pub fn commands(
        &mut self,
        gal: &Gpu,
        target: &RenderTarget,
        world: &mut DriftWorld,
        hud: bool,
    ) -> GraphicsResult<Vec<CommandBuffer>> {
        self.sync(gal, world)?;
        let started = std::time::Instant::now();
        let aspect = target.width as f32 / target.height.max(1) as f32;
        let projection = quasar_math::perspective(70f32.to_radians(), aspect, 0.1, 2000.0)
            .ok_or_else(|| quasar_frame::error("invalid projection"))?;
        let camera = VoxelCamera::looking(
            world.camera.position,
            world.camera.forward(),
            Vec3::Y,
            projection,
        )
        .ok_or_else(|| quasar_frame::error("invalid camera"))?;
        self.spawn_effects(world)?;
        self.frame += 1;
        let scene = match &mut self.scene {
            Some(scene) => {
                scene.resize(gal, target.width, target.height)?;
                scene
            }
            none => none.insert(SceneTarget::new(gal, target.width, target.height)?),
        };
        let look = self.look;
        let toon = look.edges.is_some();
        let model = if toon {
            ShadingModel::Toon
        } else {
            ShadingModel::Diffuse
        };
        let shading = VoxelShading {
            ao_strength: if toon { 0.25 } else { 0.55 },
            model,
            toon: look.toon,
            ..VoxelShading::default()
        };
        // The look's sun plus a cool fill from the far side.
        let lights = look.lights_with_sun(&[Light::Directional {
            direction: [0.6, 0.3, 0.7],
            color: [0.35, 0.45, 0.7],
            intensity: 0.18,
        }]);
        let lighting = look.lighting(&lights);
        let mut overlays = Vec::new();
        if let Some(Target::Ship(hit)) = world.target {
            let grid = world.fleet.ships[hit.ship].grid.id;
            let min = hit.hit.voxel.as_vec3() - Vec3::splat(0.01);
            overlays.push(OverlayBox {
                grid,
                min,
                max: min + Vec3::splat(1.02),
                color: [0.3, 0.9, 1.0, 1.0],
            });
        }
        let mut frame = HybridFrame::new();
        let clear = [SPACE[0], SPACE[1], SPACE[2], 1.0];
        let prepared = self.voxels.prepare(
            gal,
            scene.target(),
            &camera,
            lighting,
            None,
            &shading,
            &VoxelPassOptions {
                clear_color: Some(clear),
                overlays,
                ..VoxelPassOptions::default()
            },
        )?;
        self.voxel_stats = prepared.stats;
        frame.voxels(Stage::Opaque, prepared)?;
        // The drone: an ordinary lit mesh sharing the voxels' depth.
        let eye = world.camera.position.as_vec3();
        let view = quasar_math::look_to(eye, world.camera.forward(), Vec3::Y)
            .ok_or_else(|| quasar_frame::error("invalid view"))?;
        let view_projection = Mat4::from_glam(projection * view);
        let lit_camera = LitCamera3D {
            view_projection,
            position: eye.to_array(),
        };
        let drone_at = drone_position(world.tick());
        let drone_model = Mat4::from_glam(
            quasar_math::Mat4::from_translation(drone_at)
                * quasar_math::Mat4::from_rotation_y(world.tick() as f32 * 0.02),
        );
        self.surfaces.set_toon(look.toon)?;
        let mut drone_material = LitMaterial::textured(&self.white_view);
        drone_material.shading = model;
        drone_material.base.tint = [0.9, 0.92, 1.0, 1.0];
        drone_material.emissive = [0.05, 0.3, 0.6];
        let drone_draw = SurfaceDraw3D {
            mesh: &self.drone,
            model: drone_model,
            material: SurfaceMaterial::from(drone_material),
        };
        frame.meshes(
            Stage::Opaque,
            self.surfaces.prepare(
                gal,
                scene.target(),
                lit_camera,
                lighting,
                std::slice::from_ref(&drone_draw),
                None,
                frame.mesh_pass_options(clear),
            )?,
        )?;
        // Scorch marks projected onto whatever rock is inside each box.
        let decals: Vec<Decal<'_>> = self
            .scorches
            .iter()
            .map(|p| Decal {
                tint: [1.0, 1.0, 1.0, 0.9],
                fade: 0.5,
                ..Decal::new(
                    Mat4::from_glam(
                        quasar_math::Mat4::from_translation(*p)
                            * quasar_math::Mat4::from_scale(Vec3::splat(8.0)),
                    ),
                    &self.scorch_view,
                )
            })
            .collect();
        frame.push(
            Stage::Decals,
            self.decals
                .prepare(gal, scene, view_projection, &decals, BlendMode::Alpha)?,
        )?;
        frame.push(
            Stage::Effects,
            self.particles.prepare(
                gal,
                scene.target(),
                ParticleCamera::from_view(Mat4::from_glam(view), Mat4::from_glam(projection)),
                1.0 / 60.0,
            )?,
        )?;
        // Ink edges for the cel-shaded look.
        if let Some(edges) = look.edges {
            frame.push(
                Stage::Effects,
                self.screen.prepare_edges(gal, scene, lit_camera, &edges)?,
            )?;
        }
        // Sunlight streaming past the rock into open space.
        if let Some(rays) = look.sun_rays() {
            frame.push(
                Stage::Effects,
                self.screen
                    .prepare_god_rays(gal, scene, lit_camera, &rays)?,
            )?;
        }
        frame.push(
            Stage::Effects,
            self.outlines.prepare(
                gal,
                scene,
                view_projection,
                &[OutlineDraw {
                    mesh: &self.drone,
                    model: drone_model,
                    style: 0,
                }],
                &[OutlineStyle {
                    outline: [0.3, 0.9, 1.0, 1.0],
                    occluded_outline: [0.3, 0.9, 1.0, 0.8],
                    occluded_fill: [0.2, 0.6, 1.0, 0.25],
                    width: 2.0,
                }],
            )?,
        )?;
        let post = look.post_settings();
        frame.push(
            Stage::Post,
            self.post.prepare_scene(gal, scene, target, &post)?,
        )?;
        self.prepare_micros = started.elapsed().as_secs_f64() * 1e6;
        let frame = frame.finish();
        self.pending_submissions = frame.voxel_submissions;
        let mut commands = frame.commands;
        let mut quads_2d = 0;
        if hud {
            let mut canvas = Canvas::new(target.width, target.height)
                .map_err(|e| quasar_frame::error(e.to_string()))?;
            draw_hud(
                &mut canvas,
                world,
                &self.voxel_stats,
                self.prepare_micros,
                target.width as f32,
                target.height as f32,
            );
            let prepared_2d = self.state_2d.prepare(
                gal,
                &canvas,
                &self.textures,
                target,
                DisplayOptions::default(),
                LoadOp::Load,
            )?;
            quads_2d = prepared_2d.stats.quads;
            commands.push(prepared_2d.commands);
        }
        self.stats = RendererStats {
            draws: (self.voxel_stats.indirect_batches + self.voxel_stats.direct_draws + quads_2d)
                as u64,
            ..RendererStats::default()
        };
        Ok(commands)
    }

    pub fn release_targets(&mut self, gal: &Gpu) -> GraphicsResult<()> {
        self.state_2d.release_targets(gal)
    }

    pub fn release(&mut self, gal: &Gpu) -> GraphicsResult<()> {
        self.finish(gal)?;
        self.particles.release(gal)?;
        self.decals.release(gal)?;
        self.outlines.release(gal)?;
        self.surfaces.release(gal)?;
        self.post.release(gal)?;
        self.scene = None;
        self.state_2d.release(gal)
    }
}

/// The scene look: sun, ambient, grade, god rays and (for the cel-shaded
/// style) toon bands with ink edges.
fn look(toon: bool) -> SceneLook {
    let base = SceneLook {
        sun_direction: SUN,
        sun_color: [1.0, 0.96, 0.9],
        sun_intensity: 1.15,
        ambient: [0.07; 3],
        exposure: 1.0,
        bloom: PostSettings::default().bloom,
        god_rays: Some(GodRays {
            source: GodRaySource::Direction(SUN),
            color: [1.0, 0.92, 0.78],
            intensity: 0.35,
            ..GodRays::default()
        }),
        ..SceneLook::default()
    };
    if !toon {
        return SceneLook {
            tonemap: PostSettings::default().tonemap,
            ..base
        };
    }
    SceneLook {
        toon: ToonSettings {
            bands: 3,
            softness: 0.06,
            rim: 0.3,
        },
        ambient: [0.12, 0.12, 0.16],
        grade: ColorGrade {
            saturation: 1.25,
            ..PostSettings::default().grade
        },
        edges: Some(EdgeSettings {
            color: [0.01, 0.01, 0.015, 1.0],
            width: 1.0,
            depth_threshold: 0.05,
            crease_threshold: 0.6,
            fade_distance: 400.0,
        }),
        tonemap: PostSettings::default().tonemap,
        ..base
    }
}

fn draw_hud(
    canvas: &mut Canvas,
    world: &DriftWorld,
    voxels: &VoxelStats,
    prepare_micros: f64,
    w: f32,
    h: f32,
) {
    let scale = (w / 1150.0).min(h / 520.0).clamp(0.7, 2.5);
    let centre = [w * 0.5, h * 0.5];
    let arm = 8.0 * scale;
    let color = Color([200, 240, 255, 220]);
    canvas.rect(
        Rect::new(centre[0] - arm, centre[1] - 1.0, arm * 2.0, 2.0),
        color,
    );
    canvas.rect(
        Rect::new(centre[0] - 1.0, centre[1] - arm, 2.0, arm * 2.0),
        color,
    );
    canvas.rect(
        Rect::new(6.0, 6.0, w - 12.0, 32.0 * scale),
        Color([4, 6, 12, 160]),
    );
    let s = world.stats;
    canvas.text(
        [12.0, 12.0],
        "VOXEL DRIFT  WASD FLY  SPACE/CTRL UP/DOWN  SHIFT BOOST  LMB USE TOOL  RMB REMOVE BLOCK  R TURN  1-8/WHEEL TOOL  ESC QUIT",
        scale,
        Color([226, 238, 250, 255]),
    );
    canvas.text(
        [12.0, 12.0 + 10.0 * scale],
        &format!(
            "SHIPS {}  BLOCKS {}  SPLITS {}  DIGS {}  FILLS {}  NODES {} (L0 {} L1 {} L2 {})  TRIS {}  DRAWS {}  PREPARE {:.0}US",
            s.ships, s.ship_blocks, s.splits, s.digs, s.fills, s.nodes, s.levels[0], s.levels[1], s.levels[2], s.triangles,
            voxels.indirect_batches + voxels.direct_draws, prepare_micros
        ),
        scale,
        Color([150, 175, 205, 255]),
    );
    let look = match world.target {
        Some(Target::Asteroid { distance, .. }) => format!("ASTEROID  {distance:.1} M"),
        Some(Target::Ship(hit)) => {
            let ship = &world.fleet.ships[hit.ship];
            let b = ship.grid.volume.get(world.fleet.ch.block, hit.hit.voxel);
            let name = world
                .content
                .catalog
                .get(b)
                .map_or("?", |d| d.name.as_str())
                .to_uppercase();
            format!("SHIP {}  {name}  {:.1} M", hit.ship, hit.distance)
        }
        None => "NOTHING IN RANGE".into(),
    };
    canvas.text(
        [12.0, 12.0 + 20.0 * scale],
        &format!("LOOK {look}  TURN {}", world.turn),
        scale,
        Color([255, 210, 120, 255]),
    );
    let tools = &world.content.tools;
    let cell = 64.0 * scale;
    let x0 = centre[0] - cell * tools.len() as f32 * 0.5;
    let y0 = h - cell * 0.6 - 12.0 * scale;
    for (i, (_, name)) in tools.iter().enumerate() {
        let x = x0 + i as f32 * cell;
        let frame = if i == world.selected {
            Color([120, 220, 255, 230])
        } else {
            Color([20, 24, 34, 180])
        };
        canvas.rect(Rect::new(x + 2.0, y0, cell - 4.0, cell * 0.6), frame);
        canvas.rect(
            Rect::new(x + 4.0, y0 + 2.0, cell - 8.0, cell * 0.6 - 4.0),
            Color([16, 20, 30, 230]),
        );
        canvas.text(
            [x + 7.0, y0 + 6.0],
            &format!("{} {name}", i + 1),
            scale * 0.8,
            Color([220, 230, 240, 255]),
        );
    }
}
