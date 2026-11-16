//! Voxel renderer (asteroid LOD nodes and ship grids) plus a 2D HUD.
use crate::GeometryUpdate;
use crate::content::{TILE, mat, texture_layers};
use crate::world::{DriftWorld, Target};
use quasar_frame::{Gpu, GraphicsResult, RenderTarget};
use quasar_math::Vec3;
use quasar_profile::RendererStats;
use quasar_render_2d::{Canvas, Color, DisplayOptions, LoadOp, Rect, RenderState2D, Textures};
use quasar_render_3d::{ColorSpace, Light, Lighting, ShadingModel};
use quasar_render_voxel::{
    ChunkKey, ChunkPlacement, MaterialDef, MaterialFilter, OverlayBox, VoxelCamera, VoxelMaterials,
    VoxelPassOptions, VoxelRenderer, VoxelRendererConfig, VoxelShading, VoxelStats,
};
use vulkanic_gal::CommandBuffer;

const SPACE: [f32; 3] = [0.004, 0.005, 0.012];

pub struct Graphics {
    voxels: VoxelRenderer,
    state_2d: RenderState2D,
    textures: Textures,
    stats: RendererStats,
    pub voxel_stats: VoxelStats,
    pending_submission: Option<u64>,
    pub prepare_micros: f64,
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

fn encode_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

impl Graphics {
    pub fn new(gal: &Gpu) -> GraphicsResult<Self> {
        let voxels = VoxelRenderer::new(gal, materials(gal)?, VoxelRendererConfig::default())?;
        Ok(Self {
            voxels,
            state_2d: RenderState2D::new(gal)?,
            textures: Textures::default(),
            stats: RendererStats::default(),
            voxel_stats: VoxelStats::default(),
            pending_submission: None,
            prepare_micros: 0.0,
        })
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
        if let Some(submission) = self.pending_submission.take() {
            self.voxels.submitted(gal, submission);
        }
    }

    pub fn finish(&mut self, gal: &Gpu) -> GraphicsResult<()> {
        self.pending_submission = None;
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
        let srgb_output = !target.format.is_srgb();
        let clear = if srgb_output {
            SPACE.map(encode_srgb)
        } else {
            SPACE
        };
        let shading = VoxelShading {
            ao_strength: 0.55,
            model: ShadingModel::Diffuse,
            ..VoxelShading::default()
        };
        let lights = [
            Light::Directional {
                direction: [-0.55, -0.35, -0.75],
                color: [1.0, 0.96, 0.9],
                intensity: 1.15,
            },
            Light::Directional {
                direction: [0.6, 0.3, 0.7],
                color: [0.35, 0.45, 0.7],
                intensity: 0.18,
            },
        ];
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
        let prepared = self.voxels.prepare(
            gal,
            target,
            &camera,
            Lighting {
                ambient: [0.07; 3],
                lights: &lights,
                output_color_space: if srgb_output {
                    ColorSpace::Srgb
                } else {
                    ColorSpace::Linear
                },
            },
            None,
            &shading,
            &VoxelPassOptions {
                clear_color: Some([clear[0], clear[1], clear[2], 1.0]),
                overlays,
                ..VoxelPassOptions::default()
            },
        )?;
        self.prepare_micros = started.elapsed().as_secs_f64() * 1e6;
        self.voxel_stats = prepared.stats;
        self.pending_submission = Some(prepared.submission);
        let mut commands = vec![prepared.commands];
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
        self.state_2d.release(gal)
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
