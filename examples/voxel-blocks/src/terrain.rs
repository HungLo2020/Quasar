//! Deterministic procedural terrain: rolling hills, beaches, caves and trees.
use crate::content::Blocks;
use quasar_core::noise::{Fractal, Noise};
use quasar_math::IVec3;
use quasar_voxel::{AIR, ChannelId, Chunk, ChunkLayout, Cube16, Schema, VoxelSource};

/// World height in chunks (16 blocks each).
pub const HEIGHT_CHUNKS: i32 = 5;
pub const WORLD_TOP: i32 = HEIGHT_CHUNKS * 16;
const SEA: i32 = 26;

#[derive(Clone)]
pub struct Terrain {
    pub seed: u64,
    pub block: ChannelId<u16>,
    pub blocks: Blocks,
}

fn seed32(seed: u64) -> u32 {
    quasar_core::noise::hash_u64(seed) as u32
}

fn hash2(seed: u64, x: i32, z: i32) -> u32 {
    quasar_core::noise::hash2(seed32(seed), x, z)
}

fn hash3(seed: u64, p: IVec3) -> u32 {
    quasar_core::noise::hash3(seed32(seed), p.x, p.y, p.z)
}

/// Value noise in `[0, 1]` on a lattice of `scale` blocks.
fn noise(seed: u64, x: f32, z: f32, scale: f32) -> f32 {
    Noise::new(seed).value2(x / scale, z / scale)
}

impl Terrain {
    pub fn height(&self, x: i32, z: i32) -> i32 {
        let (x, z) = (x as f32, z as f32);
        let hills = noise(self.seed, x, z, 48.0) * 14.0
            + noise(self.seed ^ 1, x, z, 17.0) * 5.0
            + noise(self.seed ^ 2, x, z, 6.0) * 1.5;
        (20.0 + hills) as i32
    }
    fn cave(&self, p: IVec3, surface: i32) -> bool {
        if p.y < 3 || p.y > surface - 4 {
            return false;
        }
        let worm = noise(
            self.seed ^ 77,
            p.x as f32 + p.y as f32 * 1.7,
            p.z as f32 - p.y as f32 * 1.3,
            11.0,
        );
        (worm - 0.5).abs() < 0.035
    }
    /// Crystal seams: thin sheets where 3D fractal noise crosses zero,
    /// only well below the surface.
    fn ore(&self, p: IVec3) -> bool {
        if p.y > 18 {
            return false;
        }
        let n = Noise::new(self.seed ^ 0x0e5).fbm3(
            p.x as f32,
            p.y as f32 * 1.6,
            p.z as f32,
            Fractal::new(0.045, 3),
        );
        n.abs() < 0.035
    }
    fn tree_at(&self, x: i32, z: i32) -> Option<i32> {
        let h = self.height(x, z);
        (h > SEA + 1 && hash2(self.seed ^ 0x7ee, x, z).is_multiple_of(61)).then_some(h)
    }
    /// The block at a world cell.
    pub fn block_at(&self, p: IVec3) -> u16 {
        let b = &self.blocks;
        let h = self.height(p.x, p.z);
        if p.y <= h {
            if self.cave(p, h) {
                return AIR;
            }
            let beach = h <= SEA + 1;
            return if p.y == h {
                if beach { b.sand } else { b.grass }
            } else if p.y > h - 4 {
                if beach { b.sand } else { b.dirt }
            } else if self.ore(p) {
                b.ore
            } else {
                b.stone
            };
        }
        for dz in -2..=2 {
            for dx in -2..=2 {
                let (tx, tz) = (p.x + dx, p.z + dz);
                if let Some(ground) = self.tree_at(tx, tz) {
                    let trunk = 4 + (hash2(self.seed ^ 5, tx, tz) % 2) as i32;
                    if let Some(block) = tree_cell(self.seed, p, tx, tz, ground, trunk, b) {
                        return block;
                    }
                }
            }
        }
        AIR
    }
}

/// Trees: trunks of 4-5 logs with a leaf canopy.
fn tree_cell(
    seed: u64,
    p: IVec3,
    tx: i32,
    tz: i32,
    ground: i32,
    trunk: i32,
    b: &Blocks,
) -> Option<u16> {
    let (dx, dz) = (p.x - tx, p.z - tz);
    let top = ground + trunk;
    let dy = p.y - ground;
    if dx == 0 && dz == 0 && dy >= 1 && dy <= trunk {
        return Some(b.log);
    }
    let radius = if p.y >= top { 1 } else { 2 };
    if p.y >= top - 1 && p.y <= top + 1 && dx.abs() <= radius && dz.abs() <= radius {
        let corner = dx.abs() == radius && dz.abs() == radius;
        if !corner || hash3(seed, p).is_multiple_of(3) {
            return Some(b.leaves);
        }
    }
    None
}

impl VoxelSource<Cube16> for Terrain {
    fn generate(&self, schema: &Schema, coord: IVec3, _lod: u32, chunk: &mut Chunk<Cube16>) {
        if coord.y < 0 || coord.y >= HEIGHT_CHUNKS {
            return;
        }
        let origin = coord * 16;
        // Heights for the chunk plus a 2-block margin (tree canopies).
        let mut heights = [[0i32; 20]; 20];
        let mut max_h = i32::MIN;
        for (z, row) in heights.iter_mut().enumerate() {
            for (x, h) in row.iter_mut().enumerate() {
                *h = self.height(origin.x + x as i32 - 2, origin.z + z as i32 - 2);
                max_h = max_h.max(*h);
            }
        }
        // Fast path: chunks entirely above the highest possible tree are air.
        if origin.y > max_h + 8 {
            return;
        }
        let mut trees = Vec::new();
        for (z, row) in heights.iter().enumerate() {
            for (x, &ground) in row.iter().enumerate() {
                let (tx, tz) = (origin.x + x as i32 - 2, origin.z + z as i32 - 2);
                if ground > SEA + 1 && hash2(self.seed ^ 0x7ee, tx, tz).is_multiple_of(61) {
                    trees.push((
                        tx,
                        tz,
                        ground,
                        4 + (hash2(self.seed ^ 5, tx, tz) % 2) as i32,
                    ));
                }
            }
        }
        let b = &self.blocks;
        let mut values = vec![AIR; Cube16::VOLUME];
        for (i, v) in values.iter_mut().enumerate() {
            let local = Cube16::local(i).as_ivec3();
            let p = origin + local;
            let h = heights[local.z as usize + 2][local.x as usize + 2];
            *v = if p.y <= h {
                let beach = h <= SEA + 1;
                if self.cave(p, h) {
                    AIR
                } else if p.y == h {
                    if beach { b.sand } else { b.grass }
                } else if p.y > h - 4 {
                    if beach { b.sand } else { b.dirt }
                } else if self.ore(p) {
                    b.ore
                } else {
                    b.stone
                }
            } else {
                trees
                    .iter()
                    .find_map(|&(tx, tz, ground, trunk)| {
                        tree_cell(self.seed, p, tx, tz, ground, trunk, b)
                    })
                    .unwrap_or(AIR)
            };
        }
        chunk.write_all(schema, self.block, &values);
    }
}
