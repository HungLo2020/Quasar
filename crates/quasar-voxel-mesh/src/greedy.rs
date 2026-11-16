//! Binary greedy meshing for full-cube blocks.
//!
//! Face visibility is computed per column with bit operations (one `u64` per
//! column, chunk edge + 2 padding cells ≤ 64), then coplanar faces with equal
//! material, AO and light keys merge into maximal rectangles. Output is not
//! per block; use [`crate::CulledModelExtractor`] when a game needs per-block
//! quads.
use crate::culled::tangents;
use crate::quad::{MeshSink, Quad};
use quasar_math::Vec3;
use quasar_voxel::{AIR, BlockCatalog, Face, Layer, Orientation, Snapshot};

/// Material rules for greedy meshing.
pub trait GreedyMaterials {
    /// Material of `block`'s `face` (world direction), or `None` when the
    /// block is not a greedy-meshed full cube.
    fn face_material(&self, block: u16, orientation: Orientation, face: Face) -> Option<u32>;
    /// Whether `neighbour` hides the touching face of a greedy cube.
    fn hides(&self, block: u16, neighbour: u16) -> bool;
    fn layer(&self, block: u16) -> Layer;
}

/// Catalog full cubes; opaque cubes hide neighbours, identical non-opaque
/// cubes hide their shared faces.
pub struct CatalogGreedy<'a> {
    catalog: &'a BlockCatalog,
    full: Vec<bool>,
    opaque: Vec<bool>,
}

impl<'a> CatalogGreedy<'a> {
    pub fn new(catalog: &'a BlockCatalog) -> Self {
        Self {
            catalog,
            full: catalog
                .defs()
                .iter()
                .map(|d| d.full_cube && d.model == u32::MAX)
                .collect(),
            opaque: catalog.defs().iter().map(|d| d.opaque).collect(),
        }
    }
    /// Whether `block` is meshed greedily (use as the complement filter for
    /// the culled extractor).
    pub fn handles(&self, block: u16) -> bool {
        block != AIR && self.full.get(block as usize).copied().unwrap_or(false)
    }
}

impl GreedyMaterials for CatalogGreedy<'_> {
    fn face_material(&self, block: u16, orientation: Orientation, face: Face) -> Option<u32> {
        if !self.handles(block) {
            return None;
        }
        let def = self.catalog.get(block)?;
        Some(def.materials[orientation.local_face(face).index()])
    }
    fn hides(&self, block: u16, neighbour: u16) -> bool {
        if neighbour == AIR {
            return false;
        }
        let opaque = self
            .opaque
            .get(neighbour as usize)
            .copied()
            .unwrap_or(false);
        opaque || (block == neighbour && self.handles(neighbour))
    }
    fn layer(&self, block: u16) -> Layer {
        self.catalog.get(block).map_or(Layer::Opaque, |d| d.layer)
    }
}

/// Greedy extraction options.
#[derive(Clone, Copy, Debug)]
pub struct GreedyOptions {
    /// Vertex ambient occlusion (merges only faces with equal AO).
    pub ao: bool,
    /// Sample light per vertex (merges only faces with equal light).
    pub light: bool,
}

impl Default for GreedyOptions {
    fn default() -> Self {
        Self {
            ao: true,
            light: false,
        }
    }
}

/// Reusable greedy mesher.
#[derive(Debug, Default)]
pub struct GreedyExtractor {
    pub options: GreedyOptions,
    keys: Vec<u64>,
    /// Bit `u` of row `(slice, v)` marks a pending key.
    row_bits: Vec<u64>,
    present: [Vec<u64>; 3],
    opaque_bits: [Vec<u64>; 3],
    row_present: Vec<u64>,
    row_opaque: Vec<u64>,
    /// Per block ID: 0 unknown, 1 greedy, 2 not greedy.
    known: Vec<u8>,
    /// Per block ID hot-loop flags: bit 7 learned, bit 0 greedy, bit 1 opaque.
    /// Fixed size so `u16` indexing needs no bounds check.
    flags: Option<Box<[u8; 65536]>>,
    materials: Vec<[u32; 6]>,
    layers: Vec<u64>,
}

/// Packed per-face key: material + 1 (20 bits) | layer (4) | AO (8) | light (32).
#[inline]
fn key(material: u32, layer: u64, ao: [u8; 4], light: [u8; 4]) -> u64 {
    let ao_bits =
        u64::from(ao[0]) | u64::from(ao[1]) << 2 | u64::from(ao[2]) << 4 | u64::from(ao[3]) << 6;
    let light_bits = u64::from(u32::from_le_bytes(light));
    (u64::from(material.min(GREEDY_MAX_MATERIAL)) + 1)
        | (layer & 15) << 20
        | ao_bits << 24
        | light_bits << 32
}

/// Largest material ID greedy keys can carry.
pub const GREEDY_MAX_MATERIAL: u32 = 0xf_fffe;

/// Transposes a 64×64 bit matrix: bit `c` of row `r` moves to bit `r` of
/// row `c` (Hacker's Delight, recursive block swaps).
pub(crate) fn transpose64(a: &mut [u64; 64]) {
    let mut j = 32usize;
    let mut m: u64 = 0x0000_0000_FFFF_FFFF;
    while j != 0 {
        let mut k = 0usize;
        while k < 64 {
            let t = ((a[k] >> j) ^ a[k + j]) & m;
            a[k] ^= t << j;
            a[k + j] ^= t;
            k = (k + j + 1) & !j;
        }
        j >>= 1;
        m ^= m << j;
    }
}

fn layer_bits(layer: Layer) -> u64 {
    match layer {
        Layer::Opaque => 0,
        Layer::Cutout => 1,
        Layer::Translucent => 2,
        Layer::Custom(c) => 3 + u64::from(c),
    }
}

impl GreedyExtractor {
    pub fn new(options: GreedyOptions) -> Self {
        Self {
            options,
            ..Self::default()
        }
    }

    /// Caches identity-orientation materials and layers for a block ID.
    #[inline]
    fn learn<M: GreedyMaterials>(&mut self, block: u16, materials: &M) -> bool {
        let b = block as usize;
        if b >= self.known.len() {
            self.known.resize(b + 1, 0);
            self.materials.resize(b + 1, [0; 6]);
            self.layers.resize(b + 1, 0);
        }
        if self.known[b] == 0 {
            let faces: [Option<u32>; 6] = std::array::from_fn(|f| {
                materials.face_material(block, Orientation::IDENTITY, Face::from_index(f).unwrap())
            });
            if block != AIR && faces.iter().all(Option::is_some) {
                self.known[b] = 1;
                self.materials[b] = faces.map(Option::unwrap);
                self.layers[b] = layer_bits(materials.layer(block));
            } else {
                self.known[b] = 2;
            }
        }
        self.known[b] == 1
    }

    /// Meshes greedy cubes from padded snapshots (`pad >= 1`, edge ≤ 62).
    /// `opaque` (per block ID) drives AO.
    #[allow(clippy::too_many_arguments)]
    pub fn extract<M: GreedyMaterials>(
        &mut self,
        blocks: &Snapshot<u16>,
        orientation: Option<&Snapshot<u8>>,
        block_light: Option<&Snapshot<u8>>,
        sky_light: Option<&Snapshot<u8>>,
        opaque: &[bool],
        materials: &M,
        sink: &mut dyn MeshSink,
    ) -> usize {
        let pad = blocks.pad() as i32;
        assert!(pad >= 1, "greedy extraction needs a padded snapshot");
        let n = blocks.edge() as i32 - 2 * pad;
        assert!(n + 2 <= 64, "greedy columns hold at most 62 cells");
        // Learned caches are per catalog; reset them for every call so a new
        // catalog never sees stale entries (learning is cheap).
        self.known.clear();
        let nu = n as usize;
        let e = blocks.edge() as i32;
        let data = blocks.data();
        // Flat index of chunk-local cell (snapshot order: x, z, y).
        let base = |x: i32, y: i32, z: i32| ((x + pad) + e * ((z + pad) + e * (y + pad))) as usize;
        let stride = [1usize, (e * e) as usize, e as usize];
        // One scan: uniform snapshots have no visible greedy faces (air
        // emits nothing; a greedy cube hides itself).
        let (lo, hi) = data
            .iter()
            .fold((u16::MAX, 0u16), |(lo, hi), b| (lo.min(*b), hi.max(*b)));
        if lo == hi {
            return 0;
        }
        let max_id = hi as usize;
        if self.known.len() <= max_id {
            self.known.resize(max_id + 1, 0);
            self.materials.resize(max_id + 1, [0; 6]);
            self.layers.resize(max_id + 1, 0);
        }
        let mut flags = self
            .flags
            .take()
            .unwrap_or_else(|| vec![0u8; 65536].into_boxed_slice().try_into().unwrap());
        flags[..=max_id].fill(0);
        let light_at = |i: usize| -> (u8, u8) {
            (
                block_light.map_or(0, |s| s.data()[i]),
                sky_light.map_or(0, |s| s.data()[i]),
            )
        };
        let mut emitted = 0;
        if self.keys.len() != nu * nu * nu {
            self.keys.clear();
            self.keys.resize(nu * nu * nu, 0);
        }
        self.row_bits.clear();
        self.row_bits.resize(nu * nu, 0);
        // One sequential pass: rows of bits along X for every (y, z), bit
        // `x + 1` for cell `x` in `-1..=n`.
        let span = (n + 2) as usize;
        self.row_present.clear();
        self.row_present.resize(span * span, 0);
        self.row_opaque.clear();
        self.row_opaque.resize(span * span, 0);
        for y in -1..=n {
            for z in -1..=n {
                let start = base(-1, y, z);
                let mut present = 0u64;
                let mut opaque_row = 0u64;
                let cells = &data[start..start + span];
                // Uniform rows (most rows of real terrain) skip the per-cell
                // bit chain; the comparison vectorises.
                let first = cells[0];
                if cells.iter().fold(0u16, |acc, &c| acc | (c ^ first)) == 0
                    && flags[first as usize] != 0
                {
                    let f = flags[first as usize];
                    let all = (1u64 << span) - 1;
                    let r = (y + 1) as usize * span + (z + 1) as usize;
                    self.row_present[r] = if f & 1 != 0 { all } else { 0 };
                    self.row_opaque[r] = if f & 2 != 0 { all } else { 0 };
                    continue;
                }
                for (j, &b) in cells.iter().enumerate() {
                    let mut f = flags[b as usize];
                    if f == 0 {
                        let greedy = self.learn(b, materials);
                        let opaque = opaque.get(b as usize).copied().unwrap_or(false);
                        f = 0x80 | u8::from(greedy) | u8::from(opaque) << 1;
                        flags[b as usize] = f;
                    }
                    present |= u64::from(f & 1) << j;
                    opaque_row |= u64::from((f >> 1) & 1) << j;
                }
                let r = (y + 1) as usize * span + (z + 1) as usize;
                self.row_present[r] = present;
                self.row_opaque[r] = opaque_row;
            }
        }
        for a in 0..3 {
            self.present[a].clear();
            self.present[a].resize(nu * nu, 0);
            self.opaque_bits[a].clear();
            self.opaque_bits[a].resize(nu * nu, 0);
        }
        // X columns are the rows themselves: column (u = y, v = z).
        for z in 0..n {
            for y in 0..n {
                let r = (y + 1) as usize * span + (z + 1) as usize;
                let column = (z * n + y) as usize;
                self.present[0][column] = self.row_present[r];
                self.opaque_bits[0][column] = self.row_opaque[r];
            }
        }
        // Y columns (u = x, v = z): per z, transpose the y-by-x bit matrix.
        // Z columns (u = x, v = y): per y, transpose the z-by-x bit matrix.
        let mut matrix = [0u64; 64];
        for opaque in [false, true] {
            let (source, targets) = if opaque {
                (&self.row_opaque, &mut self.opaque_bits)
            } else {
                (&self.row_present, &mut self.present)
            };
            for z in 1..=nu {
                matrix.fill(0);
                for (y, row) in matrix.iter_mut().enumerate().take(span) {
                    *row = source[y * span + z];
                }
                transpose64(&mut matrix);
                targets[1][(z - 1) * nu..z * nu].copy_from_slice(&matrix[1..=nu]);
            }
            for y in 1..=nu {
                matrix.fill(0);
                matrix[..span].copy_from_slice(&source[y * span..y * span + span]);
                transpose64(&mut matrix);
                targets[2][(y - 1) * nu..y * nu].copy_from_slice(&matrix[1..=nu]);
            }
        }
        for axis in 0..3 {
            let (ua, va) = tangents(axis);
            let step = stride[axis];
            let interior = ((1u64 << n) - 1) << 1;
            let du = stride[ua] as isize;
            let dv = stride[va] as isize;
            for positive in [false, true] {
                let face = Face::from_index(axis * 2 + usize::from(positive)).unwrap();
                let normal_step = if positive {
                    step as isize
                } else {
                    -(step as isize)
                };
                let mut any = false;
                for v in 0..n {
                    for u in 0..n {
                        let column = (v * n + u) as usize;
                        let present = self.present[axis][column] & interior;
                        let blocked = if positive {
                            self.opaque_bits[axis][column] >> 1
                        } else {
                            self.opaque_bits[axis][column] << 1
                        };
                        let mut visible = present & !blocked;
                        while visible != 0 {
                            let j = visible.trailing_zeros() as i32;
                            visible &= visible - 1;
                            let slice = j - 1;
                            let mut c = [0i32; 3];
                            c[axis] = slice;
                            c[ua] = u;
                            c[va] = v;
                            let i = base(c[0], c[1], c[2]);
                            let block = data[i];
                            let front = (i as isize + normal_step) as usize;
                            if materials.hides(block, data[front]) {
                                continue;
                            }
                            let material = match orientation {
                                Some(o) => match materials.face_material(
                                    block,
                                    Orientation(o.data()[i]),
                                    face,
                                ) {
                                    Some(m) => m,
                                    None => continue,
                                },
                                None => self.materials[block as usize][face.index()],
                            };
                            let mut ao = [3u8; 4];
                            let mut light = [0u8; 4];
                            if self.options.ao || self.options.light {
                                // Corners in (u, v) order: 00, 10, 11, 01.
                                for (corner, (su, sv)) in
                                    [(-1isize, -1isize), (1, -1), (1, 1), (-1, 1)]
                                        .into_iter()
                                        .enumerate()
                                {
                                    let i1 = (front as isize + su * du) as usize;
                                    let i2 = (front as isize + sv * dv) as usize;
                                    let i3 = (front as isize + su * du + sv * dv) as usize;
                                    let solid = |i: usize| flags[data[i] as usize] & 2 != 0;
                                    let (s1, s2, c3) = (solid(i1), solid(i2), solid(i3));
                                    if self.options.ao {
                                        ao[corner] = if s1 && s2 {
                                            0
                                        } else {
                                            3 - (u8::from(s1) + u8::from(s2) + u8::from(c3))
                                        };
                                    }
                                    if self.options.light {
                                        let mut sum = (0u32, 0u32, 0u32);
                                        for (idx, blocked) in [
                                            (front, false),
                                            (i1, s1),
                                            (i2, s2),
                                            (i3, c3 || (s1 && s2)),
                                        ] {
                                            if !blocked {
                                                let (b, s) = light_at(idx);
                                                sum = (
                                                    sum.0 + u32::from(b),
                                                    sum.1 + u32::from(s),
                                                    sum.2 + 1,
                                                );
                                            }
                                        }
                                        let count = sum.2.max(1);
                                        light[corner] = ((sum.0 / count) as u8 & 15)
                                            | (((sum.1 / count) as u8 & 15) << 4);
                                    }
                                }
                            }
                            let row = slice as usize * nu + v as usize;
                            self.keys[row * nu + u as usize] =
                                key(material, self.layers[block as usize], ao, light);
                            self.row_bits[row] |= 1 << u;
                            any = true;
                        }
                    }
                }
                if !any {
                    continue;
                }
                // Merge rectangles slice by slice over pending bits only;
                // merging clears every bit, so rows are clean for the next face.
                for slice in 0..nu {
                    for v in 0..nu {
                        let row = slice * nu + v;
                        while self.row_bits[row] != 0 {
                            let u = self.row_bits[row].trailing_zeros() as usize;
                            let k = self.keys[row * nu + u];
                            let mut w = 1;
                            while u + w < nu
                                && self.row_bits[row] & (1 << (u + w)) != 0
                                && self.keys[row * nu + u + w] == k
                            {
                                w += 1;
                            }
                            let span = ((1u64 << w) - 1) << u;
                            let mut h = 1;
                            'grow: while v + h < nu {
                                let next = row + h;
                                if self.row_bits[next] & span != span {
                                    break;
                                }
                                for x in u..u + w {
                                    if self.keys[next * nu + x] != k {
                                        break 'grow;
                                    }
                                }
                                h += 1;
                            }
                            for dv in 0..h {
                                self.row_bits[row + dv] &= !span;
                            }
                            let mut c = [0i32; 3];
                            c[axis] = slice as i32;
                            c[ua] = u as i32;
                            c[va] = v as i32;
                            let block = data[base(c[0], c[1], c[2])];
                            emitted += 1;
                            sink.quad(
                                materials.layer(block),
                                &build_quad(
                                    face,
                                    slice as i32,
                                    u as i32,
                                    v as i32,
                                    w as i32,
                                    h as i32,
                                    k,
                                ),
                            );
                        }
                    }
                }
            }
        }
        self.flags = Some(flags);
        emitted
    }
}

fn build_quad(face: Face, slice: i32, u: i32, v: i32, w: i32, h: i32, k: u64) -> Quad {
    let axis = face.axis();
    let (ua, va) = tangents(axis);
    let plane = slice as f32 + if face.is_positive() { 1.0 } else { 0.0 };
    let corner = |cu: i32, cv: i32| {
        let mut p = Vec3::ZERO;
        p[axis] = plane;
        p[ua] = cu as f32;
        p[va] = cv as f32;
        p
    };
    let mut positions = [
        corner(u, v),
        corner(u + w, v),
        corner(u + w, v + h),
        corner(u, v + h),
    ];
    let material = ((k & 0xf_ffff) - 1) as u32;
    let mut ao = [0u8; 4];
    for (i, a) in ao.iter_mut().enumerate() {
        *a = ((k >> (24 + i * 2)) & 3) as u8;
    }
    let mut light = ((k >> 32) as u32).to_le_bytes();
    let normal = face.normal().as_vec3();
    if (positions[1] - positions[0])
        .cross(positions[3] - positions[0])
        .dot(normal)
        < 0.0
    {
        positions.swap(1, 3);
        ao.swap(1, 3);
        light.swap(1, 3);
    }
    let flipped = u16::from(ao[0]) + u16::from(ao[2]) < u16::from(ao[1]) + u16::from(ao[3]);
    let uv = positions.map(|p| {
        crate::format::packed_uv(face, p[ua] - u as f32, p[va] - v as f32, w as f32, h as f32)
    });
    Quad {
        positions,
        normal,
        face: Some(face),
        uv,
        material,
        ao,
        light,
        tint: u32::MAX,
        flipped,
        size: [w as u8, h as u8],
    }
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::transpose64;

    #[test]
    fn transpose_moves_bit_c_of_row_r_to_bit_r_of_row_c() {
        let mut rng = 0x1234_5678_9abc_def0u64;
        let mut a = [0u64; 64];
        for row in a.iter_mut() {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            *row = rng;
        }
        let original = a;
        transpose64(&mut a);
        for r in 0..64 {
            for c in 0..64 {
                assert_eq!((original[r] >> c) & 1, (a[c] >> r) & 1, "r={r} c={c}");
            }
        }
    }
}
