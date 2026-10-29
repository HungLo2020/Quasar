//! Headless-friendly palette/priority/live-edit demonstration using Tilequest's map.
use quasar_render_2d::*;
use quasar_tilemap::{TileEdit, render::View};
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut world =
        tilequest::World::load(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/island.tmj"))?;
    let art = tilequest::Art::load(&world)?; // Asset-service PNG, shared preloaded atlas.
    let stats = world.runtime.set_tiles(&[
        TileEdit {
            layer: world.ground,
            x: 2,
            y: 2,
            gid: 2,
        },
        TileEdit {
            layer: world.ground,
            x: 3,
            y: 2,
            gid: 2,
        },
    ])?;
    let mut canvas = Canvas::new(256, 128)?;
    art.tiles.draw_layer(
        &world.runtime,
        world.ground,
        &mut canvas,
        View {
            x: 0.,
            y: 0.,
            width: 256,
            height: 128,
            zoom: 1.,
        },
        0,
        Filter::Nearest,
    )?;
    let mut textures = art.textures;
    let image = IndexedImage::new(4, 2, [0, 1, 1, 0, 1, 2, 2, 1])?;
    let indices = textures.insert(image.image().clone());
    let base = Palette::new(vec![
        Color([25, 30, 35, 255]),
        Color([40, 140, 80, 255]),
        Color([240, 180, 40, 255]),
    ])?;
    let pri = Priority2D::default();
    for frame in 0..4 {
        let p = textures.insert(base.cycled(1..3, frame)?.image());
        canvas.set_palette(Some(PaletteBinding {
            texture: p,
            transparent_index: None,
        }));
        let destination = Rect::new(frame as f32 * 64., 72., 64., 32.);
        canvas.background_sprite(
            Sprite::new(indices, destination),
            SpriteTransform::default(),
            pri,
            Some(0),
        );
        canvas.set_palette(Some(PaletteBinding {
            texture: p,
            transparent_index: Some(0),
        }));
        canvas.priority_sprite(
            Sprite::new(indices, Rect::new(destination.x + 8., 64., 32., 48.)),
            SpriteTransform {
                flip_x: true,
                ..Default::default()
            },
            pri,
            true,
        );
        canvas.priority_sprite(
            Sprite::new(indices, Rect::new(destination.x + 16., 64., 32., 48.)),
            SpriteTransform::default(),
            pri,
            false,
        );
    }
    canvas.set_palette(None);
    canvas.text(
        [4., 116.],
        "PALETTES / MASKS / LIVE EDITS",
        1.,
        Color::WHITE,
    );
    let cpu = reference::rasterize(&canvas, &textures)?;
    #[cfg(feature = "desktop")]
    if std::env::args().any(|a| a == "--gpu") {
        let mut renderer = Renderer2D::headless()?;
        let gpu = renderer.render_to_image(&canvas, &textures)?;
        assert!(gpu.iter().zip(&cpu).all(|(a, b)| a.abs_diff(*b) <= 1));
        println!("GPU pixel parity passed ({})", renderer.device_name());
    }
    let output = std::env::args()
        .skip(1)
        .find(|a| !a.starts_with("--"))
        .unwrap_or_else(|| "/tmp/quasar-retro.png".into());
    write_png(&output, canvas.width(), canvas.height(), &cpu)?;
    println!("{output}: {stats:?}");
    Ok(())
}
