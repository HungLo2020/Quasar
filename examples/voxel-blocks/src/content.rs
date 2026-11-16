//! Block definitions, procedural textures and per-block tables.
use quasar_math::Vec3;
use quasar_voxel::{BlockCatalog, BlockDef, FaceShape, Layer};
use quasar_voxel_mesh::{ModelLibrary, box_quads, opaque_table};

/// Texture tile edge in texels.
pub const TILE: u32 = 16;

pub mod tex {
    pub const STONE: u32 = 0;
    pub const DIRT: u32 = 1;
    pub const GRASS_TOP: u32 = 2;
    pub const GRASS_SIDE: u32 = 3;
    pub const GLASS: u32 = 4;
    pub const LEAVES: u32 = 5;
    pub const LAMP: u32 = 6;
    pub const PLANKS: u32 = 7;
    pub const SAND: u32 = 8;
    pub const LOG_SIDE: u32 = 9;
    pub const LOG_TOP: u32 = 10;
    pub const COUNT: u32 = 11;
}

/// Block IDs (indices into [`Content::catalog`]).
#[derive(Clone, Copy, Debug)]
pub struct Blocks {
    pub stone: u16,
    pub dirt: u16,
    pub grass: u16,
    pub sand: u16,
    pub log: u16,
    pub leaves: u16,
    pub glass: u16,
    pub lamp: u16,
    pub planks: u16,
    pub slab: u16,
}

/// Everything meshing, lighting and collision need about blocks. Shared
/// read-only with meshing jobs.
pub struct Content {
    pub catalog: BlockCatalog,
    pub library: ModelLibrary,
    pub blocks: Blocks,
    /// Per block ID: fully blocks light and ambient occlusion.
    pub opaque: Vec<bool>,
    /// Per block ID: light emitted.
    pub emission: Vec<u8>,
    /// Per block ID: extra light attenuation (leaves).
    pub filter: Vec<u8>,
    /// Per block ID: collision box height (0 = passable).
    pub collision: Vec<f32>,
    pub hotbar: Vec<u16>,
}

impl Content {
    pub fn new() -> Self {
        let mut catalog = BlockCatalog::new();
        let mut library = ModelLibrary::new();
        let side = |s: u32, top: u32, bottom: u32| [s, s, bottom, top, s, s];
        let stone = catalog.add(BlockDef::cube("stone", tex::STONE));
        let dirt = catalog.add(BlockDef::cube("dirt", tex::DIRT));
        let grass = catalog.add(
            BlockDef::cube("grass", tex::GRASS_SIDE).with_materials(side(
                tex::GRASS_SIDE,
                tex::GRASS_TOP,
                tex::DIRT,
            )),
        );
        let sand = catalog.add(BlockDef::cube("sand", tex::SAND));
        let log = catalog.add(BlockDef::cube("log", tex::LOG_SIDE).with_materials(side(
            tex::LOG_SIDE,
            tex::LOG_TOP,
            tex::LOG_TOP,
        )));
        let leaves = catalog.add(BlockDef::cube("leaves", tex::LEAVES).with_layer(Layer::Cutout));
        let glass = catalog.add(BlockDef::cube("glass", tex::GLASS).with_layer(Layer::Translucent));
        let lamp = catalog.add(BlockDef::cube("lamp", tex::LAMP).with_emission(14));
        let planks = catalog.add(BlockDef::cube("planks", tex::PLANKS));
        // A half block: full bottom face, open top, half-height sides that
        // only cull against other half blocks.
        let slab_model = library.add(box_quads(
            Vec3::ZERO,
            Vec3::new(1.0, 0.5, 1.0),
            [tex::PLANKS; 6],
        ));
        let half = FaceShape::Shape(1);
        let slab = catalog.add(
            BlockDef::cube("slab", tex::PLANKS)
                .with_model(slab_model)
                .with_faces([half, half, FaceShape::Full, FaceShape::None, half, half]),
        );
        let blocks = Blocks {
            stone,
            dirt,
            grass,
            sand,
            log,
            leaves,
            glass,
            lamp,
            planks,
            slab,
        };
        let opaque = opaque_table(&catalog);
        let emission = catalog.defs().iter().map(|d| d.emission).collect();
        let filter = catalog
            .defs()
            .iter()
            .map(|d| u8::from(d.layer == Layer::Cutout))
            .collect();
        let collision = catalog
            .defs()
            .iter()
            .enumerate()
            .map(|(id, d)| match (d.collision, id as u16 == slab) {
                (false, _) => 0.0,
                (true, true) => 0.5,
                (true, false) => 1.0,
            })
            .collect();
        Self {
            catalog,
            library,
            blocks,
            opaque,
            emission,
            filter,
            collision,
            hotbar: vec![stone, dirt, grass, planks, slab, glass, leaves, lamp, log],
        }
    }
    pub fn name(&self, id: u16) -> &str {
        self.catalog.get(id).map_or("?", |d| d.name.as_str())
    }
}

impl Default for Content {
    fn default() -> Self {
        Self::new()
    }
}

fn hash(x: u32, y: u32, salt: u32) -> u32 {
    let mut h =
        x.wrapping_mul(0x9e37_79b1) ^ y.wrapping_mul(0x85eb_ca77) ^ salt.wrapping_mul(0xc2b2_ae3d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h
}

/// Procedural sRGB RGBA8 texture layers, one per [`tex`] constant.
pub fn texture_layers() -> Vec<Vec<u8>> {
    (0..tex::COUNT)
        .map(|layer| {
            let mut pixels = Vec::with_capacity((TILE * TILE * 4) as usize);
            for y in 0..TILE {
                for x in 0..TILE {
                    let n = (hash(x, y, layer) & 0xff) as f32 / 255.0;
                    let jitter = |c: [f32; 3], amount: f32| {
                        c.map(|v| (v * (1.0 - amount + 2.0 * amount * n)).clamp(0.0, 1.0))
                    };
                    let (rgb, alpha) = match layer {
                        tex::STONE => (jitter([0.50, 0.50, 0.52], 0.18), 1.0),
                        tex::DIRT => (jitter([0.45, 0.31, 0.20], 0.2), 1.0),
                        tex::GRASS_TOP => (jitter([0.33, 0.62, 0.22], 0.18), 1.0),
                        tex::GRASS_SIDE => {
                            let edge = 3 + (hash(x, 0, 99) % 3);
                            if y < edge {
                                (jitter([0.33, 0.62, 0.22], 0.18), 1.0)
                            } else {
                                (jitter([0.45, 0.31, 0.20], 0.2), 1.0)
                            }
                        }
                        tex::GLASS => {
                            let frame = x == 0 || y == 0 || x == TILE - 1 || y == TILE - 1;
                            if frame {
                                ([0.85, 0.92, 0.95], 0.9)
                            } else {
                                ([0.70, 0.85, 0.95], 0.22)
                            }
                        }
                        tex::LEAVES => {
                            if hash(x, y, 7).is_multiple_of(5) {
                                ([0.0; 3], 0.0)
                            } else {
                                (jitter([0.22, 0.48, 0.16], 0.3), 1.0)
                            }
                        }
                        tex::LAMP => {
                            let border = x < 2 || y < 2 || x >= TILE - 2 || y >= TILE - 2;
                            if border {
                                ([0.55, 0.42, 0.25], 1.0)
                            } else {
                                (jitter([1.0, 0.86, 0.52], 0.08), 1.0)
                            }
                        }
                        tex::PLANKS => {
                            let seam = y % 4 == 3 || (x + (y / 4) * 5) % 16 == 0;
                            if seam {
                                ([0.42, 0.30, 0.17], 1.0)
                            } else {
                                (jitter([0.70, 0.53, 0.32], 0.1), 1.0)
                            }
                        }
                        tex::SAND => (jitter([0.86, 0.80, 0.58], 0.08), 1.0),
                        tex::LOG_SIDE => {
                            let stripe = (x + hash(0, y, 5) % 2).is_multiple_of(4);
                            if stripe {
                                ([0.30, 0.21, 0.12], 1.0)
                            } else {
                                (jitter([0.42, 0.30, 0.18], 0.1), 1.0)
                            }
                        }
                        _ => {
                            let d = ((x as f32 - 7.5).powi(2) + (y as f32 - 7.5).powi(2)).sqrt();
                            if (d as u32).is_multiple_of(3) {
                                ([0.50, 0.37, 0.22], 1.0)
                            } else {
                                ([0.66, 0.52, 0.32], 1.0)
                            }
                        }
                    };
                    // Colours above are sRGB already.
                    let byte = |v: f32| (v * 255.0).round() as u8;
                    pixels.extend([byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), byte(alpha)]);
                }
            }
            pixels
        })
        .collect()
}
