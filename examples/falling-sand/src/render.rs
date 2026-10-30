//! 2D rendering for the sand demo through `quasar-render-2d`.
use crate::sand::{BALL_RADIUS, Cell, GRID_HEIGHT, GRID_WIDTH, ObstacleKind, SandWorld, Tool};
use quasar_frame::{Gpu, GraphicsResult, RenderTarget};
use quasar_profile::RendererStats;
use quasar_render_2d::{Canvas, Color, DisplayOptions, LoadOp, Rect, RenderState2D, Textures};
use quasar_ui::{Ui, UiRenderer};
use vulkanic_gal::CommandBuffer;

const BACKGROUND: Color = Color([10, 14, 24, 255]);
const SAND: Color = Color([226, 176, 82, 255]);
const SAND_DARK: Color = Color([188, 132, 52, 255]);
const ROCK: Color = Color([104, 108, 124, 255]);
const BOUND: Color = Color([64, 78, 104, 255]);
const BALL: Color = Color([92, 208, 255, 255]);
const HUD: Color = Color([224, 236, 248, 255]);
const HUD_DIM: Color = Color([142, 162, 184, 255]);

pub struct Graphics {
    state: RenderState2D,
    textures: Textures,
    ui: UiRenderer,
    stats: RendererStats,
}

impl Graphics {
    pub fn new(gal: &Gpu) -> GraphicsResult<Self> {
        Ok(Self {
            state: RenderState2D::new(gal)?,
            textures: Textures::default(),
            ui: UiRenderer::new(),
            stats: RendererStats::default(),
        })
    }

    pub fn last_stats(&self) -> RendererStats {
        self.stats
    }

    pub fn commands(
        &mut self,
        gal: &Gpu,
        target: &RenderTarget,
        world: &SandWorld,
        cursor: Option<[f32; 2]>,
        tool: Tool,
        menu: Option<&Ui>,
    ) -> GraphicsResult<Vec<CommandBuffer>> {
        let size = target;
        let width = size.width as f32;
        let height = size.height as f32;
        let cell_width = width / GRID_WIDTH as f32;
        let cell_height = height / GRID_HEIGHT as f32;
        if !cell_width.is_finite() || !cell_height.is_finite() || cell_width <= 0.0 {
            return Err(quasar_frame::error("invalid sand target extent"));
        }
        let mut canvas = Canvas::new(size.width, size.height)
            .map_err(|error| quasar_frame::error(error.to_string()))?;
        canvas.clear = BACKGROUND;

        for (_, obstacle) in world.obstacles() {
            let bounds = obstacle.bounds;
            canvas.rect(
                Rect::new(
                    (bounds.min[0] as f32 * cell_width).floor(),
                    (bounds.min[1] as f32 * cell_height).floor(),
                    ((bounds.max[0] - bounds.min[0]) as f32 * cell_width).ceil(),
                    ((bounds.max[1] - bounds.min[1]) as f32 * cell_height).ceil(),
                ),
                if obstacle.kind == ObstacleKind::Bound {
                    BOUND
                } else {
                    ROCK
                },
            );
        }

        // Merge horizontal sand runs to keep the quad count far below one per
        // cell and leave headroom for the HUD.
        for y in 0..GRID_HEIGHT {
            let mut x = 0;
            while x < GRID_WIDTH {
                if world.cell(x, y) != Some(Cell::Sand) {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < GRID_WIDTH && world.cell(x, y) == Some(Cell::Sand) {
                    x += 1;
                }
                let color = if (x + y) % 7 == 0 { SAND_DARK } else { SAND };
                canvas.rect(
                    Rect::new(
                        start as f32 * cell_width,
                        y as f32 * cell_height,
                        (x - start) as f32 * cell_width + 0.5,
                        cell_height + 0.5,
                    ),
                    color,
                );
            }
        }

        let ball = world.ball_position();
        let radius = BALL_RADIUS;
        let center = [ball[0] * cell_width, ball[1] * cell_height];
        let rx = radius * cell_width;
        let ry = radius * cell_height;
        canvas.rect(
            Rect::new(center[0] - rx, center[1] - ry * 0.55, rx * 2.0, ry * 1.1),
            BALL,
        );
        canvas.rect(
            Rect::new(center[0] - rx * 0.55, center[1] - ry, rx * 1.1, ry * 2.0),
            BALL,
        );
        canvas.rect(
            Rect::new(
                center[0] - rx * 0.75,
                center[1] - ry * 0.75,
                rx * 1.5,
                ry * 1.5,
            ),
            BALL,
        );

        if let Some(cursor) = cursor {
            let brush = match tool {
                Tool::Sand => 4.0,
                Tool::Rock => 7.0,
                Tool::Erase => 6.0,
            };
            let color = match tool {
                Tool::Sand => SAND,
                Tool::Rock => ROCK,
                Tool::Erase => Color([240, 96, 96, 255]),
            };
            let x = cursor[0] * cell_width;
            let y = cursor[1] * cell_height;
            let r = brush * cell_width;
            let thickness = (cell_width * 0.8).max(1.0);
            canvas.rect(Rect::new(x - r, y - r, r * 2.0, thickness), color);
            canvas.rect(
                Rect::new(x - r, y + r - thickness, r * 2.0, thickness),
                color,
            );
            canvas.rect(Rect::new(x - r, y - r, thickness, r * 2.0), color);
            canvas.rect(
                Rect::new(x + r - thickness, y - r, thickness, r * 2.0),
                color,
            );
        }

        let stats = world.stats();
        let text_scale = (width / 1180.0).min(height / 420.0).clamp(0.7, 2.5);
        canvas.text(
            [12.0, 12.0],
            "FALLING SAND  LMB PAINT  RMB ERASE  1 SAND  2 ROCK  3 ERASE  R RESET  F11 FULLSCREEN",
            text_scale,
            HUD,
        );
        canvas.text(
            [12.0, 12.0 + 10.0 * text_scale],
            &format!(
                "SAND {}  ROCKS {}  BALL {:.0},{:.0}  NEAR {}  STEPS {}  EVENTS {}",
                stats.sand,
                stats.obstacles,
                stats.ball[0],
                stats.ball[1],
                stats.nearby_obstacles,
                stats.physics_steps,
                stats.collision_events
            ),
            text_scale,
            HUD_DIM,
        );

        if let Some(menu) = menu {
            self.ui.draw(&mut canvas, menu);
        }
        let prepared = self.state.prepare(
            gal,
            &canvas,
            &self.textures,
            target,
            DisplayOptions::default(),
            LoadOp::Clear,
        )?;
        self.stats = RendererStats {
            draws: prepared.stats.quads as u64,
            geometry_upload_bytes: prepared.stats.geometry_upload_bytes as u64,
            texture_upload_bytes: prepared.stats.texture_upload_bytes as u64,
            uniform_upload_bytes: 0,
        };
        Ok(vec![prepared.commands])
    }

    pub fn release_targets(&mut self, gal: &Gpu) -> GraphicsResult<()> {
        self.state.release_targets(gal)
    }

    pub fn release(&mut self, gal: &Gpu) -> GraphicsResult<()> {
        self.state.release(gal)
    }
}
