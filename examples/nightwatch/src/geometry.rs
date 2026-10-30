//! Procedural geometry and a shared white texture; no external assets required.
use quasar_frame::{Gpu, GraphicsResult, texture_2d};
use quasar_render_3d::Vertex;
use vulkanic_gal::*;
pub(crate) fn upload_white(gal: &Gpu) -> GraphicsResult<(Texture, TextureView)> {
    upload_rgba(gal, 1, 1, vec![255; 4])
}
pub(crate) fn upload_normal(gal: &Gpu, metal: bool) -> GraphicsResult<(Texture, TextureView)> {
    let mut pixels = Vec::with_capacity(64 * 64 * 4);
    let height = |x: f32, y: f32| {
        if metal {
            (x * std::f32::consts::TAU * 4.).sin() * (y * std::f32::consts::TAU * 4.).sin() * 0.012
        } else {
            (x * std::f32::consts::TAU * 7.).sin() * (y * std::f32::consts::TAU * 5.).cos() * 0.004
                + (x * std::f32::consts::TAU * 13.).cos() * 0.0015
        }
    };
    for y in 0..64 {
        for x in 0..64 {
            let (u, v) = (x as f32 / 64., y as f32 / 64.);
            let dx = (height(u + 1. / 64., v) - height(u - 1. / 64., v)) * 32.;
            let dy = (height(u, v + 1. / 64.) - height(u, v - 1. / 64.)) * 32.;
            let length = (dx * dx + dy * dy + 1.).sqrt();
            for n in [-dx / length, -dy / length, 1. / length] {
                pixels.push(((n * 0.5 + 0.5) * 255.).round() as u8);
            }
            pixels.push(255);
        }
    }
    upload_rgba(gal, 64, 64, pixels)
}
fn upload_rgba(
    gal: &Gpu,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
) -> GraphicsResult<(Texture, TextureView)> {
    let texture = texture_2d(
        &gal.device,
        "Nightwatch procedural texture",
        width,
        height,
        TextureFormat::Rgba8Unorm,
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
    )?;
    gal.queue.write_texture(
        texture.as_image_copy(),
        &pixels,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        texture.size(),
    );
    let view = texture.create_view(&Default::default());
    Ok((texture, view))
}

pub(crate) fn cube() -> (Vec<Vertex>, Vec<u32>) {
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
