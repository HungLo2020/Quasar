//! Neighbourhood-aware bevelled blocks: chamfered edges and chipped corners
//! that follow which faces are open.
//!
//! [`BevelExtractor`] meshes full-cube blocks so exposed edges are
//! chamfered and exposed corners cut by a facet, for hand-cut stone, worn
//! rock and rounded toy bricks, while blocks stay true cubes on the grid
//! (collision, picking and editing are unchanged). Cuts depend on the
//! neighbourhood:
//!
//! - Only edges and corners where every adjoining face is open are cut, and
//!   faces between neighbours are never emitted.
//! - Each lattice point's cut depth comes from a seeded hash of its world
//!   position and the largest bevel scale among the blocks touching it, so
//!   neighbouring blocks cut a shared corner identically and their chamfers
//!   meet without gaps.
//! - A corner is kept sharp ("pinned") when cutting it would expose a face
//!   of the neighbour across a closed side that is not cut the same way.
//!
//! The per-block policy returns `None` to skip a block (mesh it with another
//! extractor, for example [`crate::CulledModelExtractor`] for models),
//! `Some(0.0)` for a flat cube (ground, liquids) or `Some(scale)` for a
//! bevelled one; larger scales cut deeper (mining wear).
use crate::ExtractStats;
use crate::culled::cube_face;
use crate::quad::{MeshSink, Quad};
use quasar_math::{IVec3, Vec2, Vec3};
use quasar_voxel::{BlockCatalog, Face, Snapshot};

/// Cut sizes and variation for [`BevelExtractor`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BevelOptions {
    /// Typical cut depth (fraction of a block) at bevel scale 1.
    pub radius: f32,
    /// How much cuts vary per corner, 0 (uniform) to 1.
    pub variation: f32,
    /// Fraction of corners knocked off hard.
    pub chip_chance: f32,
    /// Extra depth of a hard chip.
    pub chip_depth: f32,
    /// Upper bound on any cut (below 0.5 so faces never invert).
    pub max_cut: f32,
    /// Per-block brightness variation (0 = none), applied as a vertex tint.
    pub tint_variation: f32,
    /// Vertex ambient occlusion on faces.
    pub ao: bool,
    pub seed: u32,
}

impl Default for BevelOptions {
    fn default() -> Self {
        Self {
            radius: 0.12,
            variation: 0.6,
            chip_chance: 0.12,
            chip_depth: 0.18,
            max_cut: 0.46,
            tint_variation: 0.15,
            ao: true,
            seed: 0,
        }
    }
}

/// Blocks to mesh: a snapshot padded by at least one voxel (as for culled
/// extraction), the per-block opacity table (which neighbours close a face
/// and shade AO) and optional block and sky light. Each face takes the
/// light of the cell in front of it; chamfers and corner facets the
/// brightest of their open sides.
#[derive(Clone, Copy)]
pub struct BevelInput<'a> {
    pub blocks: &'a Snapshot<u16>,
    pub opaque: &'a [bool],
    pub block_light: Option<&'a Snapshot<u8>>,
    pub sky_light: Option<&'a Snapshot<u8>>,
}

impl<'a> BevelInput<'a> {
    pub fn new(blocks: &'a Snapshot<u16>, opaque: &'a [bool]) -> Self {
        Self {
            blocks,
            opaque,
            block_light: None,
            sky_light: None,
        }
    }
    /// Packed light (block in the low nibble, sky in the high) at `p`.
    fn light(&self, p: IVec3) -> u8 {
        let block = self.block_light.map_or(0, |s| s.at(p.x, p.y, p.z) & 15);
        let sky = self.sky_light.map_or(15, |s| s.at(p.x, p.y, p.z) & 15);
        block | sky << 4
    }
}

/// The brighter of two packed lights, per nibble.
fn brighter(a: u8, b: u8) -> u8 {
    (a & 15).max(b & 15) | (a >> 4).max(b >> 4) << 4
}

/// See the module docs. Reusable.
#[derive(Clone, Debug, Default)]
pub struct BevelExtractor {
    pub options: BevelOptions,
}

#[inline]
fn hash(p: IVec3, seed: u32, salt: u32) -> f32 {
    let h = quasar_core::noise::hash3(seed ^ salt.wrapping_mul(0x9e37_79b9), p.x, p.y, p.z);
    quasar_core::noise::unit_f32(h)
}

/// The face on `axis` at corner `c` (positive when `c[axis]` is 1).
fn side(axis: usize, c: IVec3) -> Face {
    match (axis, c[axis] == 1) {
        (0, false) => Face::NegX,
        (0, true) => Face::PosX,
        (1, false) => Face::NegY,
        (1, true) => Face::PosY,
        (2, false) => Face::NegZ,
        _ => Face::PosZ,
    }
}

fn corner(k: i32) -> IVec3 {
    IVec3::new(k & 1, k >> 1 & 1, k >> 2 & 1)
}

fn corner_bit(c: IVec3) -> u8 {
    1 << (c.x | c.y << 1 | c.z << 2)
}

impl BevelExtractor {
    pub fn new(options: BevelOptions) -> Self {
        Self { options }
    }

    /// Meshes every block for which `bevel(block)` is `Some`.
    pub fn extract(
        &mut self,
        input: BevelInput<'_>,
        catalog: &BlockCatalog,
        bevel: impl Fn(u16) -> Option<f32>,
        sink: &mut dyn MeshSink,
    ) -> ExtractStats {
        let blocks = input.blocks;
        let pad = blocks.pad() as i32;
        assert!(pad >= 1, "bevel extraction needs a padded snapshot");
        let n = blocks.edge() as i32 - 2 * pad;
        let origin = blocks.coord() * n;
        let closes = |p: IVec3| {
            let b = blocks.at(p.x, p.y, p.z);
            input.opaque.get(b as usize).copied().unwrap_or(false)
        };
        let scale_of = |p: IVec3| bevel(blocks.at(p.x, p.y, p.z)).unwrap_or(0.0).max(0.0);
        let mut stats = ExtractStats::default();
        for y in 0..n {
            for z in 0..n {
                for x in 0..n {
                    let cell = IVec3::new(x, y, z);
                    let block = blocks.at(x, y, z);
                    let Some(scale) = bevel(block) else {
                        continue;
                    };
                    let Some(def) = catalog.get(block) else {
                        continue;
                    };
                    if !def.full_cube {
                        continue;
                    }
                    stats.cells += 1;
                    let mut open = [false; 6];
                    for face in Face::ALL {
                        open[face.index()] = !closes(cell + face.normal());
                    }
                    if !open.contains(&true) {
                        continue;
                    }
                    let scale = scale.max(0.0);
                    // Corners whose cut would open a hole into a neighbour.
                    let mut pinned = 0u8;
                    if scale > 0.0 {
                        for k in 0..8 {
                            let c = corner(k);
                            let faces = [side(0, c), side(1, c), side(2, c)];
                            let mut closed = faces.into_iter().filter(|f| !open[f.index()]);
                            let (Some(only), None) = (closed.next(), closed.next()) else {
                                continue;
                            };
                            let across = cell + only.normal();
                            let matches = scale_of(across) > 0.0
                                && faces
                                    .into_iter()
                                    .filter(|f| *f != only)
                                    .all(|f| !closes(across + f.normal()));
                            if !matches {
                                pinned |= corner_bit(c);
                            }
                        }
                    }
                    let world = origin + cell;
                    let cuts: [f32; 8] = std::array::from_fn(|k| {
                        let c = corner(k as i32);
                        if scale <= 0.0 || pinned & corner_bit(c) != 0 {
                            return 0.0;
                        }
                        // The largest scale around the lattice point, so
                        // every block sharing it cuts it the same.
                        let lattice = cell + c;
                        let mut around = 0.0f32;
                        for o in 0..8 {
                            around = around.max(scale_of(lattice - corner(o)));
                        }
                        self.cut(world + c, around)
                    });
                    let shape = Shape {
                        open,
                        cuts,
                        bevelled: scale > 0.0,
                    };
                    let tint = self.tint(world);
                    self.emit(&shape, cell, def, tint, blocks, &input, sink, &mut stats);
                }
            }
        }
        stats
    }

    /// Cut depth at a lattice point.
    fn cut(&self, lattice: IVec3, scale: f32) -> f32 {
        let o = &self.options;
        let n = hash(lattice, o.seed, 17);
        let mut depth = o.radius * (1.0 + o.variation * (n * 2.0 - 1.0));
        if hash(lattice, o.seed, 29) < o.chip_chance {
            depth += o.chip_depth * (0.5 + 0.5 * hash(lattice, o.seed, 31));
        }
        (depth * scale).clamp(0.0, o.max_cut.min(0.49))
    }

    fn tint(&self, world: IVec3) -> u32 {
        if self.options.tint_variation <= 0.0 {
            return 0xffff_ffff;
        }
        let tone = 1.0 - self.options.tint_variation * hash(world, self.options.seed, 3);
        let g = (tone.clamp(0.0, 1.0) * 255.0).round() as u32;
        0xff00_0000 | g << 16 | g << 8 | g
    }

    #[allow(clippy::too_many_arguments)]
    fn emit(
        &self,
        shape: &Shape,
        cell: IVec3,
        def: &quasar_voxel::BlockDef,
        tint: u32,
        blocks: &Snapshot<u16>,
        input: &BevelInput<'_>,
        sink: &mut dyn MeshSink,
        stats: &mut ExtractStats,
    ) {
        let base = cell.as_vec3();
        let opaque = |p: IVec3| {
            let b = blocks.at(p.x, p.y, p.z);
            input.opaque.get(b as usize).copied().unwrap_or(false)
        };
        let mut push = |quad: Quad| {
            sink.quad(def.layer, &quad);
            stats.quads += 1;
        };
        let lit = |faces: &[Face]| {
            faces
                .iter()
                .map(|f| input.light(cell + f.normal()))
                .fold(0u8, brighter)
        };
        for face in Face::ALL {
            if !shape.is_open(face) {
                stats.culled += 1;
                continue;
            }
            let light = lit(&[face]);
            let corners = cube_face(face);
            let positions = corners.map(|c| shape.vertex(face, unit_corner(c)));
            // UVs follow the unit face, cropped by the cuts.
            let e_u = corners[1] - corners[0];
            let e_v = corners[3] - corners[0];
            let uv = positions.map(|p| {
                let d = p - corners[0];
                Vec2::new(d.dot(e_u), 1.0 - d.dot(e_v))
            });
            let ao = if self.options.ao {
                face_ao(face, cell, &corners, &opaque)
            } else {
                [3; 4]
            };
            let flush = positions
                .iter()
                .zip(corners.iter())
                .all(|(p, c)| (*p - *c).length_squared() < 1e-9);
            push(Quad {
                positions: positions.map(|p| base + p),
                normal: face.normal().as_vec3(),
                face: flush.then_some(face),
                uv,
                material: def.materials[face.index()],
                ao,
                light: [light; 4],
                tint,
                flipped: u16::from(ao[0]) + u16::from(ao[2]) < u16::from(ao[1]) + u16::from(ao[3]),
                size: [1, 1],
            });
        }
        if !shape.bevelled {
            return;
        }
        // A facet on each corner where all three faces are open.
        for k in 0..8 {
            let c = corner(k);
            let faces = [side(0, c), side(1, c), side(2, c)];
            if !faces.iter().all(|f| shape.is_open(*f)) || shape.cuts[k as usize] <= 0.0 {
                continue;
            }
            let points = faces.map(|f| shape.vertex(f, c));
            let outward = faces.iter().map(|f| f.normal().as_vec3()).sum::<Vec3>();
            if let Some(q) = facet(
                base,
                [points[0], points[1], points[2], points[2]],
                outward,
                lip_material(def, &faces),
                tint,
                lit(&faces),
            ) {
                push(q);
            }
        }
        // Chamfers along edges where both faces are open.
        for a in Face::ALL {
            for b in Face::ALL {
                if a.index() >= b.index() || a.axis() == b.axis() {
                    continue;
                }
                if !shape.is_open(a) || !shape.is_open(b) {
                    continue;
                }
                let third = 3 - a.axis() - b.axis();
                let mut ends = [IVec3::ZERO; 2];
                for (i, end) in ends.iter_mut().enumerate() {
                    end[a.axis()] = i32::from(a.is_positive());
                    end[b.axis()] = i32::from(b.is_positive());
                    end[third] = i as i32;
                }
                let quad = [
                    shape.vertex(a, ends[0]),
                    shape.vertex(a, ends[1]),
                    shape.vertex(b, ends[1]),
                    shape.vertex(b, ends[0]),
                ];
                let outward = (a.normal() + b.normal()).as_vec3();
                if let Some(q) = facet(
                    base,
                    quad,
                    outward,
                    lip_material(def, &[a, b]),
                    tint,
                    lit(&[a, b]),
                ) {
                    push(q);
                }
            }
        }
    }
}

/// Which faces are open and how deep each corner is cut.
struct Shape {
    open: [bool; 6],
    cuts: [f32; 8],
    bevelled: bool,
}

impl Shape {
    fn is_open(&self, face: Face) -> bool {
        self.open[face.index()]
    }
    /// Where face `face`'s vertex at corner `c` lands: pulled in along each
    /// other axis whose face at that corner is also open.
    fn vertex(&self, face: Face, c: IVec3) -> Vec3 {
        let mut p = c.as_vec3();
        let r = self.cuts[(c.x | c.y << 1 | c.z << 2) as usize];
        if r <= 0.0 {
            return p;
        }
        for axis in 0..3 {
            if axis != face.axis() && self.is_open(side(axis, c)) {
                p[axis] += if c[axis] == 1 { -r } else { r };
            }
        }
        p
    }
}

fn unit_corner(p: Vec3) -> IVec3 {
    IVec3::new(
        i32::from(p.x > 0.5),
        i32::from(p.y > 0.5),
        i32::from(p.z > 0.5),
    )
}

/// 4-level AO from the cells in front of the face.
fn face_ao(
    face: Face,
    cell: IVec3,
    corners: &[Vec3; 4],
    opaque: &impl Fn(IVec3) -> bool,
) -> [u8; 4] {
    let front = cell + face.normal();
    let (ua, va) = match face.axis() {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    corners.map(|c| {
        let mut su = IVec3::ZERO;
        su[ua] = if c[ua] > 0.5 { 1 } else { -1 };
        let mut sv = IVec3::ZERO;
        sv[va] = if c[va] > 0.5 { 1 } else { -1 };
        let (s1, s2) = (opaque(front + su), opaque(front + sv));
        if s1 && s2 {
            0
        } else {
            3 - (u8::from(s1) + u8::from(s2) + u8::from(opaque(front + su + sv)))
        }
    })
}

/// Chamfers use a side face's material (so a top "lip" texture band shows);
/// purely vertical edges use the side.
fn lip_material(def: &quasar_voxel::BlockDef, faces: &[Face]) -> u32 {
    let side = faces
        .iter()
        .find(|f| f.axis() != 1)
        .copied()
        .unwrap_or(Face::PosY);
    def.materials[side.index()]
}

/// A chamfer or corner facet, wound to face `outward`; `None` when
/// degenerate.
fn facet(
    base: Vec3,
    mut points: [Vec3; 4],
    outward: Vec3,
    material: u32,
    tint: u32,
    light: u8,
) -> Option<Quad> {
    let mut normal = (points[2] - points[0])
        .cross(points[3] - points[1])
        .normalize_or_zero();
    if normal == Vec3::ZERO {
        normal = (points[1] - points[0])
            .cross(points[2] - points[0])
            .normalize_or_zero();
    }
    if normal == Vec3::ZERO || (points[0] - points[1]).length_squared() < 1e-8 {
        return None;
    }
    if normal.dot(outward) < 0.0 {
        points.reverse();
        normal = -normal;
    }
    // Map onto the upper band of the texture, along the edge.
    let along = (points[1] - points[0]).normalize_or_zero();
    let uv = points.map(|p| {
        let d = p - points[0];
        Vec2::new(
            d.dot(along).abs().fract(),
            (p.y - points[0].y).abs() * 0.5 + 0.02,
        )
    });
    Some(Quad {
        positions: points.map(|p| base + p),
        normal,
        face: None,
        uv,
        material,
        ao: [3; 4],
        light: [light; 4],
        tint,
        flipped: false,
        size: [1, 1],
    })
}
