//! CPU reference snapshots for headless CLI use and GPU conformance comparisons.
//! Native games submit the same Canvas to Renderer2D; this is never their presenter.
use crate::{Canvas, Filter, Image, Result, Textures};
pub fn rasterize(canvas: &Canvas, textures: &Textures) -> Result<Vec<u8>> {
    rasterize_with_filter(canvas, textures, Filter::Nearest)
}
pub fn rasterize_with_filter(
    canvas: &Canvas,
    textures: &Textures,
    filter: Filter,
) -> Result<Vec<u8>> {
    canvas.validate(textures)?;
    let mut pixels = canvas
        .clear
        .0
        .repeat((canvas.width * canvas.height) as usize);
    for q in &canvas.quads {
        let image = textures
            .get(q.texture)
            .ok_or_else(|| crate::invalid("GPU-only texture has no CPU reference pixels"))?;
        let source = q.source.unwrap_or(crate::Rect::new(
            0.0,
            0.0,
            image.width as f32,
            image.height as f32,
        ));
        let det = q.x[0] * q.y[1] - q.x[1] * q.y[0];
        if det.abs() < 1e-12 {
            continue;
        }
        let xs = [
            q.origin[0],
            q.origin[0] + q.x[0],
            q.origin[0] + q.y[0],
            q.origin[0] + q.x[0] + q.y[0],
        ];
        let ys = [
            q.origin[1],
            q.origin[1] + q.x[1],
            q.origin[1] + q.y[1],
            q.origin[1] + q.x[1] + q.y[1],
        ];
        let bounds = q.clip.bounds();
        let min_x = xs
            .into_iter()
            .fold(f32::INFINITY, f32::min)
            .max(bounds[0])
            .floor()
            .max(0.0) as u32;
        let max_x = xs
            .into_iter()
            .fold(f32::NEG_INFINITY, f32::max)
            .min(bounds[2])
            .ceil()
            .min(canvas.width as f32)
            .max(0.0) as u32;
        let min_y = ys
            .into_iter()
            .fold(f32::INFINITY, f32::min)
            .max(bounds[1])
            .floor()
            .max(0.0) as u32;
        let max_y = ys
            .into_iter()
            .fold(f32::NEG_INFINITY, f32::max)
            .min(bounds[3])
            .ceil()
            .min(canvas.height as f32)
            .max(0.0) as u32;
        for y in min_y..max_y {
            for x in min_x..max_x {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                if px < bounds[0] || py < bounds[1] || px >= bounds[2] || py >= bounds[3] {
                    continue;
                }
                let dx = px - q.origin[0];
                let dy = py - q.origin[1];
                let u = (dx * q.y[1] - dy * q.y[0]) / det;
                let v = (q.x[0] * dy - q.x[1] * dx) / det;
                if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                    continue;
                }
                let texel = sample(
                    image,
                    source.x + u * source.width,
                    source.y + v * source.height,
                    q.filter.unwrap_or(filter),
                );
                let src: [f32; 4] = std::array::from_fn(|i| texel[i] * q.color.0[i] as f32 / 255.0);
                let index = ((y * canvas.width + x) * 4) as usize;
                for c in 0..4 {
                    let value = if c == 3 {
                        src[c] + pixels[index + c] as f32 * (1.0 - src[3] / 255.0)
                    } else {
                        src[c] * (src[3] / 255.0)
                            + pixels[index + c] as f32 * (1.0 - src[3] / 255.0)
                    };
                    pixels[index + c] = value.round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    Ok(pixels)
}
fn sample(image: &Image, x: f32, y: f32, filter: Filter) -> [f32; 4] {
    let pixel = |x: i32, y: i32| {
        let index = ((y.clamp(0, image.height as i32 - 1) as u32 * image.width
            + x.clamp(0, image.width as i32 - 1) as u32)
            * 4) as usize;
        [
            image.rgba[index] as f32,
            image.rgba[index + 1] as f32,
            image.rgba[index + 2] as f32,
            image.rgba[index + 3] as f32,
        ]
    };
    if filter == Filter::Nearest {
        return pixel(x.floor() as i32, y.floor() as i32);
    }
    let x = x - 0.5;
    let y = y - 0.5;
    let a = x - x.floor();
    let b = y - y.floor();
    let p = pixel(x.floor() as i32, y.floor() as i32);
    let q = pixel(x.floor() as i32 + 1, y.floor() as i32);
    let r = pixel(x.floor() as i32, y.floor() as i32 + 1);
    let s = pixel(x.floor() as i32 + 1, y.floor() as i32 + 1);
    std::array::from_fn(|i| {
        p[i] * (1.0 - a) * (1.0 - b) + q[i] * a * (1.0 - b) + r[i] * (1.0 - a) * b + s[i] * a * b
    })
}
