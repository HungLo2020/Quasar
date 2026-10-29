//! Procedural geometry and a shared white texture; no external assets required.
use quasar_render_3d::Vertex;
use vulkanic_gal::{commands::BufferImageCopyRegion, resources::*, *};
pub(crate) fn upload_white(gal: &mut VulkanicGal) -> GalResult<(Handle, Handle)> {
    upload_rgba(gal, 1, 1, vec![255; 4])
}
pub(crate) fn upload_normal(gal: &mut VulkanicGal, metal: bool) -> GalResult<(Handle, Handle)> {
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
    gal: &mut VulkanicGal,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
) -> GalResult<(Handle, Handle)> {
    let size = Extent3d {
        width,
        height,
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
        size: pixels.len() as u64,
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
                    data: pixels,
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
                    bytes_per_row: width * 4,
                    rows_per_image: height,
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
