//! One GAL, lit world + lit first-person weapon + direct GAL barriers + 2D HUD.
use crate::props::{ModelId, PropVisual};
use crate::{
    Result, geometry,
    world::{Weapon, World},
};
use quasar_frame::{Gpu, GraphicsResult, RenderTarget, error};
use quasar_profile::RendererStats;
use quasar_render_2d::{Canvas, Color, DisplayOptions, LoadOp, Rect, RenderState2D, Textures};
use quasar_render_3d::*;
use vulkanic_gal::{CommandBuffer, TextureView};

pub struct Graphics {
    world: SurfaceRenderState3D,
    weapon: SurfaceRenderState3D,
    shadows: ShadowRenderState3D,
    shadow_target: Option<ShadowTarget>,
    options: GraphicsOptions,
    normal_concrete_view: TextureView,
    normal_metal_view: TextureView,
    hud: RenderState2D,
    cube: Mesh,
    view: TextureView,
    textures: Textures,
    stats: RendererStats,
    work: RenderWork,
    models: crate::models::Models,
}
#[derive(Clone, Copy, Debug)]
pub struct GraphicsOptions {
    pub shadows: bool,
    pub normal_maps: bool,
    pub shadow_resolution: u32,
}
impl Default for GraphicsOptions {
    fn default() -> Self {
        Self {
            shadows: true,
            normal_maps: true,
            shadow_resolution: 1024,
        }
    }
}
#[derive(Clone, Copy)]
struct Instance {
    model: Mat4,
    color: [f32; 3],
    emissive: [f32; 3],
    unlit: bool,
    specular: f32,
}
impl Instance {
    fn cube(center: [f32; 3], half: [f32; 3], color: [f32; 3]) -> Self {
        Self {
            model: Mat4::translation(center[0], center[1], center[2])
                * Mat4::scale(half[0], half[1], half[2]),
            color,
            emissive: [0.; 3],
            unlit: false,
            specular: 0.15,
        }
    }
    fn glow(center: [f32; 3], half: [f32; 3], color: [f32; 3]) -> Self {
        Self {
            emissive: color,
            unlit: true,
            ..Self::cube(center, half, [0.; 3])
        }
    }
    fn transformed(mut self, parent: Mat4) -> Self {
        self.model = parent * self.model;
        self
    }
}
impl Graphics {
    pub fn new(gal: &Gpu) -> GraphicsResult<Self> {
        Self::with_options(gal, GraphicsOptions::default())
    }
    pub fn with_options(gal: &Gpu, options: GraphicsOptions) -> GraphicsResult<Self> {
        Self::with_submission(
            gal,
            options,
            SubmissionConfig {
                instancing: true,
                frustum_culling: true,
                ..SubmissionConfig::default()
            },
        )
    }
    pub fn with_submission(
        gal: &Gpu,
        options: GraphicsOptions,
        submission: SubmissionConfig,
    ) -> GraphicsResult<Self> {
        Self::with_asset_root(
            gal,
            options,
            submission,
            &crate::models::Models::default_root(),
        )
    }
    pub fn with_asset_root(
        gal: &Gpu,
        options: GraphicsOptions,
        submission: SubmissionConfig,
        root: &std::path::Path,
    ) -> GraphicsResult<Self> {
        let world = SurfaceRenderState3D::with_submission(gal, 16, 2, submission)?;
        let weapon = SurfaceRenderState3D::with_submission(gal, 8, 2, submission)?;
        let hud = RenderState2D::new(gal)?;
        let (vertices, indices) = geometry::cube();
        let cube = Mesh::upload(gal, &vertices, &indices)?;
        let (_texture, view) = geometry::upload_white(gal)?;
        let (_normal_concrete, normal_concrete_view) = geometry::upload_normal(gal, false)?;
        let (_normal_metal, normal_metal_view) = geometry::upload_normal(gal, true)?;
        let shadows = ShadowRenderState3D::with_submission(
            gal,
            ShadowConfig {
                resolution: options.shadow_resolution,
                max_depth_bytes: 96 * 1024 * 1024,
                ..Default::default()
            },
            submission,
        )?;
        let models = crate::models::Models::load(gal, root)?;
        Ok(Self {
            shadow_target: None,
            shadows,
            models,
            options,
            normal_concrete_view,
            normal_metal_view,
            world,
            weapon,
            hud,
            cube,
            view,
            textures: Textures::default(),
            stats: RendererStats::default(),
            work: RenderWork::default(),
        })
    }
    pub fn work_stats(&self) -> RenderWork {
        self.work
    }
    pub fn set_diagnostics(&mut self, d: quasar_diagnostics::Diagnostics) {
        self.world.set_diagnostics(d.clone());
        self.weapon.set_diagnostics(d.clone());
        self.shadows.set_diagnostics(d.clone());
        self.models.set_diagnostics(d);
    }
    pub fn default_asset_root() -> std::path::PathBuf {
        crate::models::Models::default_root()
    }
    pub fn asset_root(&self) -> &std::path::Path {
        &self.models.root
    }
    pub fn reload_assets(&mut self, gal: &Gpu) -> GraphicsResult<()> {
        self.world.clear_bindings(gal)?;
        self.weapon.clear_bindings(gal)?;
        self.shadows.clear_bindings(gal)?;
        self.models.reload(gal)
    }
    pub fn stats(&self) -> RendererStats {
        self.stats
    }
    pub fn set_effects(&mut self, shadows: bool, normal_maps: bool) {
        self.options.shadows = shadows;
        self.options.normal_maps = normal_maps;
    }
    pub fn effects(&self) -> (bool, bool) {
        (self.options.shadows, self.options.normal_maps)
    }
    pub fn resize_shadow_maps(&mut self, gal: &Gpu, resolution: u32) -> GraphicsResult<()> {
        let target = if self.shadow_target.is_some() {
            Some(ShadowTarget::new(
                gal,
                ShadowConfig {
                    resolution,
                    ..self.shadows.config()
                },
            )?)
        } else {
            None
        };
        self.shadows.resize(gal, resolution)?;
        self.world.clear_bindings(gal)?;
        self.weapon.clear_bindings(gal)?;
        self.shadow_target = target;
        self.options.shadow_resolution = resolution;
        Ok(())
    }
    fn draws<'a>(
        cube: &'a Mesh,
        view: &TextureView,
        instances: &[Instance],
        normal_view: Option<&TextureView>,
        receive: bool,
    ) -> Vec<SurfaceDraw3D<'a>> {
        instances
            .iter()
            .map(|i| {
                let mut material = LitMaterial::textured(view);
                material.base.tint = [i.color[0], i.color[1], i.color[2], 1.];
                material.shading = if i.unlit {
                    ShadingModel::Unlit
                } else {
                    ShadingModel::BlinnPhong
                };
                material.specular = [i.specular; 3];
                material.shininess = 48.;
                material.emissive = i.emissive;
                let mut surface = SurfaceMaterial::from(material);
                surface.receives_shadows = receive;
                if !i.unlit {
                    surface.normal_map = normal_view.map(|view| NormalMap {
                        uv_scale: [2.; 2],
                        strength: 0.5,
                        ..NormalMap::new(view)
                    });
                }
                SurfaceDraw3D {
                    mesh: cube,
                    model: i.model,
                    material: surface,
                }
            })
            .collect()
    }
    pub fn commands(
        &mut self,
        gal: &Gpu,
        target: &RenderTarget,
        w: &World,
        paused: bool,
    ) -> GraphicsResult<Vec<CommandBuffer>> {
        let extent = target;
        let aspect = extent.width as f32 / extent.height as f32;
        let projection = Mat4::perspective(80.0f32.to_radians(), aspect, 0.04, 100.).unwrap();
        self.models
            .bind(gal, &mut self.world, &mut self.weapon, &mut self.shadows)?;
        let eye = w.player;
        let f = w.forward();
        let view = Mat4::look_at(
            eye,
            [eye[0] + f[0], eye[1] + f[1], eye[2] + f[2]],
            [0., 1., 0.],
        )
        .unwrap();
        let mut lights = vec![Light::Directional {
            direction: [0.25, -1., -0.3],
            color: [0.38, 0.5, 0.7],
            intensity: 0.65,
        }];
        for prop in w.props() {
            let PropVisual::Cuboid {
                color,
                emissive: true,
                ..
            } = prop.visual
            else {
                continue;
            };
            lights.push(Light::Point {
                position: prop.pose.translation,
                color,
                intensity: 2.2,
                range: 11.,
                attenuation: Attenuation::Smooth,
            });
        }
        let flashlight_index = w.flashlight.then_some(lights.len());
        if flashlight_index.is_some() {
            lights.push(Light::Spot {
                position: eye,
                direction: f,
                color: [0.85, 0.93, 1.],
                intensity: 2.2,
                range: 24.,
                inner_angle: 0.20,
                outer_angle: 0.48,
                attenuation: Attenuation::Smooth,
            });
        }
        if w.muzzle_ticks > 0 {
            lights.push(Light::Point {
                position: [eye[0] + f[0] * 0.7, eye[1] - 0.1, eye[2] + f[2] * 0.7],
                color: [1., 0.5, 0.12],
                intensity: 5.,
                range: 7.,
                attenuation: Attenuation::Smooth,
            });
        }
        let instances = scene(w);
        let mut draws = Self::draws(
            &self.cube,
            &self.view,
            &instances,
            self.options
                .normal_maps
                .then_some(&self.normal_concrete_view),
            true,
        );
        for prop in w.props() {
            let PropVisual::Model { asset, half } = prop.visual else {
                continue;
            };
            let mut imported =
                self.models.gpu[&asset].draws(self.models.fit(asset, prop.pose, half));
            for draw in &mut imported {
                for (c, tint) in [0.5, 0.65, 0.28].into_iter().enumerate() {
                    draw.material.lit.base.tint[c] *= tint;
                }
            }
            draws.extend(imported);
        }
        let casters: Vec<_> = draws
            .iter()
            .filter(|d| d.material.lit.shading != ShadingModel::Unlit)
            .map(SurfaceDraw3D::shadow_caster)
            .collect();
        let mut views = vec![];
        if self.options.shadows {
            let vp = Mat4::orthographic(-23., 23., -23., 23., 0.1, 90.).unwrap()
                * Mat4::look_at([-12.5, 50., 15.], [0., 0., 0.], [0., 0., -1.]).unwrap();
            views.push(ShadowView {
                light_index: 0,
                view_projection: vp,
                settings: ShadowSettings {
                    normal_bias: 0.025,
                    bias: 0.0006,
                    ..ShadowSettings::default()
                },
                casters: &casters,
            });
            if let Some(light_index) = flashlight_index {
                views.push(ShadowView {
                    light_index,
                    view_projection: Mat4::perspective(0.96, 1., 0.08, 24.).unwrap() * view,
                    settings: ShadowSettings {
                        normal_bias: 0.02,
                        bias: 0.0002,
                        ..ShadowSettings::default()
                    },
                    casters: &casters,
                });
            }
        }
        let shadow_frame = if views.is_empty() {
            None
        } else {
            if self.shadow_target.is_none() {
                self.shadow_target = Some(self.shadows.create_target(gal)?);
            }
            Some(
                self.shadows
                    .prepare_into(gal, &views, self.shadow_target.as_ref().unwrap())?,
            )
        };
        let shadow_stats = shadow_frame
            .as_ref()
            .map_or(ShadowStats::default(), |s| s.stats);
        let world = self.world.prepare(
            gal,
            target,
            LitCamera3D {
                view_projection: projection * view,
                position: eye,
            },
            Lighting {
                ambient: [0.035, 0.045, 0.065],
                lights: &lights,
                output_color_space: ColorSpace::Srgb,
            },
            &draws,
            shadow_frame.as_ref().map(PreparedShadows::binding),
            PassOptions {
                clear_color: Some([0.014, 0.022, 0.035, 1.]),
                clear_depth: true,
            },
        )?;
        let weapon_instances = weapon(w);
        let mut draws = Self::draws(
            &self.cube,
            &self.view,
            &weapon_instances,
            self.options.normal_maps.then_some(&self.normal_metal_view),
            false,
        );
        let (root, reload) = weapon_pose(w);
        let gun_id = if w.weapon == Weapon::Rifle {
            ModelId::Rifle
        } else {
            ModelId::Pistol
        };
        let gun_scale = if gun_id == ModelId::Pistol {
            0.65
        } else {
            0.75
        };
        let gun_root = root
            * Mat4::translation(0., 0., -0.14 + w.recoil * 0.035)
            * Mat4::scale(gun_scale, gun_scale, gun_scale);
        for (i, instance) in self.models.gpu[&gun_id].instances().iter().enumerate() {
            let parent = if instance.name == "magazine" {
                gun_root * Mat4::translation(0., -reload * 0.28, reload * 0.08)
            } else {
                gun_root
            };
            let mut draw = self.models.gpu[&gun_id].draw(i, parent).unwrap();
            draw.material.receives_shadows = false;
            for (c, tint) in [0.5, 0.65, 0.25].into_iter().enumerate() {
                draw.material.lit.base.tint[c] *= tint;
            }
            draws.push(draw);
        }
        let mut gun_lights = vec![Light::Directional {
            direction: [-0.5, -0.6, -1.],
            color: [0.65, 0.8, 1.],
            intensity: 0.75,
        }];
        if w.muzzle_ticks > 0 {
            gun_lights.push(Light::Point {
                position: [0.15, -0.12, -0.75],
                color: [1., 0.4, 0.08],
                intensity: 4.,
                range: 2.,
                attenuation: Attenuation::Smooth,
            });
        }
        let gun = self.weapon.prepare(
            gal,
            target,
            LitCamera3D {
                view_projection: projection,
                position: [0.; 3],
            },
            Lighting {
                ambient: [0.08, 0.09, 0.12],
                lights: &gun_lights,
                output_color_space: ColorSpace::Srgb,
            },
            &draws,
            None,
            LitPassOptions {
                pass: PassOptions {
                    clear_color: None,
                    clear_depth: true,
                },
            },
        )?;
        let mut canvas =
            Canvas::new(extent.width, extent.height).map_err(|e| error(e.to_string()))?;
        hud(&mut canvas, w, paused, self.effects());
        let hud = self.hud.prepare(
            gal,
            &canvas,
            &self.textures,
            target,
            DisplayOptions::default(),
            LoadOp::Load,
        )?;
        self.work = self.world.work_stats();
        self.work += self.weapon.work_stats();
        if shadow_frame.is_some() {
            self.work += self.shadows.work_stats();
        }
        self.stats = RendererStats {
            draws: (world.stats.draws + gun.stats.draws + hud.stats.batches + shadow_stats.draws)
                as u64,
            geometry_upload_bytes: hud.stats.geometry_upload_bytes as u64,
            texture_upload_bytes: hud.stats.texture_upload_bytes as u64,
            uniform_upload_bytes: (world.stats.uniform_upload_bytes
                + gun.stats.uniform_upload_bytes
                + shadow_stats.uniform_upload_bytes) as u64,
        };
        let mut commands: Vec<_> = shadow_frame.into_iter().map(|s| s.commands).collect();
        commands.push(world.commands);
        commands.push(gun.commands);
        commands.push(hud.commands);
        Ok(commands)
    }
    pub fn release_targets(&mut self, gal: &Gpu) -> GraphicsResult<()> {
        self.world.release_targets(gal)?;
        self.weapon.release_targets(gal)?;
        self.hud.release_targets(gal)
    }
    pub fn release(&mut self, gal: &Gpu) -> GraphicsResult<()> {
        self.world.release(gal)?;
        self.weapon.release(gal)?;
        self.shadows.release(gal)?;
        self.hud.release(gal)?;
        self.models.release(gal)?;
        self.cube.release(gal)?;
        Ok(())
    }
}
fn scene(w: &World) -> Vec<Instance> {
    let mut out = Vec::with_capacity(400);
    let mut add = |c, h, color| out.push(Instance::cube(c, h, color));
    // Industrial courtyard: alternating floor slabs and recessed service lanes.
    for x in -4..4 {
        for z in -4..4 {
            let value = if (x + z) % 2 == 0 { 0.12 } else { 0.095 };
            add(
                [x as f32 * 4. + 2., -0.118, z as f32 * 4. + 2.],
                [1.98, 0.12, 1.98],
                [value, value * 1.1, value * 1.2],
            );
        }
    }
    for side in [-1., 1.] {
        for z in -6..7 {
            add(
                [side * 2.4, 0.012, z as f32 * 2.],
                [0.05, 0.012, 0.55],
                [0.55, 0.32, 0.055],
            );
        }
    }
    for prop in w.props() {
        if let PropVisual::Cuboid {
            half,
            color,
            emissive,
        } = prop.visual
        {
            let mut instance = if emissive {
                Instance::glow([0.; 3], half, color)
            } else {
                Instance::cube([0.; 3], half, color)
            };
            instance = instance.transformed(Mat4(prop.pose.matrix().expect("validated prop pose")));
            out.push(instance);
        }
    }
    for (index, z) in w.zombies.iter().enumerate() {
        let dead = if z.alive() {
            0.
        } else {
            (z.dead_ticks as f32 / 35.).min(1.)
        };
        let root = Mat4::translation(z.position[0], z.feet_y + 0.08, z.position[1])
            * Mat4::rotation_y(z.yaw)
            * Mat4::rotation_x(dead * 1.5);
        let skin = if z.hit_ticks > 0 {
            [0.8, 0.23, 0.12]
        } else {
            [0.27, 0.40, 0.23]
        };
        let cloth = if index % 2 == 0 {
            [0.16, 0.22, 0.18]
        } else {
            [0.24, 0.17, 0.14]
        };
        out.push(Instance::cube([0., 1.03, 0.], [0.32, 0.38, 0.20], cloth).transformed(root));
        out.push(Instance::cube([0., 1.58, 0.], [0.23, 0.25, 0.22], skin).transformed(root));
        out.push(
            Instance::cube([0., 1.8, -0.02], [0.24, 0.06, 0.23], [0.12, 0.15, 0.10])
                .transformed(root),
        );
        for side in [-1., 1.] {
            let leg = root
                * Mat4::translation(side * 0.17, 0.65, 0.)
                * Mat4::rotation_x(z.gait.sin() * side * 0.5 * (1. - dead));
            out.push(
                Instance::cube([0., -0.31, 0.], [0.13, 0.31, 0.15], [0.10, 0.13, 0.14])
                    .transformed(leg),
            );
            out.push(
                Instance::cube([0., -0.58, 0.07], [0.14, 0.10, 0.22], [0.055, 0.065, 0.07])
                    .transformed(leg),
            );
            let arm = root
                * Mat4::translation(side * 0.43, 1.28, 0.)
                * Mat4::rotation_x(-0.9 + z.gait.cos() * side * 0.18);
            out.push(Instance::cube([0., -0.28, 0.], [0.11, 0.29, 0.12], cloth).transformed(arm));
            out.push(Instance::cube([0., -0.59, 0.], [0.10, 0.12, 0.10], skin).transformed(arm));
            out.push(
                Instance::glow(
                    [side * 0.095, 1.62, 0.224],
                    [0.036, 0.025, 0.015],
                    [0.9, 0.32, 0.045],
                )
                .transformed(root),
            );
        }
        out.push(
            Instance::cube(
                [0., 1.48, 0.225],
                [0.12, 0.025, 0.015],
                [0.07, 0.055, 0.025],
            )
            .transformed(root),
        );
    }
    out
}
fn weapon_pose(w: &World) -> (Mat4, f32) {
    let reload = if w.reload_ticks > 0 {
        (w.reload_ticks as f32
            / if w.weapon == Weapon::Pistol {
                75.
            } else {
                110.
            }
            * std::f32::consts::PI)
            .sin()
    } else {
        0.
    };
    let bob = (w.distance_walked * 7.).sin() * 0.008;
    let root = Mat4::translation(0.23 + bob, -0.20 - reload * 0.35, -0.46 + w.recoil * 0.08)
        * Mat4::rotation_x(w.recoil * 0.17 + reload * 0.65);
    (root, reload)
}
fn weapon(w: &World) -> Vec<Instance> {
    let (root, _) = weapon_pose(w);
    let mut out = vec![];
    let mut add = |c, h, color| out.push(Instance::cube(c, h, color).transformed(root));
    // Hands, grip, reciprocating slide/receiver and a separate barrel/sight assembly.
    add([0.02, -0.16, 0.17], [0.065, 0.08, 0.10], [0.17, 0.14, 0.11]);
    add(
        [-0.065, -0.09, 0.02],
        [0.07, 0.06, 0.10],
        [0.14, 0.17, 0.16],
    );
    let rifle = w.weapon == Weapon::Rifle;
    if w.muzzle_ticks > 0 {
        let z = if rifle { -0.65 } else { -0.40 };
        let pulse = w.muzzle_ticks as f32 / 4.;
        out.push(
            Instance::glow(
                [0., 0.025, z],
                [0.035 + 0.035 * pulse, 0.035 + 0.055 * pulse, 0.06],
                [1., 0.65, 0.13],
            )
            .transformed(root),
        );
        out.push(
            Instance::glow(
                [0., 0.025, z - 0.07],
                [0.018, 0.018, 0.04],
                [1., 0.92, 0.55],
            )
            .transformed(root),
        );
    }
    out
}
fn hud(c: &mut Canvas, w: &World, paused: bool, effects: (bool, bool)) {
    let sw = c.width() as f32;
    let sh = c.height() as f32;
    let scale = (sw / 960.).clamp(0.7, 1.8);
    let mut text = |pos: [f32; 2], s: &str, color: [u8; 4]| c.text(pos, s, scale, Color(color));
    text([26., 24.], "N I G H T W A T C H", [235, 226, 201, 255]);
    text(
        [26., 24. + 14. * scale],
        "SECTOR 09  //  QUASAR LIGHTING LAB",
        [117, 150, 160, 255],
    );
    text(
        [26., 24. + 28. * scale],
        &format!(
            "H SHADOWS {}   N NORMAL MAPS {}",
            if effects.0 { "ON" } else { "OFF" },
            if effects.1 { "ON" } else { "OFF" }
        ),
        [117, 150, 160, 255],
    );
    text(
        [sw - 200. * scale, 26.],
        &format!("WAVE {:02}   KILLS {:03}", w.wave, w.kills),
        [235, 200, 120, 255],
    );
    text(
        [26., sh - 54. * scale],
        w.weapon.name(),
        [205, 214, 210, 255],
    );
    text(
        [26., sh - 36. * scale],
        &format!(
            "HEALTH {:03}   {}   HOSTILES {:02}",
            w.health as u32,
            if w.flashlight {
                "LIGHT ON"
            } else {
                "LIGHT OFF"
            },
            w.living()
        ),
        [125, 173, 161, 255],
    );
    text(
        [sw - 190. * scale, sh - 52. * scale],
        &format!("{:02} / {:02}", w.ammo(), w.weapon.capacity()),
        [245, 224, 175, 255],
    );
    text(
        [sw - 240. * scale, sh - 32. * scale],
        if w.reload_ticks > 0 {
            "RELOADING..."
        } else {
            "R RELOAD  /  1 2 WEAPONS"
        },
        [148, 168, 173, 255],
    );
    text(
        [26., sh - 16. * scale],
        "WASD MOVE  SHIFT SPRINT  SPACE JUMP  LMB FIRE  F FLASHLIGHT  ESC PAUSE",
        [105, 125, 134, 255],
    );
    let color = Color(if w.hit_marker > 0 {
        [255, 119, 72, 255]
    } else {
        [190, 211, 206, 210]
    });
    let mid = [sw * 0.5, sh * 0.5];
    let gap = 5. + w.recoil * 4.;
    for side in [-1., 1.] {
        c.rect(
            Rect::new(
                mid[0] + if side > 0. { gap } else { -gap - 6. },
                mid[1] - 1.,
                6.,
                2.,
            ),
            color,
        );
        c.rect(
            Rect::new(
                mid[0] - 1.,
                mid[1] + if side > 0. { gap } else { -gap - 6. },
                2.,
                6.,
            ),
            color,
        );
    }
    if w.hurt_ticks > 0 {
        let alpha = (w.hurt_ticks * 4).min(90) as u8;
        for (x, y, width, height) in [
            (0., 0., sw, 14.),
            (0., sh - 14., sw, 14.),
            (0., 0., 14., sh),
            (sw - 14., 0., 14., sh),
        ] {
            c.rect(Rect::new(x, y, width, height), Color([175, 24, 12, alpha]));
        }
    }
    if paused || w.game_over() {
        c.rect(Rect::new(0., 0., sw, sh), Color([4, 8, 14, 190]));
        let title = if w.game_over() {
            "YOU DID NOT SURVIVE"
        } else {
            "PAUSED"
        };
        c.text(
            [sw * 0.5 - title.len() as f32 * 5. * scale, sh * 0.45],
            title,
            scale * 2.,
            Color([240, 222, 182, 255]),
        );
        c.text(
            [sw * 0.5 - 150. * scale, sh * 0.54],
            if w.game_over() {
                "ENTER TO RESTART   /   ESC TO QUIT"
            } else {
                "ESC OR CLICK TO RESUME   /   Q TO QUIT"
            },
            scale,
            Color([150, 176, 183, 255]),
        );
    }
}
pub fn screenshot(w: &World, width: u32, height: u32, path: &std::path::Path) -> Result<()> {
    screenshot_with_options(w, width, height, path, GraphicsOptions::default())
}
pub fn screenshot_with_options(
    w: &World,
    width: u32,
    height: u32,
    path: &std::path::Path,
    options: GraphicsOptions,
) -> Result<()> {
    screenshot_with_submission(
        w,
        width,
        height,
        path,
        options,
        SubmissionConfig {
            instancing: true,
            frustum_culling: true,
            ..SubmissionConfig::default()
        },
    )
}
pub fn screenshot_with_submission(
    w: &World,
    width: u32,
    height: u32,
    path: &std::path::Path,
    options: GraphicsOptions,
    submission: SubmissionConfig,
) -> Result<()> {
    screenshot_with_asset_root(w, width, height, path, options, submission, None)
}
pub fn screenshot_with_asset_root(
    w: &World,
    width: u32,
    height: u32,
    path: &std::path::Path,
    options: GraphicsOptions,
    submission: SubmissionConfig,
    root: Option<&std::path::Path>,
) -> Result<()> {
    let gal = Gpu::headless()?;
    let mut graphics = Graphics::with_asset_root(
        &gal,
        options,
        submission,
        root.unwrap_or(&Graphics::default_asset_root()),
    )?;
    let target = quasar_frame::OffscreenTarget::new(&gal, width, height)?;
    let image = target.render(&gal, |gpu, target| graphics.commands(gpu, target, w, false))?;
    quasar_render_2d::write_png(path, width, height, &image.rgba)?;
    Ok(())
}
