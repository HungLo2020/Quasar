//! One shared native device for raw wgpu commands, 3D and 2D drawing.
use quasar_frame::{FrameReport, Gpu, GraphicsResult, OffscreenTarget, RenderTarget, texture_2d};
use quasar_render_2d::{
    Canvas, DisplayOptions, LoadOp, Rect, RenderState2D, Sprite, TextureId, Textures,
};
use quasar_render_3d::{
    Camera3D, Draw3D, Mat4, Material, Mesh, PassOptions, RenderState3D, Vertex,
};
use vulkanic_gal::{
    CommandBuffer, SubmissionIndex, Texture, TextureFormat, TextureUsages, TextureView,
};
pub struct Demo {
    pub two_d: RenderState2D,
    pub three_d: RenderState3D,
    pub mesh: Mesh,
    pub textures: Textures,
    pub shared_id: TextureId,
    pub image: Texture,
    pub image_view: TextureView,
}
impl Demo {
    pub fn new(gpu: &Gpu) -> GraphicsResult<Self> {
        let mut two_d = RenderState2D::new(gpu)?;
        let three_d = RenderState3D::new(gpu)?;
        let (v, i) = cube();
        let mesh = Mesh::upload(gpu, &v, &i)?;
        let image = texture_2d(
            &gpu.device,
            "shared checker",
            8,
            8,
            TextureFormat::Rgba8Unorm,
            TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING,
        )?;
        let pixels: Vec<_> = (0..64)
            .flat_map(|i| {
                if (i % 8 / 2 + i / 8 / 2) % 2 == 0 {
                    [225, 170, 75, 255]
                } else {
                    [35, 135, 185, 255]
                }
            })
            .collect();
        gpu.queue.write_texture(
            image.as_image_copy(),
            &pixels,
            vulkanic_gal::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(32),
                rows_per_image: Some(8),
            },
            image.size(),
        );
        let image_view = image.create_view(&Default::default());
        let mut textures = Textures::default();
        let shared_id = two_d.register_texture(gpu, &mut textures, &image_view)?;
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
    pub fn commands(
        &mut self,
        gpu: &Gpu,
        target: &RenderTarget,
        angle: f32,
        overlay: bool,
    ) -> GraphicsResult<Vec<CommandBuffer>> {
        let projection =
            Mat4::perspective(0.9, target.width as f32 / target.height as f32, 0.1, 100.).unwrap();
        let world = self.three_d.prepare(
            gpu,
            target,
            Camera3D {
                view_projection: projection * Mat4::translation(0., 0., -5.),
                light_direction: [0.4, 0.8, 1.],
            },
            &[Draw3D {
                mesh: &self.mesh,
                model: Mat4::rotation_y(angle) * Mat4::rotation_x(-0.35),
                material: Material::textured(&self.image_view),
            }],
            PassOptions::default(),
        )?;
        let mut commands = vec![world.commands];
        if overlay {
            let mut canvas =
                Canvas::new(target.width, target.height).map_err(quasar_frame::error)?;
            canvas.rect(Rect::new(12., 12., 270., 80.), [7, 15, 25, 210]);
            canvas.text([24., 24.], "ONE GAL / 3D + 2D", 2., [235, 245, 250, 255]);
            canvas.text([24., 48.], "SHARED GPU TEXTURE", 1., [135, 215, 220, 255]);
            canvas.sprite(Sprite::new(self.shared_id, Rect::new(232., 48., 32., 32.)));
            canvas.rect(
                Rect::new(
                    target.width as f32 / 2. - 8.,
                    target.height as f32 / 2. - 1.,
                    16.,
                    2.,
                ),
                [255, 255, 255, 128],
            );
            let hud = self.two_d.prepare(
                gpu,
                &canvas,
                &self.textures,
                target,
                DisplayOptions::default(),
                LoadOp::Load,
            )?;
            commands.push(hud.commands);
        }
        Ok(commands)
    }
    pub fn release_targets(&mut self, gpu: &Gpu) -> GraphicsResult<()> {
        self.two_d.release_targets(gpu)?;
        self.three_d.release_targets(gpu)
    }
    pub fn release(&mut self, gpu: &Gpu) -> GraphicsResult<()> {
        self.two_d.release(gpu)?;
        self.three_d.release(gpu)?;
        self.mesh.release(gpu)
    }
}
/// Direct upstream queue submission, retained as a comparison to frame helpers.
pub fn submit_explicit(
    gpu: &Gpu,
    prepare: impl FnOnce(&Gpu) -> GraphicsResult<Vec<CommandBuffer>>,
) -> GraphicsResult<(SubmissionIndex, FrameReport)> {
    let start = std::time::Instant::now();
    let commands = prepare(gpu)?;
    let prepare_nanos = quasar_frame::nanos(start.elapsed());
    let submit = std::time::Instant::now();
    let index = gpu.queue.submit(commands);
    Ok((
        index,
        FrameReport {
            prepare_nanos,
            submit_nanos: quasar_frame::nanos(submit.elapsed()),
            cpu_frame_nanos: quasar_frame::nanos(start.elapsed()),
            ..Default::default()
        },
    ))
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
    inner: OffscreenTarget,
    pub target: RenderTarget,
}
impl Offscreen {
    pub fn new(gpu: &Gpu, width: u32, height: u32) -> GraphicsResult<Self> {
        let inner = OffscreenTarget::new(gpu, width, height)?;
        Ok(Self {
            target: inner.target.clone(),
            inner,
        })
    }
    pub fn image(
        &mut self,
        gpu: &Gpu,
        demo: &mut Demo,
        angle: f32,
        overlay: bool,
    ) -> GraphicsResult<Vec<u8>> {
        Ok(self
            .inner
            .render(gpu, |gpu, target| {
                demo.commands(gpu, target, angle, overlay)
            })?
            .rgba)
    }
    pub fn release(self, _gpu: &Gpu) -> GraphicsResult<()> {
        Ok(())
    }
    pub fn texture(&self) -> &Texture {
        &self.inner.texture
    }
}
pub fn extent(width: u32, height: u32) -> vulkanic_gal::Extent3d {
    vulkanic_gal::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    }
}
#[cfg(test)]
mod tests;
