use quasar_frame::{Gpu, GraphicsResult, texture_2d};
use vulkanic_gal::*;
pub fn upload_color(gpu: &Gpu, color: [u8; 4]) -> GraphicsResult<(Texture, TextureView)> {
    upload_pixels(gpu, 1, 1, color.to_vec())
}
pub fn upload_pixels(
    gpu: &Gpu,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
) -> GraphicsResult<(Texture, TextureView)> {
    let texture = texture_2d(
        &gpu.device,
        "test pixels",
        width,
        height,
        TextureFormat::Rgba8Unorm,
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
    )?;
    gpu.queue.write_texture(
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
