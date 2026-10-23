//! Explicit one-device composition; renderers own no frame coordinator.
use quasar_render_2d::{
    Canvas, DisplayOptions, LoadOp, Rect, RenderState2D, Sprite, TextureId, Textures,
};
use quasar_render_3d::{
    Camera3D, Draw3D, Mat4, Material, Mesh, PassOptions, RenderState3D, Vertex,
};
use vulkanic_gal::{metrics::SubmitProfile, resources::*, *};

pub struct Demo {
    pub two_d: RenderState2D,
    pub three_d: RenderState3D,
    pub mesh: Mesh,
    pub textures: Textures,
    pub shared_id: TextureId,
    pub image: Handle,
    pub image_view: Handle,
}
impl Demo {
    pub fn new(gal: &mut VulkanicGal) -> GalResult<Self> {
        let mut two_d = RenderState2D::new(gal)?;
        let three_d = RenderState3D::new(gal)?;
        let (vertices, indices) = cube();
        let mesh = Mesh::upload(gal, &vertices, &indices)?;
        let image = gal.create_texture(TextureDesc {
            label: "shared-checker".into(),
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            extent: extent(8, 8),
            mip_levels: 1,
            array_layers: 1,
            usages: vec![TextureUsage::TransferDst, TextureUsage::Sampled],
        })?;
        let upload = gal.create_buffer(BufferDesc {
            label: "checker-upload".into(),
            size: 256,
            memory: MemoryDomain::Upload,
            usages: vec![BufferUsage::TransferSrc, BufferUsage::HostWrite],
        })?;
        let pixels: Vec<u8> = (0..64)
            .flat_map(|i| {
                if (i % 8 / 2 + i / 8 / 2) % 2 == 0 {
                    [225, 170, 75, 255]
                } else {
                    [35, 135, 185, 255]
                }
            })
            .collect();
        gal.submit(SubmissionBatch {
            label: "checker-upload".into(),
            command_lists: vec![list(
                "checker-upload",
                vec![
                    CommandOp::HostWriteBuffer {
                        buffer: upload,
                        offset: 0,
                        data: pixels,
                    },
                    barrier(
                        upload,
                        TextureUsageState::TransferDst,
                        TextureUsageState::TransferSrc,
                    ),
                    barrier(
                        image,
                        TextureUsageState::Undefined,
                        TextureUsageState::TransferDst,
                    ),
                    CommandOp::CopyBufferToTexture(copy(upload, image, extent(8, 8))),
                    barrier(
                        image,
                        TextureUsageState::TransferDst,
                        TextureUsageState::ShaderRead,
                    ),
                ],
            )],
        })?;
        gal.retire(upload)?;
        let image_view = gal.create_texture_view(TextureViewDesc {
            label: "checker-view".into(),
            texture: image,
            format: TextureFormat::Rgba8Unorm,
            base_mip: 0,
            mip_count: 1,
            base_layer: 0,
            layer_count: 1,
        })?;
        let mut textures = Textures::default();
        let shared_id = two_d.register_texture(gal, &mut textures, image_view)?;
        Ok(Self {
            two_d,
            three_d,
            mesh,
            textures,
            shared_id,
            image,
            image_view,
        })
    }
    /// Both frontends return command lists. This example explicitly orders
    /// world -> attachment dependency -> HUD, without readback or a second GAL.
    pub fn commands(
        &mut self,
        gal: &mut VulkanicGal,
        target: Handle,
        angle: f32,
        overlay: bool,
    ) -> GalResult<Vec<CommandList>> {
        let size = gal.pass_target_extent(target)?;
        let projection =
            Mat4::perspective(0.9, size.width as f32 / size.height as f32, 0.1, 100.).unwrap();
        let world = self.three_d.prepare(
            gal,
            target,
            Camera3D {
                view_projection: projection * Mat4::translation(0., 0., -5.),
                light_direction: [0.4, 0.8, 1.],
            },
            &[Draw3D {
                mesh: &self.mesh,
                model: Mat4::rotation_y(angle) * Mat4::rotation_x(-0.35),
                material: Material::textured(self.image_view),
            }],
            PassOptions::default(),
        )?;
        let mut commands = vec![world.commands];
        if overlay {
            let mut canvas = Canvas::new(size.width, size.height)
                .map_err(|e| GalError::invalid_argument(e.to_string()))?;
            canvas.rect(Rect::new(12., 12., 270., 80.), [7, 15, 25, 210]);
            canvas.text([24., 24.], "ONE GAL / 3D + 2D", 2., [235, 245, 250, 255]);
            canvas.text([24., 48.], "SHARED GPU TEXTURE", 1., [135, 215, 220, 255]);
            canvas.sprite(Sprite::new(self.shared_id, Rect::new(232., 48., 32., 32.)));
            canvas.rect(
                Rect::new(
                    size.width as f32 / 2. - 8.,
                    size.height as f32 / 2. - 1.,
                    16.,
                    2.,
                ),
                [255, 255, 255, 128],
            );
            let hud = self.two_d.prepare(
                gal,
                &canvas,
                &self.textures,
                target,
                DisplayOptions::default(),
                LoadOp::Load,
            )?;
            if target.kind() != Some(HandleKind::FrameTarget) {
                commands.push(list(
                    "world-to-hud",
                    vec![barrier(
                        gal.pass_target_color_attachment(target)?,
                        TextureUsageState::ColorAttachment,
                        TextureUsageState::ColorAttachment,
                    )],
                ));
            }
            commands.push(hud.commands);
        }
        Ok(commands)
    }
    pub fn release_targets(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        self.two_d.release_targets(gal)?;
        self.three_d.release_targets(gal)
    }
    pub fn release(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        self.two_d.release(gal)?;
        self.three_d.release(gal)?;
        self.mesh.release(gal)?;
        gal.retire(self.image_view)?;
        gal.retire(self.image)
    }
}
/// Initial explicit orchestration, retained as a comparison path after the
/// small reusable frame transaction is extracted from this implementation.
pub fn submit_explicit(
    gal: &mut VulkanicGal,
    prepare: impl FnOnce(&mut VulkanicGal) -> GalResult<Vec<CommandList>>,
) -> GalResult<(SyncToken, SubmitProfile)> {
    gal.begin_command_recording()?;
    let result = (|| {
        let commands = prepare(gal)?;
        let mut profile = SubmitProfile::default();
        let token = gal.submit_profiled(
            SubmissionBatch {
                label: "explicit-composition".into(),
                command_lists: commands,
            },
            &mut profile,
        )?;
        Ok((token, profile))
    })();
    let finish = gal.finish_command_recording();
    result.and_then(|r| finish.map(|()| r))
}
pub fn cube() -> (Vec<Vertex>, Vec<u32>) {
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
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
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
pub struct Offscreen {
    pub color: Handle,
    pub view: Handle,
    pub depth: Handle,
    pub depth_view: Handle,
    pub target: Handle,
    pub readback: Handle,
    pub size: Extent3d,
    pub initialized: bool,
}
impl Offscreen {
    pub fn new(gal: &mut VulkanicGal, width: u32, height: u32) -> GalResult<Self> {
        let size = extent(width, height);
        let color = gal.create_texture(TextureDesc {
            label: "composition-color".into(),
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            extent: size,
            mip_levels: 1,
            array_layers: 1,
            usages: vec![
                TextureUsage::ColorAttachment,
                TextureUsage::TransferSrc,
                TextureUsage::Sampled,
            ],
        })?;
        let view = gal.create_texture_view(TextureViewDesc {
            label: "composition-color-view".into(),
            texture: color,
            format: TextureFormat::Rgba8Unorm,
            base_mip: 0,
            mip_count: 1,
            base_layer: 0,
            layer_count: 1,
        })?;
        let depth = gal.create_texture(TextureDesc {
            label: "composition-depth".into(),
            dimension: TextureDimension::D2,
            format: TextureFormat::Depth32Float,
            extent: size,
            mip_levels: 1,
            array_layers: 1,
            usages: vec![TextureUsage::DepthStencilAttachment],
        })?;
        let depth_view = gal.create_texture_view(TextureViewDesc {
            label: "composition-depth-view".into(),
            texture: depth,
            format: TextureFormat::Depth32Float,
            base_mip: 0,
            mip_count: 1,
            base_layer: 0,
            layer_count: 1,
        })?;
        let target = gal.create_render_target(RenderTargetDesc {
            label: "composition-target".into(),
            color_views: vec![view],
            depth_stencil_view: Some(depth_view),
            extent: size,
        })?;
        let readback = gal.create_buffer(BufferDesc {
            label: "composition-readback".into(),
            size: u64::from(width) * u64::from(height) * 4,
            memory: MemoryDomain::Readback,
            usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
        })?;
        Ok(Self {
            color,
            view,
            depth,
            depth_view,
            target,
            readback,
            size,
            initialized: false,
        })
    }
    pub fn image(
        &mut self,
        gal: &mut VulkanicGal,
        demo: &mut Demo,
        angle: f32,
        overlay: bool,
    ) -> GalResult<Vec<u8>> {
        let (token, _) = submit_explicit(gal, |gal| {
            let mut commands = vec![list(
                "offscreen-in",
                vec![barrier(
                    self.color,
                    if self.initialized {
                        TextureUsageState::TransferSrc
                    } else {
                        TextureUsageState::Undefined
                    },
                    TextureUsageState::ColorAttachment,
                )],
            )];
            commands.extend(demo.commands(gal, self.target, angle, overlay)?);
            commands.push(list(
                "offscreen-readback",
                vec![
                    barrier(
                        self.color,
                        TextureUsageState::ColorAttachment,
                        TextureUsageState::TransferSrc,
                    ),
                    CommandOp::CopyTextureToBuffer(copy(self.readback, self.color, self.size)),
                    barrier(
                        self.readback,
                        TextureUsageState::TransferDst,
                        TextureUsageState::TransferSrc,
                    ),
                    CommandOp::HostReadBuffer {
                        buffer: self.readback,
                        offset: 0,
                        size: u64::from(self.size.width) * u64::from(self.size.height) * 4,
                    },
                ],
            ));
            Ok(commands)
        })?;
        gal.retire_through(token.submission)?;
        self.initialized = true;
        gal.completed_host_reads()
            .into_iter()
            .find(|r| r.buffer == self.readback && r.submission == token.submission)
            .map(|r| r.bytes)
            .ok_or_else(|| GalError::backend("composition readback missing"))
    }
    pub fn release(self, gal: &mut VulkanicGal) -> GalResult<()> {
        for h in [
            self.target,
            self.view,
            self.color,
            self.depth_view,
            self.depth,
            self.readback,
        ] {
            gal.retire(h)?;
        }
        Ok(())
    }
}
pub fn extent(width: u32, height: u32) -> Extent3d {
    Extent3d {
        width,
        height,
        depth: 1,
    }
}
pub fn barrier(resource: Handle, before: TextureUsageState, after: TextureUsageState) -> CommandOp {
    CommandOp::Barrier(ResourceBarrier {
        resource,
        subresources: None,
        before,
        after,
        src_queue: QueueClass::Graphics,
        dst_queue: QueueClass::Graphics,
    })
}
pub fn list(label: &str, operations: Vec<CommandOp>) -> CommandList {
    CommandList::from(CommandListDesc {
        label: label.into(),
        operations,
    })
}
pub fn copy(buffer: Handle, texture: Handle, size: Extent3d) -> BufferImageCopyRegion {
    BufferImageCopyRegion {
        buffer,
        buffer_offset: 0,
        bytes_per_row: size.width * 4,
        rows_per_image: size.height,
        texture,
        texture_mip: 0,
        texture_layer: 0,
        texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
        extent: size,
    }
}
#[cfg(test)]
mod tests;
