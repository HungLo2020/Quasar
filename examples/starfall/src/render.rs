//! Shared-GAL 3D world plus 2D HUD rendering.

use crate::content::TextureData;
use crate::world::{GameWorld, Vec3};
use quasar_frame::{Gpu, GraphicsResult, RenderTarget};
use quasar_profile::RendererStats;
use quasar_render_2d::RenderStats as Stats2D;
use quasar_render_2d::{Canvas, DisplayOptions, LoadOp, Rect, RenderState2D, Textures};
use quasar_render_3d::RenderStats as Stats3D;
use quasar_render_3d::{
    Camera3D, Draw3D, Mat4, Material, Mesh, PassOptions, RenderState3D, Vertex,
};
use quasar_ui::{Ui, UiRenderer};
use vulkanic_gal::{CommandBuffer, Texture, TextureView};

pub struct Graphics {
    pub two_d: RenderState2D,
    pub three_d: RenderState3D,
    mesh: Mesh,
    texture_view: TextureView,
    textures: Textures,
    ui: UiRenderer,
    stats: RendererStats,
}
impl Graphics {
    pub fn new(gal: &Gpu, texture: &TextureData) -> GraphicsResult<Self> {
        let two_d = RenderState2D::new(gal)?;
        let three_d = RenderState3D::new(gal)?;
        let (vertices, indices) = cube();
        let mesh = Mesh::upload(gal, &vertices, &indices)?;
        let (_texture_handle, texture_view) = match upload_texture(gal, texture) {
            Ok(value) => value,
            Err(error) => {
                let mut mesh = mesh;
                let _ = mesh.release(gal);
                let mut two_d = two_d;
                let _ = two_d.release(gal);
                let mut three_d = three_d;
                let _ = three_d.release(gal);
                return Err(error);
            }
        };
        Ok(Self {
            two_d,
            three_d,
            mesh,
            texture_view,
            textures: Textures::default(),
            ui: UiRenderer::new(),
            stats: RendererStats::default(),
        })
    }
    pub fn last_stats(&self) -> RendererStats {
        self.stats
    }
    #[cfg(feature = "desktop")]
    pub fn enable_text(&mut self, renderer: quasar_ui::TextRenderer) {
        self.ui.enable_text(renderer);
    }
    pub fn commands(
        &mut self,
        gal: &Gpu,
        target: &RenderTarget,
        world: &GameWorld,
        menu: Option<&Ui>,
    ) -> GraphicsResult<Vec<CommandBuffer>> {
        let size = target;
        let aspect = size.width as f32 / size.height.max(1) as f32;
        let eye = world.camera_eye();
        let focus = world.player + Vec3::new(0., 0.6, 0.);
        let view = Mat4::look_at(
            [eye.x, eye.y, eye.z],
            [focus.x, focus.y, focus.z],
            [0., 1., 0.],
        )
        .ok_or_else(|| quasar_frame::error("invalid game camera"))?;
        let projection = Mat4::perspective(0.9, aspect, 0.1, 120.)
            .ok_or_else(|| quasar_frame::error("invalid game projection"))?;
        let mut draws = Vec::with_capacity(32 + world.crystals.len() + world.drones.len());
        draws.push(Draw3D {
            mesh: &self.mesh,
            model: Mat4::translation(0., -0.55, 0.) * Mat4::scale(25., 0.6, 25.),
            material: Material {
                texture_view: self.texture_view.clone(),
                tint: [0.16, 0.20, 0.30, 1.],
                transparent: false,
                depth_write: true,
            },
        });
        for line in -6..=6 {
            draws.push(Draw3D {
                mesh: &self.mesh,
                model: Mat4::translation(line as f32 * 2.0, 0.01, 0.)
                    * Mat4::scale(0.018, 0.012, 23.),
                material: Material {
                    texture_view: self.texture_view.clone(),
                    tint: [0.22, 0.55, 0.72, 1.],
                    transparent: false,
                    depth_write: true,
                },
            });
            draws.push(Draw3D {
                mesh: &self.mesh,
                model: Mat4::translation(0., 0.01, line as f32 * 2.0)
                    * Mat4::scale(23., 0.012, 0.018),
                material: Material {
                    texture_view: self.texture_view.clone(),
                    tint: [0.22, 0.55, 0.72, 1.],
                    transparent: false,
                    depth_write: true,
                },
            });
        }
        for (index, crystal) in world.crystals.iter().filter(|c| c.active).enumerate() {
            let bob = (world.tick as f32 * 0.06 + index as f32).sin() * 0.12;
            draws.push(Draw3D {
                mesh: &self.mesh,
                model: Mat4::translation(
                    crystal.position.x,
                    crystal.position.y + 0.7 + bob,
                    crystal.position.z,
                ) * Mat4::rotation_y(world.tick as f32 * 0.035 + index as f32)
                    * Mat4::scale(0.38, 0.75, 0.38),
                material: Material {
                    texture_view: self.texture_view.clone(),
                    tint: [0.25, 1.0, 0.7, 1.],
                    transparent: false,
                    depth_write: true,
                },
            });
        }
        for (index, drone) in world.drones.iter().filter(|d| d.active).enumerate() {
            draws.push(Draw3D {
                mesh: &self.mesh,
                model: Mat4::translation(drone.position.x, drone.position.y, drone.position.z)
                    * Mat4::rotation_y(world.tick as f32 * 0.05 + index as f32)
                    * Mat4::scale(0.55, 0.55, 0.55),
                material: Material {
                    texture_view: self.texture_view.clone(),
                    tint: [1.0, 0.22, 0.28, 1.],
                    transparent: false,
                    depth_write: true,
                },
            });
        }
        if world.portal_ready {
            for index in 0..4 {
                let angle = index as f32 * std::f32::consts::FRAC_PI_2 + world.tick as f32 * 0.02;
                let position = world.portal + Vec3::new(angle.cos() * 1.0, 0.9, angle.sin() * 1.0);
                draws.push(Draw3D {
                    mesh: &self.mesh,
                    model: Mat4::translation(position.x, position.y, position.z)
                        * Mat4::rotation_y(-angle)
                        * Mat4::scale(0.18, 1.6, 0.18),
                    material: Material {
                        texture_view: self.texture_view.clone(),
                        tint: [0.3, 0.9, 1.0, 1.],
                        transparent: false,
                        depth_write: true,
                    },
                });
            }
        }
        draws.push(Draw3D {
            mesh: &self.mesh,
            model: Mat4::translation(world.player.x, 0.65, world.player.z)
                * Mat4::rotation_y(world.yaw)
                * Mat4::scale(0.72, 1.15, 0.72),
            material: Material {
                texture_view: self.texture_view.clone(),
                tint: [0.25, 0.6, 1.0, 1.],
                transparent: false,
                depth_write: true,
            },
        });
        draws.push(Draw3D {
            mesh: &self.mesh,
            model: Mat4::translation(world.player.x, 0.8, world.player.z)
                * Mat4::rotation_y(world.yaw)
                * Mat4::translation(0., 0., 0.55)
                * Mat4::scale(0.28, 0.28, 0.55),
            material: Material {
                texture_view: self.texture_view.clone(),
                tint: [0.55, 0.9, 1.0, 1.],
                transparent: false,
                depth_write: true,
            },
        });
        for shot in &world.shots {
            draws.push(Draw3D {
                mesh: &self.mesh,
                model: Mat4::translation(shot.position.x, shot.position.y, shot.position.z)
                    * Mat4::rotation_y(shot.yaw())
                    * Mat4::scale(0.13, 0.13, 0.45),
                material: Material {
                    texture_view: self.texture_view.clone(),
                    tint: [0.5, 1.0, 1.0, 1.],
                    transparent: false,
                    depth_write: true,
                },
            });
        }
        let world_prepared = self.three_d.prepare(
            gal,
            target,
            Camera3D {
                view_projection: projection * view,
                light_direction: [0.35, 0.82, 0.25],
            },
            &draws,
            PassOptions {
                clear_color: Some([0.012, 0.018, 0.04, 1.]),
                clear_depth: true,
            },
        )?;
        let stats3d: Stats3D = world_prepared.stats;
        #[cfg(feature = "desktop")]
        if let Some(menu) = menu {
            self.ui
                .prepare_text(&mut self.two_d, gal, &mut self.textures, menu)?;
        }
        let mut canvas = Canvas::new(size.width, size.height)
            .map_err(|error| quasar_frame::error(error.to_string()))?;
        hud(&mut canvas, world);
        if let Some(menu) = menu {
            self.ui.draw(&mut canvas, menu);
        }
        let hud_prepared = self.two_d.prepare(
            gal,
            &canvas,
            &self.textures,
            target,
            DisplayOptions::default(),
            LoadOp::Load,
        )?;
        let stats2d: Stats2D = hud_prepared.stats;
        self.stats = RendererStats {
            draws: stats3d.draws as u64 + stats2d.quads as u64,
            geometry_upload_bytes: stats2d.geometry_upload_bytes as u64,
            texture_upload_bytes: stats2d.texture_upload_bytes as u64,
            uniform_upload_bytes: stats3d.uniform_upload_bytes as u64,
        };
        let mut commands = vec![world_prepared.commands];
        commands.push(hud_prepared.commands);
        Ok(commands)
    }
    pub fn release_targets(&mut self, gal: &Gpu) -> GraphicsResult<()> {
        self.two_d.release_targets(gal)?;
        self.three_d.release_targets(gal)
    }
    pub fn release(&mut self, gal: &Gpu) -> GraphicsResult<()> {
        self.two_d.release(gal)?;
        self.three_d.release(gal)?;
        self.mesh.release(gal)?;
        Ok(())
    }
}

fn hud(canvas: &mut Canvas, world: &GameWorld) {
    let width = canvas.width() as f32;
    let height = canvas.height() as f32;
    canvas.rect(Rect::new(16., 16., width - 32., 74.), [3, 8, 18, 220]);
    canvas.text([28., 28.], "STARFALL", 2., [120, 220, 255, 255]);
    canvas.text(
        [28., 54.],
        &format!(
            "SCORE {}  CRYSTALS {}  DRONES {}",
            world.score,
            world.crystals_remaining(),
            world.drones_remaining()
        ),
        1.,
        [220, 240, 250, 255],
    );
    canvas.text(
        [width - 230., 28.],
        &format!("TIME {:02.1}", world.time_left),
        1.2,
        [255, 220, 140, 255],
    );
    canvas.text(
        [width - 230., 52.],
        &format!("HP {}", world.health.max(0)),
        1.2,
        [120, 255, 170, 255],
    );
    canvas.rect(
        Rect::new(width / 2. - 5., height / 2. - 5., 10., 10.),
        [180, 240, 255, 180],
    );
    canvas.text(
        [28., height - 42.],
        "WASD MOVE  MOUSE AIM  LMB/SPACE SHOOT  Q/E TURN  F11 FULLSCREEN  R RESTART  ESC QUIT",
        1.2,
        [140, 170, 190, 255],
    );
    if world.portal_ready && !world.finished() {
        canvas.text(
            [width / 2. - 120., height - 84.],
            "PORTAL OPEN - RETURN TO THE RING",
            1.,
            [120, 255, 230, 255],
        );
    }
    if world.won {
        canvas.text(
            [width / 2. - 46., height / 2. - 24.],
            "STARFALL COMPLETE",
            2.,
            [180, 255, 200, 255],
        );
        canvas.text(
            [width / 2. - 78., height / 2. + 4.],
            "PRESS R TO FLY AGAIN",
            1.,
            [220, 240, 250, 255],
        );
    } else if world.lost {
        canvas.text(
            [width / 2. - 34., height / 2. - 24.],
            "SIGNAL LOST",
            2.,
            [255, 140, 140, 255],
        );
        canvas.text(
            [width / 2. - 78., height / 2. + 4.],
            "PRESS R TO RESTART",
            1.,
            [240, 220, 220, 255],
        );
    }
}

fn upload_texture(gal: &Gpu, data: &TextureData) -> GraphicsResult<(Texture, TextureView)> {
    let expected = data.width as usize * data.height as usize * 4;
    if data.rgba.len() != expected || data.width == 0 || data.height == 0 {
        return Err(quasar_frame::error("invalid RGBA texture dimensions"));
    }
    let texture = quasar_frame::texture_2d(
        &gal.device,
        "Starfall image",
        data.width,
        data.height,
        vulkanic_gal::TextureFormat::Rgba8Unorm,
        vulkanic_gal::TextureUsages::TEXTURE_BINDING | vulkanic_gal::TextureUsages::COPY_DST,
    )?;
    gal.queue.write_texture(
        texture.as_image_copy(),
        &data.rgba,
        vulkanic_gal::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(data.width * 4),
            rows_per_image: Some(data.height),
        },
        texture.size(),
    );
    let view = texture.create_view(&Default::default());
    Ok((texture, view))
}

fn cube() -> (Vec<Vertex>, Vec<u32>) {
    let faces = [
        (
            [0., 0., 1.],
            [[-1., -1., 1.], [1., -1., 1.], [1., 1., 1.], [-1., 1., 1.]],
        ),
        (
            [0., 0., -1.],
            [
                [1., -1., -1.],
                [-1., -1., -1.],
                [-1., 1., -1.],
                [1., 1., -1.],
            ],
        ),
        (
            [1., 0., 0.],
            [[1., -1., 1.], [1., -1., -1.], [1., 1., -1.], [1., 1., 1.]],
        ),
        (
            [-1., 0., 0.],
            [
                [-1., -1., -1.],
                [-1., -1., 1.],
                [-1., 1., 1.],
                [-1., 1., -1.],
            ],
        ),
        (
            [0., 1., 0.],
            [[-1., 1., 1.], [1., 1., 1.], [1., 1., -1.], [-1., 1., -1.]],
        ),
        (
            [0., -1., 0.],
            [
                [-1., -1., -1.],
                [1., -1., -1.],
                [1., -1., 1.],
                [-1., -1., 1.],
            ],
        ),
    ];
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (normal, positions) in faces {
        let base = vertices.len() as u32;
        for (position, uv) in positions
            .into_iter()
            .zip([[0., 1.], [1., 1.], [1., 0.], [0., 0.]])
        {
            vertices.push(Vertex {
                position,
                normal,
                uv,
            });
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (vertices, indices)
}
