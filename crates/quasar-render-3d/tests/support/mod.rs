use vulkanic_gal::{commands::BufferImageCopyRegion, resources::*, *};
pub fn upload_color(gal: &mut VulkanicGal, color: [u8; 4]) -> GalResult<(Handle, Handle)> {
    upload_pixels(gal, 1, 1, color.to_vec())
}
pub fn upload_pixels(
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
        label: "surface-test-color".into(),
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        extent: size,
        mip_levels: 1,
        array_layers: 1,
        usages: vec![TextureUsage::Sampled, TextureUsage::TransferDst],
    })?;
    let view = match gal.create_texture_view(TextureViewDesc {
        label: "surface-test-color-view".into(),
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
        label: "surface-test-color-upload".into(),
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
        label: "surface-test-color-upload".into(),
        command_lists: vec![CommandList::from(CommandListDesc {
            label: "surface-test-color-upload".into(),
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
