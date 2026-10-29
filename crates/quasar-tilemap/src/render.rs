//! Atlas loading and ordered tile submission; never creates a graphics device.
use crate::*;
#[cfg(feature = "png")]
use quasar_render_2d::Image;
use quasar_render_2d::{
    Camera2D, Canvas, Color, Filter, Rect, SpriteAtlas, SpriteTransform, Textures,
};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Copy)]
pub struct View {
    pub x: f64,
    pub y: f64,
    pub width: u32,
    pub height: u32,
    pub zoom: f32,
}
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct DrawStats {
    pub visited_cells: usize,
    pub emitted_tiles: usize,
    pub visited_rows: usize,
}
impl std::ops::AddAssign for DrawStats {
    fn add_assign(&mut self, other: Self) {
        self.visited_cells += other.visited_cells;
        self.emitted_tiles += other.emitted_tiles;
        self.visited_rows += other.visited_rows;
    }
}
pub struct LoadedTilesets {
    atlases: Vec<SpriteAtlas>,
    definitions: Vec<Tileset>,
}
impl LoadedTilesets {
    #[cfg(feature = "png")]
    /// Loads each unique PNG once. Enforces declared image dimensions and a
    /// 256 MiB aggregate RGBA budget. No GPU or window is initialized.
    pub fn load(map: &Map, root: &AssetRoot, textures: &mut Textures) -> Result<Self> {
        map.validate()?;
        let mut images = BTreeMap::new();
        let mut atlases = Vec::new();
        let mut inserted = Vec::new();
        let mut total = 0usize;
        let result = (|| {
            for set in &map.tilesets {
                let path = crate::io::relative("", &set.image)?;
                let (id, width, height) = if let Some(info) = images.get(&path) {
                    *info
                } else {
                    let image = Image::load_png(root.resolve(&path)?)
                        .map_err(|e| error("png", e.to_string()))?;
                    total = total
                        .checked_add(image.rgba().len())
                        .ok_or_else(|| error("bounds", "image budget overflow"))?;
                    if total > 256 * 1024 * 1024 {
                        return Err(error("bounds", "tileset images exceed 256 MiB"));
                    }
                    let width = image.width();
                    let height = image.height();
                    let id = textures.insert(image);
                    inserted.push(id);
                    images.insert(path, (id, width, height));
                    (id, width, height)
                };
                let atlas = tileset_atlas(set, id, (width, height))?;
                atlases.push(atlas);
            }
            Ok(Self {
                atlases,
                definitions: map.tilesets.clone(),
            })
        })();
        if result.is_err() {
            for id in inserted {
                textures.remove(id);
            }
        }
        result
    }
    /// Builds atlas metadata from caller-owned texture identities. No file I/O,
    /// pixel copy, GPU allocation or texture ownership transfer occurs. The resolver
    /// runs once per unique image path; register GAL views with RenderState2D first.
    pub fn from_textures(
        map: &Map,
        textures: &Textures,
        mut resolve: impl FnMut(&str) -> Result<quasar_render_2d::TextureId>,
    ) -> Result<Self> {
        map.validate()?;
        let mut images = BTreeMap::new();
        let mut atlases = Vec::with_capacity(map.tilesets.len());
        for set in &map.tilesets {
            let path = crate::io::relative("", &set.image)?;
            let id = if let Some(&id) = images.get(&path) {
                id
            } else {
                let id = resolve(&path)?;
                images.insert(path, id);
                id
            };
            let (width, height) = textures
                .dimensions(id)
                .ok_or_else(|| error("atlas", "unknown preloaded texture"))?;
            let atlas = tileset_atlas(set, id, (width, height))?;
            atlases.push(atlas);
        }
        Ok(Self {
            atlases,
            definitions: map.tilesets.clone(),
        })
    }
    pub fn atlas(&self, index: usize) -> Option<&SpriteAtlas> {
        self.atlases.get(index)
    }
    /// Draws one explicit layer; camera is restored so HUD and custom draws can
    /// coexist. Start with an identity object transform for world tile rendering.
    pub fn draw_layer(
        &self,
        runtime: &Runtime,
        layer_id: u32,
        canvas: &mut Canvas,
        view: View,
        elapsed_ms: u64,
        filter: Filter,
    ) -> Result<DrawStats> {
        if !view.x.is_finite()
            || !view.y.is_finite()
            || view.x.abs() > 1e9
            || view.y.abs() > 1e9
            || !view.zoom.is_finite()
            || view.zoom <= 0.
            || view.zoom > 64.
            || canvas.width() != view.width
            || canvas.height() != view.height
        {
            return Err(error("view", "invalid camera or canvas size"));
        }
        if self.definitions != runtime.map().tilesets {
            return Err(error("atlas", "loaded atlas declarations differ from map"));
        }
        let layer = runtime
            .map()
            .layer(layer_id)
            .ok_or_else(|| error("layer", "unknown draw layer"))?;
        if !layer.visible
            || layer.opacity == 0.
            || matches!(layer.content, LayerContent::Objects { .. })
        {
            return Ok(DrawStats::default());
        }
        let origin = [
            view.x * f64::from(layer.parallax[0]),
            view.y * f64::from(layer.parallax[1]),
        ];
        let bounds = Bounds {
            x: origin[0],
            y: origin[1],
            width: f64::from(view.width) / f64::from(view.zoom),
            height: f64::from(view.height) / f64::from(view.zoom),
        };
        let previous = canvas.camera();
        canvas.set_camera(Camera2D {
            center: [
                (origin[0] + bounds.width / 2.) as f32,
                (origin[1] + bounds.height / 2.) as f32,
            ],
            zoom: view.zoom,
            rotation: 0.,
        });
        let mut emitted = 0;
        let map = runtime.map();
        let result = runtime.for_each_cell(layer_id, bounds, |cell| {
            let (set, local) = map.resolve(cell.gid).expect("validated tile").unwrap();
            let local = runtime
                .animated_tile(set, local, elapsed_ms)
                .expect("validated tile");
            let destination = Rect::new(
                cell.x as f32 * map.tile_width as f32 + layer.offset[0],
                cell.y as f32 * map.tile_height as f32 + layer.offset[1],
                map.tile_width as f32,
                map.tile_height as f32,
            );
            let mut sprite = self.atlases[set]
                .sprite(local, destination)
                .expect("validated atlas region");
            let mut color = layer.tint;
            color[3] = (f32::from(color[3]) * layer.opacity).round() as u8;
            sprite.tint = Color(color);
            sprite.filter = Some(filter);
            canvas.sprite_transformed(
                sprite,
                SpriteTransform {
                    transpose: cell.gid & TRANSPOSE != 0,
                    flip_x: cell.gid & FLIP_X != 0,
                    flip_y: cell.gid & FLIP_Y != 0,
                },
            );
            emitted += 1;
        });
        canvas.set_camera(previous);
        let query = result?;
        Ok(DrawStats {
            visited_cells: query.visited_cells,
            emitted_tiles: emitted,
            visited_rows: query.visited_rows,
        })
    }
}
#[cfg(feature = "png")]
pub fn preview(
    runtime: &Runtime,
    root: &AssetRoot,
    view: View,
    elapsed_ms: u64,
) -> Result<(Vec<u8>, DrawStats)> {
    if u64::from(view.width) * u64::from(view.height) > 16 * 1024 * 1024 {
        return Err(error("bounds", "preview exceeds 64 MiB"));
    }
    let mut textures = Textures::default();
    let atlases = LoadedTilesets::load(runtime.map(), root, &mut textures)?;
    let mut canvas =
        Canvas::new(view.width, view.height).map_err(|e| error("view", e.to_string()))?;
    canvas.clear = Color(runtime.map().background);
    let mut stats = DrawStats::default();
    for layer in &runtime.map().layers {
        stats += atlases.draw_layer(
            runtime,
            layer.id,
            &mut canvas,
            view,
            elapsed_ms,
            Filter::Nearest,
        )?;
    }
    let pixels = quasar_render_2d::reference::rasterize(&canvas, &textures)
        .map_err(|e| error("render", e.to_string()))?;
    Ok((pixels, stats))
}
#[cfg(feature = "png")]
pub fn write_png(
    path: impl AsRef<std::path::Path>,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Result<()> {
    let file = std::fs::File::create(path).map_err(|e| error("io", e.to_string()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(pixels))
        .map_err(|e| error("png", e.to_string()))
}

fn tileset_atlas(
    set: &Tileset,
    id: quasar_render_2d::TextureId,
    (width, height): (u32, u32),
) -> Result<SpriteAtlas> {
    if (width, height) != (set.image_width, set.image_height) {
        return Err(error(
            "atlas",
            format!("{} image dimensions differ from tileset", set.image),
        ));
    }
    let atlas = SpriteAtlas::grid(
        id,
        [width, height],
        [set.tile_width, set.tile_height],
        set.margin,
        set.spacing,
    )
    .map_err(|e| error("atlas", e.to_string()))?;
    if atlas.columns() != set.columns || atlas.len() < set.tile_count {
        return Err(error("atlas", "atlas metadata mismatch"));
    }
    Ok(atlas)
}
