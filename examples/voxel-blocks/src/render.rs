//! Voxel renderer plus a 2D HUD on one GAL frame.
use crate::content::{Content, TILE, tex, texture_layers};
use crate::world::{BlockWorld, EDGE};
use quasar_frame::{Gpu, GraphicsResult, RenderTarget};
use quasar_math::{Mat4, Vec3};
use quasar_profile::RendererStats;
use quasar_render_2d::{Canvas, Color, DisplayOptions, LoadOp, Rect, RenderState2D, Textures};
use quasar_render_3d::{ColorSpace, Light, Lighting, ShadingModel};
use quasar_render_voxel::{
    ChunkKey, ChunkPlacement, Fog, MaterialDef, MaterialFilter, OverlayBox, VoxelCamera,
    VoxelMaterials, VoxelPassOptions, VoxelRenderer, VoxelRendererConfig, VoxelShading, VoxelStats,
};
use quasar_voxel::{GridId, GridTransform};
use vulkanic_gal::CommandBuffer;

const GRID: GridId = GridId(0);
/// Linear sky colour.
const SKY: [f32; 3] = [0.45, 0.66, 0.92];

pub struct Graphics {
    voxels: VoxelRenderer,
    state_2d: RenderState2D,
    textures: Textures,
    stats: RendererStats,
    pub voxel_stats: VoxelStats,
    pending_submission: Option<u64>,
    /// CPU time spent preparing the voxel pass last frame.
    pub prepare_micros: f64,
    /// Skip chunks hidden behind terrain (connectivity culling).
    pub occlusion: bool,
}

fn materials(gal: &Gpu) -> GraphicsResult<VoxelMaterials> {
    let defs: Vec<MaterialDef> = (0..tex::COUNT)
        .map(|layer| match layer {
            tex::LEAVES => MaterialDef::layer(layer).with_alpha_test(0.5),
            tex::LAMP => MaterialDef::layer(layer).with_emissive([0.55, 0.42, 0.2]),
            _ => MaterialDef::layer(layer),
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
        let mut voxels = VoxelRenderer::new(gal, materials(gal)?, VoxelRendererConfig::default())?;
        voxels.set_grid(GRID, GridTransform::default(), 1.0);
        Ok(Self {
            voxels,
            state_2d: RenderState2D::new(gal)?,
            textures: Textures::default(),
            stats: RendererStats::default(),
            voxel_stats: VoxelStats::default(),
            pending_submission: None,
            prepare_micros: 0.0,
            occlusion: true,
        })
    }

    pub fn last_stats(&self) -> RendererStats {
        self.stats
    }

    /// Uploads finished meshes and removes unloaded ones.
    pub fn sync(&mut self, gal: &Gpu, world: &mut BlockWorld) -> GraphicsResult<()> {
        self.voxels.retire_completed(gal);
        let updates = world.take_mesh_updates();
        for c in updates.remove {
            self.voxels.remove(ChunkKey::new(GRID, c));
        }
        for (c, mesh) in &updates.upload {
            self.voxels.upload(
                gal,
                ChunkKey::new(GRID, *c),
                ChunkPlacement::new(*c * EDGE),
                mesh,
            )?;
        }
        Ok(())
    }

    /// Call after the frame's commands were submitted.
    pub fn submitted(&mut self, gal: &Gpu) {
        if let Some(submission) = self.pending_submission.take() {
            self.voxels.submitted(gal, submission);
        }
    }

    /// Waits for the GPU and frees retired geometry (headless tools).
    pub fn finish(&mut self, gal: &Gpu) -> GraphicsResult<()> {
        self.pending_submission = None;
        self.voxels.wait_and_retire(gal)
    }

    pub fn commands(
        &mut self,
        gal: &Gpu,
        target: &RenderTarget,
        world: &mut BlockWorld,
        hud: bool,
    ) -> GraphicsResult<Vec<CommandBuffer>> {
        self.sync(gal, world)?;
        let started = std::time::Instant::now();
        let aspect = target.width as f32 / target.height.max(1) as f32;
        let far = (world.view_radius as f32 + 1.0) * EDGE as f32;
        let projection = quasar_math::perspective(75f32.to_radians(), aspect, 0.05, far + 64.0)
            .ok_or_else(|| quasar_frame::error("invalid projection"))?;
        let eye = world.player.eye();
        let forward = world.player.forward();
        let camera = VoxelCamera::looking(eye.as_dvec3(), forward, Vec3::Y, projection)
            .ok_or_else(|| quasar_frame::error("invalid camera"))?;
        let world_vp = camera.relative_view_projection() * Mat4::from_translation(-eye);
        let visible = if self.occlusion {
            world.visible_chunks(world_vp, far)
        } else {
            None
        };
        self.voxels.set_visible_chunks(GRID, visible.as_deref());
        let srgb_output = !target.format.is_srgb();
        let fog_color = SKY;
        let clear = if srgb_output {
            SKY.map(encode_srgb)
        } else {
            SKY
        };
        let shading = VoxelShading {
            ao_strength: 0.5,
            voxel_light: true,
            sky_brightness: 1.0,
            min_light: 0.05,
            model: ShadingModel::Diffuse,
            fog: Some(Fog {
                color: fog_color,
                start: far * 0.55,
                end: far * 0.95,
            }),
            ..VoxelShading::default()
        };
        let lights = [Light::Directional {
            direction: [-0.35, -0.85, -0.4],
            color: [1.0, 0.97, 0.9],
            intensity: 0.45,
        }];
        let mut overlays = Vec::new();
        if let Some(hit) = world.target {
            let min = hit.voxel.as_vec3() - Vec3::splat(0.002);
            overlays.push(OverlayBox {
                grid: GRID,
                min,
                max: min + Vec3::splat(1.004),
                color: [0.02, 0.02, 0.02, 1.0],
            });
        }
        let prepared = self.voxels.prepare(
            gal,
            target,
            &camera,
            Lighting {
                ambient: [0.62; 3],
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
            draw_hud(&mut canvas, world, &self.voxel_stats, self.prepare_micros);
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
            geometry_upload_bytes: 0,
            texture_upload_bytes: 0,
            uniform_upload_bytes: 0,
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

fn draw_hud(canvas: &mut Canvas, world: &BlockWorld, voxels: &VoxelStats, prepare_micros: f64) {
    let scale = (canvas.width() as f32 / 1150.0)
        .min(canvas.height() as f32 / 520.0)
        .clamp(0.7, 2.5);
    let (w, h) = (canvas.width() as f32, canvas.height() as f32);
    let centre = [w * 0.5, h * 0.5];
    let arm = 8.0 * scale;
    let color = Color([240, 240, 240, 220]);
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
        Color([8, 10, 14, 150]),
    );
    let content: &Content = &world.content;
    let s = world.stats;
    let p = world.player.position;
    canvas.text(
        [12.0, 12.0],
        "VOXEL BLOCKS  WASD MOVE  SPACE JUMP  F FLY  LMB BREAK  RMB PLACE  1-9/WHEEL SELECT  F11 FULLSCREEN  ESC QUIT",
        scale,
        Color([236, 242, 250, 255]),
    );
    canvas.text(
        [12.0, 12.0 + 10.0 * scale],
        &format!(
            "POS {:.1},{:.1},{:.1}  COLUMNS {}  CHUNKS {}  MESHED {}  PENDING {}  QUADS {}  VISIBLE {}  DRAWS {}  PREPARE {:.0}US",
            p.x, p.y, p.z, s.columns, s.chunks, s.meshed, s.pending_meshes, s.quads, s.visible,
            voxels.indirect_batches + voxels.direct_draws, prepare_micros
        ),
        scale,
        Color([170, 190, 210, 255]),
    );
    let look = world
        .target
        .map(|hit| {
            let b = world.volume.get(world.ch.block, hit.voxel);
            let light = world.volume.get(world.ch.block_light, hit.adjacent());
            let sky = world.volume.get(world.ch.sky_light, hit.adjacent());
            format!(
                "LOOK {} AT {},{},{}  LIGHT {light} SKY {sky}",
                content.name(b).to_uppercase(),
                hit.voxel.x,
                hit.voxel.y,
                hit.voxel.z
            )
        })
        .unwrap_or_else(|| "LOOK NOTHING".into());
    canvas.text(
        [12.0, 12.0 + 20.0 * scale],
        &look,
        scale,
        Color([255, 214, 120, 255]),
    );
    // Hotbar.
    let slots = content.hotbar.len();
    let cell = 46.0 * scale;
    let x0 = centre[0] - cell * slots as f32 * 0.5;
    let y0 = h - cell - 12.0 * scale;
    for (i, id) in content.hotbar.iter().enumerate() {
        let x = x0 + i as f32 * cell;
        let selected = i == world.selected;
        let frame = if selected {
            Color([255, 255, 255, 230])
        } else {
            Color([20, 20, 24, 170])
        };
        canvas.rect(Rect::new(x + 2.0, y0, cell - 4.0, cell - 4.0), frame);
        canvas.rect(
            Rect::new(x + 5.0, y0 + 3.0, cell - 10.0, cell - 10.0),
            Color([36, 38, 44, 220]),
        );
        let name = content.name(*id).to_uppercase();
        let label: String = name.chars().take(6).collect();
        canvas.text(
            [x + 6.0, y0 + cell * 0.4],
            &label,
            scale * 0.8,
            Color([230, 230, 230, 255]),
        );
        canvas.text(
            [x + 6.0, y0 + 6.0],
            &format!("{}", i + 1),
            scale * 0.8,
            Color([150, 150, 150, 255]),
        );
    }
}
