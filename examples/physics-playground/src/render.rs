//! Shared-GAL 3D scene plus optional 2D HUD.
use crate::world::{Playground, PropKind, StaticBox};
use quasar_profile::RendererStats;
use quasar_render_2d::{Canvas, Color, DisplayOptions, LoadOp, Rect, RenderState2D, Textures};
use quasar_render_3d::{
    Camera3D, Draw3D, Mat4, Material, Mesh, PassOptions, RenderState3D, Vertex,
};
use quasar_spatial::{Aabb, Frustum, Plane};
use vulkanic_gal::{
    CommandList, CommandListDesc, CommandOp, GalError, GalResult, Handle, HandleKind,
    ResourceBarrier, SubmissionBatch, TextureFormat, TextureOrigin3d, TextureUsage,
    TextureUsageState, TextureViewDesc, VulkanicGal, commands::BufferImageCopyRegion, resources::*,
};

pub struct Graphics {
    state_3d: RenderState3D,
    state_2d: RenderState2D,
    cube: Mesh,
    sphere: Mesh,
    texture: Handle,
    texture_view: Handle,
    textures: Textures,
    stats: RendererStats,
}

impl Graphics {
    pub fn new(gal: &mut VulkanicGal) -> GalResult<Self> {
        let state_3d = RenderState3D::new(gal)?;
        let state_2d = RenderState2D::new(gal)?;
        let (cube_vertices, cube_indices) = cube();
        let cube = Mesh::upload(gal, &cube_vertices, &cube_indices)?;
        let (sphere_vertices, sphere_indices) = sphere(14, 20);
        let sphere = match Mesh::upload(gal, &sphere_vertices, &sphere_indices) {
            Ok(mesh) => mesh,
            Err(error) => {
                let mut cube = cube;
                let _ = cube.release(gal);
                let mut state_3d = state_3d;
                let _ = state_3d.release(gal);
                let mut state_2d = state_2d;
                let _ = state_2d.release(gal);
                return Err(error);
            }
        };
        let (texture, texture_view) = match upload_white(gal) {
            Ok(value) => value,
            Err(error) => {
                let mut cube = cube;
                let _ = cube.release(gal);
                let mut sphere = sphere;
                let _ = sphere.release(gal);
                let mut state_3d = state_3d;
                let _ = state_3d.release(gal);
                let mut state_2d = state_2d;
                let _ = state_2d.release(gal);
                return Err(error);
            }
        };
        Ok(Self {
            state_3d,
            state_2d,
            cube,
            sphere,
            texture,
            texture_view,
            textures: Textures::default(),
            stats: RendererStats::default(),
        })
    }

    pub fn last_stats(&self) -> RendererStats {
        self.stats
    }

    pub fn commands(
        &mut self,
        gal: &mut VulkanicGal,
        target: Handle,
        world: &Playground,
    ) -> GalResult<Vec<CommandList>> {
        let size = gal.pass_target_extent(target)?;
        let aspect = size.width as f32 / size.height.max(1) as f32;
        let eye = world.camera_eye();
        let forward = world.forward();
        let view = Mat4::look_at(
            eye,
            [
                eye[0] + forward[0],
                eye[1] + forward[1],
                eye[2] + forward[2],
            ],
            [0.0, 1.0, 0.0],
        )
        .ok_or_else(|| GalError::invalid_argument("invalid playground camera"))?;
        let projection = Mat4::perspective(90.0_f32.to_radians(), aspect, 0.05, 90.0)
            .ok_or_else(|| GalError::invalid_argument("invalid playground projection"))?;
        let view_projection = projection * view;
        let frustum = frustum(view_projection);
        let region = Aabb::new([-20.0, -2.0, -20.0], [20.0, 7.0, 20.0])
            .map_err(|error| GalError::invalid_argument(error.to_string()))?;
        let visible = world.props_in_frustum(&frustum, region);
        let mut draws = Vec::with_capacity(24 + visible.len());
        for StaticBox { bounds, tint } in world.static_boxes() {
            let center = bounds.center();
            let extents = bounds.extents();
            draws.push(Draw3D {
                mesh: &self.cube,
                model: Mat4::translation(center[0] as f32, center[1] as f32, center[2] as f32)
                    * Mat4::scale(
                        extents[0] as f32 * 0.5,
                        extents[1] as f32 * 0.5,
                        extents[2] as f32 * 0.5,
                    ),
                material: Material {
                    texture_view: self.texture_view,
                    tint: *tint,
                    transparent: false,
                    depth_write: true,
                },
            });
        }
        for index in visible {
            let prop = &world.props()[index];
            let (position, rotation) = world
                .prop_transform(index)
                .ok_or_else(|| GalError::invalid_argument("missing prop transform"))?;
            let scale = prop.kind.half_extents();
            draws.push(Draw3D {
                mesh: if matches!(prop.kind, PropKind::Ball { .. }) {
                    &self.sphere
                } else {
                    &self.cube
                },
                model: Mat4::translation(position[0], position[1], position[2])
                    * quaternion(rotation[0], rotation[1], rotation[2], rotation[3])
                    * Mat4::scale(scale[0], scale[1], scale[2]),
                material: Material {
                    texture_view: self.texture_view,
                    tint: prop.kind.tint(),
                    transparent: false,
                    depth_write: true,
                },
            });
        }
        for index in 0..world.agents().len() {
            let agent = &world.agents()[index];
            let pulse = world.agent_pulse(index);
            let state = world.agent_state(index).unwrap_or("idle");
            let tint = match state {
                "alert" => [1.0, 0.28, 0.22, 1.0],
                "move" => [0.35, 1.0, 0.55, 1.0],
                _ => [0.25, 0.75, 1.0, 1.0],
            };
            let scale = crate::world::AGENT_RADIUS * (1.7 * pulse);
            draws.push(Draw3D {
                mesh: &self.sphere,
                model: Mat4::translation(
                    agent.position[0],
                    crate::world::AGENT_RADIUS,
                    agent.position[1],
                ) * Mat4::scale(scale, scale, scale),
                material: Material {
                    texture_view: self.texture_view,
                    tint,
                    transparent: false,
                    depth_write: true,
                },
            });
        }
        if world.debug_nav() {
            for index in 0..world.agents().len() {
                if let Some(path) = world.agent_path(index) {
                    for cell in path.iter().step_by(2) {
                        let center = [
                            cell.coords[0] as f32 - crate::world::NAV_EXTENT + 0.5,
                            cell.coords[1] as f32 - crate::world::NAV_EXTENT + 0.5,
                        ];
                        draws.push(Draw3D {
                            mesh: &self.cube,
                            model: Mat4::translation(center[0], 0.05, center[1])
                                * Mat4::scale(0.18, 0.04, 0.18),
                            material: Material {
                                texture_view: self.texture_view,
                                tint: [1.0, 0.9, 0.25, 1.0],
                                transparent: false,
                                depth_write: true,
                            },
                        });
                    }
                }
            }
        }
        let sensor_bounds = world.sensor_bounds();
        let sensor = sensor_bounds.center();
        let sensor_extents = sensor_bounds.extents();
        draws.push(Draw3D {
            mesh: &self.cube,
            model: Mat4::translation(sensor[0] as f32, sensor[1] as f32, sensor[2] as f32)
                * Mat4::scale(
                    sensor_extents[0] as f32 * 0.5,
                    sensor_extents[1] as f32 * 0.5,
                    sensor_extents[2] as f32 * 0.5,
                ),
            material: Material {
                texture_view: self.texture_view,
                tint: if world.zone_active() {
                    [1.0, 0.82, 0.2, 0.35]
                } else {
                    [0.3, 0.7, 1.0, 0.16]
                },
                transparent: true,
                depth_write: false,
            },
        });
        let prepared_3d = self.state_3d.prepare(
            gal,
            target,
            Camera3D {
                view_projection,
                light_direction: [0.35, 0.82, 0.25],
            },
            &draws,
            PassOptions {
                clear_color: Some([0.035, 0.05, 0.075, 1.0]),
                clear_depth: true,
            },
        )?;
        let stats_3d = prepared_3d.stats;

        let mut canvas = Canvas::new(size.width, size.height)
            .map_err(|error| GalError::invalid_argument(error.to_string()))?;
        hud(&mut canvas, world);
        let prepared_2d = self.state_2d.prepare(
            gal,
            &canvas,
            &self.textures,
            target,
            DisplayOptions::default(),
            LoadOp::Load,
        )?;
        let stats_2d = prepared_2d.stats;
        self.stats = RendererStats {
            draws: stats_3d.draws as u64 + stats_2d.quads as u64,
            geometry_upload_bytes: stats_2d.geometry_upload_bytes as u64,
            texture_upload_bytes: stats_2d.texture_upload_bytes as u64,
            uniform_upload_bytes: stats_3d.uniform_upload_bytes as u64,
        };
        let mut commands = vec![prepared_3d.commands];
        if target.kind() != Some(HandleKind::FrameTarget) {
            let attachment = gal.pass_target_color_attachment(target)?;
            commands.push(CommandList::from(CommandListDesc {
                label: "playground-to-hud".into(),
                operations: vec![CommandOp::Barrier(ResourceBarrier {
                    resource: attachment,
                    subresources: None,
                    before: TextureUsageState::ColorAttachment,
                    after: TextureUsageState::ColorAttachment,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Graphics,
                })],
            }));
        }
        commands.push(prepared_2d.commands);
        Ok(commands)
    }

    pub fn release_targets(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        self.state_2d.release_targets(gal)?;
        self.state_3d.release_targets(gal)
    }

    pub fn release(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        self.state_2d.release(gal)?;
        self.state_3d.release(gal)?;
        self.cube.release(gal)?;
        self.sphere.release(gal)?;
        gal.retire(self.texture_view)?;
        gal.retire(self.texture)?;
        self.texture = Handle::NULL;
        Ok(())
    }
}

fn hud(canvas: &mut Canvas, world: &Playground) {
    let stats = world.stats();
    let scale = (canvas.width() as f32 / 1150.0)
        .min(canvas.height() as f32 / 520.0)
        .clamp(0.7, 2.5);
    let center = [canvas.width() as f32 * 0.5, canvas.height() as f32 * 0.5];
    let arm = 8.0 * scale;
    let color = Color([220, 238, 255, 220]);
    canvas.rect(Rect::new(center[0] - arm, center[1] - 1.0, arm, 2.0), color);
    canvas.rect(Rect::new(center[0], center[1] - 1.0, arm, 2.0), color);
    canvas.rect(Rect::new(center[0] - 1.0, center[1] - arm, 2.0, arm), color);
    canvas.rect(Rect::new(center[0] - 1.0, center[1], 2.0, arm), color);

    canvas.text(
        [12.0, 12.0],
        "PHYSICS PLAYGROUND  WASD MOVE  SPACE JUMP  LMB KICK  F BALL  N NAV DEBUG  R RESET  F11 FULLSCREEN",
        scale,
        Color([226, 238, 250, 255]),
    );
    canvas.text(
        [12.0, 12.0 + 10.0 * scale],
        &format!(
            "POS {:.1},{:.1},{:.1}  VY {:.1}  GROUND {}  PROPS {} ({} BALLS)  AWAKE {}  NEAR {}  ZONE {}",
            stats.player[0],
            stats.player[1],
            stats.player[2],
            stats.velocity_y,
            stats.grounded,
            stats.props,
            stats.balls,
            stats.awake,
            stats.nearby_props,
            stats.zone_active
        ),
        scale,
        Color([148, 170, 194, 255]),
    );
    canvas.text(
        [12.0, 12.0 + 30.0 * scale],
        &format!(
            "AGENTS {}  PATH NODES {}  NAV GEN {}  DEBUG {}  STATES {}",
            stats.agents,
            stats.agent_path_nodes,
            stats.nav_generation,
            if stats.debug_nav { "ON" } else { "OFF" },
            (0..stats.agents)
                .filter_map(|index| world.agent_state(index))
                .map(|state| state.to_ascii_uppercase())
                .collect::<Vec<_>>()
                .join(",")
        ),
        scale,
        Color([188, 156, 240, 255]),
    );
    let target = stats
        .look_hit
        .map(|hit| {
            if let Some(index) = hit.prop {
                format!(
                    "LOOK: {} #{index}  DIST {:.1}  PRESS LMB TO KICK",
                    if matches!(world.props()[index].kind, PropKind::Ball { .. }) {
                        "BALL"
                    } else {
                        "BLOCK"
                    },
                    hit.distance
                )
            } else {
                format!("LOOK: WORLD  DIST {:.1}", hit.distance)
            }
        })
        .unwrap_or_else(|| "LOOK: NOTHING IN RANGE".into());
    canvas.text(
        [12.0, 12.0 + 20.0 * scale],
        &target,
        scale,
        Color([255, 206, 112, 255]),
    );
}

fn frustum(view_projection: Mat4) -> Frustum<3> {
    let mut rows = [[0.0f64; 4]; 4];
    for (row, values) in rows.iter_mut().enumerate() {
        for (column, value) in values.iter_mut().enumerate() {
            *value = f64::from(view_projection.0[column][row]);
        }
    }
    let mut planes = Vec::with_capacity(6);
    for (left, right) in [
        (rows[0], rows[3]),
        (
            [-rows[0][0], -rows[0][1], -rows[0][2], -rows[0][3]],
            rows[3],
        ),
        (rows[1], rows[3]),
        (
            [-rows[1][0], -rows[1][1], -rows[1][2], -rows[1][3]],
            rows[3],
        ),
    ] {
        if let Ok(plane) = Plane::new(
            [left[0] + right[0], left[1] + right[1], left[2] + right[2]],
            left[3] + right[3],
        ) {
            planes.push(plane);
        }
    }
    if let Ok(plane) = Plane::new([rows[2][0], rows[2][1], rows[2][2]], rows[2][3]) {
        planes.push(plane);
    }
    let far = [
        rows[3][0] - rows[2][0],
        rows[3][1] - rows[2][1],
        rows[3][2] - rows[2][2],
        rows[3][3] - rows[2][3],
    ];
    if let Ok(plane) = Plane::new([far[0], far[1], far[2]], far[3]) {
        planes.push(plane);
    }
    Frustum::new(planes).unwrap_or_else(|_| {
        Frustum::new(vec![
            Plane::new([1.0, 0.0, 0.0], 1000.0).unwrap(),
            Plane::new([0.0, 1.0, 0.0], 1000.0).unwrap(),
            Plane::new([0.0, 0.0, 1.0], 1000.0).unwrap(),
        ])
        .expect("fallback frustum")
    })
}

fn quaternion(x: f32, y: f32, z: f32, w: f32) -> Mat4 {
    let xx = x * x;
    let yy = y * y;
    let zz = z * z;
    let xy = x * y;
    let xz = x * z;
    let yz = y * z;
    let wx = w * x;
    let wy = w * y;
    let wz = w * z;
    Mat4([
        [1.0 - 2.0 * (yy + zz), 2.0 * (xy + wz), 2.0 * (xz - wy), 0.0],
        [2.0 * (xy - wz), 1.0 - 2.0 * (xx + zz), 2.0 * (yz + wx), 0.0],
        [2.0 * (xz + wy), 2.0 * (yz - wx), 1.0 - 2.0 * (xx + yy), 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ])
}

fn upload_white(gal: &mut VulkanicGal) -> GalResult<(Handle, Handle)> {
    let size = Extent3d {
        width: 1,
        height: 1,
        depth: 1,
    };
    let texture = gal.create_texture(TextureDesc {
        label: "playground-white".into(),
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        extent: size,
        mip_levels: 1,
        array_layers: 1,
        usages: vec![TextureUsage::Sampled, TextureUsage::TransferDst],
    })?;
    let view = match gal.create_texture_view(TextureViewDesc {
        label: "playground-white-view".into(),
        texture,
        format: TextureFormat::Rgba8Unorm,
        base_mip: 0,
        mip_count: 1,
        base_layer: 0,
        layer_count: 1,
    }) {
        Ok(view) => view,
        Err(error) => {
            let _ = gal.retire(texture);
            return Err(error);
        }
    };
    let upload = match gal.create_buffer(BufferDesc {
        label: "playground-white-upload".into(),
        size: 4,
        memory: MemoryDomain::Upload,
        usages: vec![BufferUsage::TransferSrc, BufferUsage::HostWrite],
    }) {
        Ok(upload) => upload,
        Err(error) => {
            let _ = gal.retire(view);
            let _ = gal.retire(texture);
            return Err(error);
        }
    };
    let result = gal.submit(SubmissionBatch {
        label: "playground-white-upload".into(),
        command_lists: vec![CommandList::from(CommandListDesc {
            label: "playground-white-upload".into(),
            operations: vec![
                CommandOp::HostWriteBuffer {
                    buffer: upload,
                    offset: 0,
                    data: vec![255; 4],
                },
                CommandOp::Barrier(ResourceBarrier {
                    resource: upload,
                    subresources: None,
                    before: TextureUsageState::TransferDst,
                    after: TextureUsageState::TransferSrc,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Graphics,
                }),
                CommandOp::Barrier(ResourceBarrier {
                    resource: texture,
                    subresources: None,
                    before: TextureUsageState::Undefined,
                    after: TextureUsageState::TransferDst,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Graphics,
                }),
                CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
                    buffer: upload,
                    buffer_offset: 0,
                    bytes_per_row: 4,
                    rows_per_image: 1,
                    texture,
                    texture_mip: 0,
                    texture_layer: 0,
                    texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                    extent: size,
                }),
                CommandOp::Barrier(ResourceBarrier {
                    resource: texture,
                    subresources: None,
                    before: TextureUsageState::TransferDst,
                    after: TextureUsageState::ShaderRead,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Graphics,
                }),
            ],
        })],
    });
    let _ = gal.retire(upload);
    if let Err(error) = result {
        let _ = gal.retire(view);
        let _ = gal.retire(texture);
        return Err(error);
    }
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
    let uvs = [[0., 1.], [1., 1.], [1., 0.], [0., 0.]];
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (normal, corners) in faces {
        let base = vertices.len() as u32;
        for (corner, uv) in corners.into_iter().zip(uvs) {
            vertices.push(Vertex {
                position: corner,
                normal,
                uv,
            });
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (vertices, indices)
}

fn sphere(rings: u32, sectors: u32) -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    for ring in 0..=rings {
        let latitude = std::f32::consts::PI * ring as f32 / rings as f32;
        let (sin_lat, cos_lat) = latitude.sin_cos();
        for sector in 0..=sectors {
            let longitude = std::f32::consts::TAU * sector as f32 / sectors as f32;
            let (sin_lon, cos_lon) = longitude.sin_cos();
            let normal = [sin_lat * cos_lon, cos_lat, sin_lat * sin_lon];
            vertices.push(Vertex {
                position: normal,
                normal,
                uv: [sector as f32 / sectors as f32, ring as f32 / rings as f32],
            });
        }
    }
    let mut indices = Vec::new();
    let row = sectors + 1;
    for ring in 0..rings {
        for sector in 0..sectors {
            let a = ring * row + sector;
            let b = a + row;
            indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    (vertices, indices)
}
