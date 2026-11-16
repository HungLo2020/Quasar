//! Materials, procedural textures and the ship block catalog.
use quasar_math::{Vec2, Vec3};
use quasar_voxel::{BlockCatalog, BlockDef, Face, FaceShape, Layer};
use quasar_voxel_mesh::{ModelLibrary, ModelQuad, cube_quad, opaque_table};

pub const TILE: u32 = 16;

/// Material table indices (texture layer = index).
pub mod mat {
    pub const ROCK: u32 = 0;
    pub const ORE: u32 = 1;
    pub const ICE: u32 = 2;
    pub const ARMOR: u32 = 3;
    pub const ARMOR_DARK: u32 = 4;
    pub const GLASS: u32 = 5;
    pub const THRUSTER: u32 = 6;
    pub const LIGHT: u32 = 7;
    pub const COUNT: u32 = 8;
}

#[derive(Clone, Copy, Debug)]
pub struct ShipBlocks {
    pub armor: u16,
    pub dark: u16,
    pub slope: u16,
    pub window: u16,
    pub thruster: u16,
    pub light: u16,
}

/// What a tool slot does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Drill,
    Filler,
    Block(u16),
}

pub struct ShipContent {
    pub catalog: BlockCatalog,
    pub library: ModelLibrary,
    pub blocks: ShipBlocks,
    pub opaque: Vec<bool>,
    pub tools: Vec<(Tool, &'static str)>,
}

/// A wedge rising from the front (+Z, height 0) to the back (-Z, full
/// height): full bottom and back faces, triangular sides, a sloped top.
fn slope_model(material: u32) -> Vec<ModelQuad> {
    let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
    let uv = |u: f32, w: f32| Vec2::new(u, w);
    let mut quads = vec![
        cube_quad(Face::NegY, material),
        cube_quad(Face::NegZ, material),
    ];
    quads.push(ModelQuad {
        positions: [v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 0.), v(0., 1., 0.)],
        uv: [uv(0., 1.), uv(1., 1.), uv(1., 0.), uv(0., 0.)],
        cull: None,
        face: None,
        material,
        shade: false,
        tint: u32::MAX,
    });
    // Triangles as quads with a repeated corner.
    quads.push(ModelQuad {
        positions: [v(0., 0., 0.), v(0., 0., 1.), v(0., 1., 0.), v(0., 1., 0.)],
        uv: [uv(0., 1.), uv(1., 1.), uv(0., 0.), uv(0., 0.)],
        cull: Some(Face::NegX),
        face: Some(Face::NegX),
        material,
        shade: false,
        tint: u32::MAX,
    });
    quads.push(ModelQuad {
        positions: [v(1., 0., 1.), v(1., 0., 0.), v(1., 1., 0.), v(1., 1., 0.)],
        uv: [uv(0., 1.), uv(1., 1.), uv(1., 0.), uv(1., 0.)],
        cull: Some(Face::PosX),
        face: Some(Face::PosX),
        material,
        shade: false,
        tint: u32::MAX,
    });
    quads
}

impl ShipContent {
    pub fn new() -> Self {
        let mut catalog = BlockCatalog::new();
        let mut library = ModelLibrary::new();
        let armor = catalog.add(BlockDef::cube("armor", mat::ARMOR));
        let dark = catalog.add(BlockDef::cube("dark armor", mat::ARMOR_DARK));
        let wedge = library.add(slope_model(mat::ARMOR));
        let side = FaceShape::Shape(2);
        // Mounts only on the full faces (bottom and back).
        let slope = catalog.add(
            BlockDef::cube("slope", mat::ARMOR)
                .with_model(wedge)
                .with_faces([
                    side,
                    side,
                    FaceShape::Full,
                    FaceShape::None,
                    FaceShape::Full,
                    FaceShape::None,
                ])
                .with_mounts(Face::NegY.bit() | Face::NegZ.bit()),
        );
        let window =
            catalog.add(BlockDef::cube("window", mat::GLASS).with_layer(Layer::Translucent));
        // Two cells long (local +Z): a multi-cell block.
        let thruster = catalog.add(
            BlockDef::cube("thruster", mat::THRUSTER)
                .with_materials([
                    mat::THRUSTER,
                    mat::THRUSTER,
                    mat::THRUSTER,
                    mat::THRUSTER,
                    mat::ARMOR_DARK,
                    mat::THRUSTER,
                ])
                .with_footprint(vec![quasar_math::IVec3::ZERO, quasar_math::IVec3::Z]),
        );
        let light = catalog.add(BlockDef::cube("light", mat::LIGHT).with_emission(12));
        let blocks = ShipBlocks {
            armor,
            dark,
            slope,
            window,
            thruster,
            light,
        };
        let opaque = opaque_table(&catalog);
        Self {
            catalog,
            library,
            blocks,
            opaque,
            tools: vec![
                (Tool::Drill, "DRILL"),
                (Tool::Filler, "FILLER"),
                (Tool::Block(armor), "ARMOR"),
                (Tool::Block(dark), "DARK"),
                (Tool::Block(slope), "SLOPE"),
                (Tool::Block(window), "WINDOW"),
                (Tool::Block(thruster), "THRUSTER"),
                (Tool::Block(light), "LIGHT"),
            ],
        }
    }
}

impl Default for ShipContent {
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

/// sRGB RGBA8 layers, one per [`mat`] entry.
pub fn texture_layers() -> Vec<Vec<u8>> {
    (0..mat::COUNT)
        .map(|layer| {
            let mut pixels = Vec::with_capacity((TILE * TILE * 4) as usize);
            for y in 0..TILE {
                for x in 0..TILE {
                    let n = (hash(x, y, layer) & 0xff) as f32 / 255.0;
                    let edge = x == 0 || y == 0 || x == TILE - 1 || y == TILE - 1;
                    let jitter = |c: [f32; 3], a: f32| {
                        c.map(|v| (v * (1.0 - a + 2.0 * a * n)).clamp(0.0, 1.0))
                    };
                    let (rgb, alpha) = match layer {
                        mat::ROCK => (jitter([0.42, 0.39, 0.36], 0.25), 1.0),
                        mat::ORE => {
                            if hash(x / 3, y / 3, 41).is_multiple_of(4) {
                                ([0.85, 0.55, 0.22], 1.0)
                            } else {
                                (jitter([0.40, 0.37, 0.35], 0.2), 1.0)
                            }
                        }
                        mat::ICE => (jitter([0.72, 0.84, 0.92], 0.08), 1.0),
                        mat::ARMOR => {
                            if edge {
                                ([0.48, 0.50, 0.53], 1.0)
                            } else {
                                (jitter([0.66, 0.68, 0.70], 0.04), 1.0)
                            }
                        }
                        mat::ARMOR_DARK => {
                            if edge {
                                ([0.16, 0.17, 0.19], 1.0)
                            } else {
                                (jitter([0.26, 0.28, 0.31], 0.05), 1.0)
                            }
                        }
                        mat::GLASS => {
                            if edge {
                                ([0.35, 0.40, 0.45], 1.0)
                            } else {
                                ([0.55, 0.78, 0.95], 0.3)
                            }
                        }
                        mat::THRUSTER => {
                            let ring = ((x as f32 - 7.5).powi(2) + (y as f32 - 7.5).powi(2)).sqrt();
                            if ring < 4.0 {
                                ([0.95, 0.55, 0.20], 1.0)
                            } else if edge {
                                ([0.2, 0.2, 0.22], 1.0)
                            } else {
                                ([0.36, 0.37, 0.40], 1.0)
                            }
                        }
                        _ => {
                            if edge {
                                ([0.3, 0.3, 0.3], 1.0)
                            } else {
                                ([1.0, 0.95, 0.75], 1.0)
                            }
                        }
                    };
                    let byte = |v: f32| (v * 255.0).round() as u8;
                    pixels.extend([byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), byte(alpha)]);
                }
            }
            pixels
        })
        .collect()
}
