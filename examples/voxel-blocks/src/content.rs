//! Block definitions, procedural textures and per-block tables.
use quasar_voxel::{BlockCatalog, BlockDef, Layer};
use quasar_voxel_mesh::{ModelLibrary, opaque_table};

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
    /// Stone with crystal flecks, and the mask of the flecks that glow.
    pub const ORE: u32 = 11;
    pub const ORE_GLOW: u32 = 12;
    pub const COUNT: u32 = 13;
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
    /// Glowing crystal ore (emissive mask), seamed through deep stone.
    pub ore: u16,
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
    /// Per block ID for the sculpted style: `Some(scale)` meshes the block
    /// with bevelled edges ([`quasar_voxel_mesh::BevelExtractor`]), `None`
    /// with the classic per-block models.
    pub bevel: Vec<Option<f32>>,
    /// Mesh with bevelled stone instead of sharp cubes.
    pub sculpted: bool,
}

impl Content {
    pub fn new() -> Self {
        let mut catalog = BlockCatalog::new();
        let library = ModelLibrary::new();
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
        // A half block: a native partial-height block. Its full bottom face,
        // open top and half-height sides (which only cull against other half
        // blocks) are derived from its bounds.
        let slab = catalog.add(BlockDef::cube("slab", tex::PLANKS).with_height(0.5));
        // Glows through its emissive mask only; worldgen places no light
        // emitters, so it adds no voxel light.
        let ore = catalog.add(BlockDef::cube("crystal ore", tex::ORE));
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
            ore,
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
            .map(|d| if d.collision { d.bounds[1][1] } else { 0.0 })
            .collect();
        let bevel = (0..catalog_len(&blocks))
            .map(|id| (id == stone || id == ore).then_some(1.0))
            .collect();
        Self {
            library,
            blocks,
            opaque,
            emission,
            filter,
            collision,
            hotbar: vec![stone, dirt, grass, planks, slab, glass, leaves, lamp, log],
            bevel,
            catalog,
            sculpted: false,
        }
    }
    /// The same content meshed in the sculpted style.
    pub fn sculpted() -> Self {
        Self {
            sculpted: true,
            ..Self::new()
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

/// One past the highest block ID.
fn catalog_len(blocks: &Blocks) -> u16 {
    blocks.ore + 1
}

fn hash(x: u32, y: u32, salt: u32) -> u32 {
    let mut h =
        x.wrapping_mul(0x9e37_79b1) ^ y.wrapping_mul(0x85eb_ca77) ^ salt.wrapping_mul(0xc2b2_ae3d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h
}

/// Crystal flecks of the ore texture (and its glow mask).
fn crystal_fleck(x: u32, y: u32) -> bool {
    let (cx, cy) = (x / 4, y / 4);
    let seed = hash(cx, cy, 41);
    let (fx, fy) = (cx * 4 + seed % 3, cy * 4 + (seed >> 8) % 3);
    !seed.is_multiple_of(3) && (x == fx || x == fx + 1) && (y == fy || y == fy + 1)
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
                        tex::ORE | tex::ORE_GLOW => {
                            let fleck = crystal_fleck(x, y);
                            match (layer, fleck) {
                                (tex::ORE, true) => ([0.55, 0.95, 0.88], 1.0),
                                (tex::ORE, false) => (jitter([0.46, 0.46, 0.50], 0.18), 1.0),
                                (_, true) => ([1.0, 1.0, 1.0], 1.0),
                                (_, false) => ([0.0, 0.0, 0.0], 1.0),
                            }
                        }
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
