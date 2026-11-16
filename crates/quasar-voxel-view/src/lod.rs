//! Level-of-detail selection and downsampling.
//!
//! Density maps use an octree/clipmap of nodes with a fixed cell count per
//! node (`CELLS³`, 32 by default) whose voxel size doubles per level, so a
//! node at level `l` covers `CELLS · 2^l` voxels. [`LodSelector`] picks leaf
//! nodes around a camera with hysteresis and reports which faces border a
//! different level (for seam skirts).
use quasar_math::{IVec3, Vec3};
use quasar_voxel::{ChannelId, ChunkLayout, Face, Raw, Snapshot, Value, Volume};
use std::collections::{BTreeMap, BTreeSet};

/// One LOD node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LodNode {
    pub level: u32,
    /// Node coordinate at its level (voxel origin = coord · cells · 2^level).
    pub coord: IVec3,
}

impl Ord for LodNode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.level, self.coord.x, self.coord.y, self.coord.z).cmp(&(
            other.level,
            other.coord.x,
            other.coord.y,
            other.coord.z,
        ))
    }
}
impl PartialOrd for LodNode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl LodNode {
    pub fn size(self, cells: u32) -> i32 {
        (cells << self.level) as i32
    }
    pub fn origin(self, cells: u32) -> IVec3 {
        self.coord * self.size(cells)
    }
    pub fn parent(self) -> Self {
        Self {
            level: self.level + 1,
            coord: IVec3::new(
                self.coord.x.div_euclid(2),
                self.coord.y.div_euclid(2),
                self.coord.z.div_euclid(2),
            ),
        }
    }
    pub fn children(self) -> [Self; 8] {
        std::array::from_fn(|i| Self {
            level: self.level - 1,
            coord: self.coord * 2
                + IVec3::new((i & 1) as i32, ((i >> 1) & 1) as i32, ((i >> 2) & 1) as i32),
        })
    }
    pub fn contains_voxel(self, cells: u32, voxel: IVec3) -> bool {
        let lo = self.origin(cells);
        let hi = lo + IVec3::splat(self.size(cells));
        voxel.cmpge(lo).all() && voxel.cmplt(hi).all()
    }
}

/// Selection parameters.
#[derive(Clone, Copy, Debug)]
pub struct LodConfig {
    /// Cells per node edge.
    pub cells: u32,
    /// Coarsest level (root nodes).
    pub max_level: u32,
    /// Split a node while `distance < split * node size`.
    pub split: f32,
    /// Hysteresis fraction (0.1 = merge only beyond 110% of the split distance).
    pub hysteresis: f32,
}

impl Default for LodConfig {
    fn default() -> Self {
        Self {
            cells: 32,
            max_level: 6,
            split: 1.5,
            hysteresis: 0.15,
        }
    }
}

/// A selected node and the faces where it borders a coarser or finer node.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectedNode {
    pub node: LodNode,
    pub seam_faces: u8,
}

/// Chooses leaf nodes covering a set of root nodes around a camera.
#[derive(Clone, Debug, Default)]
pub struct LodSelector {
    pub config: LodConfig,
    split_last: BTreeSet<LodNode>,
}

impl LodSelector {
    pub fn new(config: LodConfig) -> Self {
        Self {
            config,
            split_last: BTreeSet::new(),
        }
    }
    fn distance(&self, node: LodNode, camera: Vec3) -> f32 {
        let lo = node.origin(self.config.cells).as_vec3();
        let hi = lo + Vec3::splat(node.size(self.config.cells) as f32);
        (camera.clamp(lo, hi) - camera).length()
    }
    /// Leaf nodes in sorted order. `roots` are nodes at `max_level` (e.g. the
    /// bounds of an asteroid); `camera` is in grid voxel space.
    pub fn select(&mut self, roots: &[IVec3], camera: Vec3) -> Vec<SelectedNode> {
        let mut leaves = BTreeSet::new();
        let mut split = BTreeSet::new();
        let mut stack: Vec<LodNode> = roots
            .iter()
            .map(|c| LodNode {
                level: self.config.max_level,
                coord: *c,
            })
            .collect();
        while let Some(node) = stack.pop() {
            if node.level == 0 {
                leaves.insert(node);
                continue;
            }
            let size = node.size(self.config.cells) as f32;
            let d = self.distance(node, camera);
            let was_split = self.split_last.contains(&node);
            // Two-sided band: split nodes stay split until well past the
            // threshold; unsplit nodes split only well inside it.
            let band = if was_split {
                1.0 + self.config.hysteresis
            } else {
                1.0 - self.config.hysteresis
            };
            let threshold = self.config.split * size * band;
            if d < threshold {
                split.insert(node);
                stack.extend(node.children());
            } else {
                leaves.insert(node);
            }
        }
        self.split_last = split;
        let level_of = Self::level_map(&leaves);
        leaves
            .iter()
            .map(|node| SelectedNode {
                node: *node,
                seam_faces: Self::seams(*node, &level_of, self.config.cells),
            })
            .collect()
    }
    fn level_map(leaves: &BTreeSet<LodNode>) -> BTreeMap<(i32, i32, i32, u32), ()> {
        leaves
            .iter()
            .map(|n| ((n.coord.x, n.coord.y, n.coord.z, n.level), ()))
            .collect()
    }
    fn seams(node: LodNode, leaves: &BTreeMap<(i32, i32, i32, u32), ()>, cells: u32) -> u8 {
        let mut mask = 0;
        let size = node.size(cells);
        for face in Face::ALL {
            // A probe voxel just outside the face, at the face centre.
            let centre = node.origin(cells) + IVec3::splat(size / 2);
            let mut probe = centre;
            probe[face.axis()] = if face.is_positive() {
                node.origin(cells)[face.axis()] + size
            } else {
                node.origin(cells)[face.axis()] - 1
            };
            let same = leaves.contains_key(&{
                let n = node.coord + face.normal();
                (n.x, n.y, n.z, node.level)
            });
            if same {
                continue;
            }
            // Any leaf covering the probe at another level makes a seam.
            let covered = (0..=16u32).any(|level| {
                let s = (cells << level) as i32;
                let c = IVec3::new(
                    probe.x.div_euclid(s),
                    probe.y.div_euclid(s),
                    probe.z.div_euclid(s),
                );
                level != node.level && leaves.contains_key(&(c.x, c.y, c.z, level))
            });
            if covered {
                mask |= face.bit();
            }
        }
        mask
    }
}

/// Fills a padded density snapshot for `node` by sampling every `2^level`
/// voxels of `density` (and optionally `material`). Edge = cells + 2·pad.
#[allow(clippy::too_many_arguments)]
pub fn sample_lod<L: ChunkLayout, M: Value>(
    volume: &Volume<L>,
    density: ChannelId<i8>,
    material: Option<ChannelId<M>>,
    node: LodNode,
    cells: u32,
    pad: u32,
    out_density: &mut Snapshot<u8>,
    out_material: Option<&mut Snapshot<M::Raw>>,
) {
    let stride = 1i32 << node.level;
    let origin = node.origin(cells);
    let p = pad as i32;
    let edge = cells as usize + 2 * pad as usize;
    let mut cursor = quasar_voxel::Cursor::default();
    let mut values = Vec::with_capacity(edge * edge * edge);
    let mut mats = Vec::with_capacity(if material.is_some() {
        edge * edge * edge
    } else {
        0
    });
    for y in -p..cells as i32 + p {
        for z in -p..cells as i32 + p {
            for x in -p..cells as i32 + p {
                let voxel = origin + IVec3::new(x, y, z) * stride;
                values.push(volume.get_with(&mut cursor, density, voxel) as u8);
                if let Some(m) = material {
                    mats.push(volume.get(m, voxel).to_raw());
                }
            }
        }
    }
    out_density.fill_with(edge, pad, node.coord, values);
    if let (Some(out), Some(_)) = (out_material, material) {
        out.fill_with(edge, pad, node.coord, mats);
    }
}

/// Majority-material downsample of a cubic volume for far LOD: each coarse
/// cell takes the most common non-`empty` value of its `2^level`³ block
/// (ties by lower value), or `empty` when fewer than half are filled.
#[allow(clippy::too_many_arguments)]
pub fn downsample_majority<L: ChunkLayout, T: Value>(
    volume: &Volume<L>,
    channel: ChannelId<T>,
    origin: IVec3,
    level: u32,
    cells: u32,
    pad: u32,
    empty: T,
    out: &mut Snapshot<T::Raw>,
) {
    let stride = 1i32 << level;
    let p = pad as i32;
    let edge = cells as usize + 2 * pad as usize;
    let mut values = Vec::with_capacity(edge * edge * edge);
    let mut counts: BTreeMap<u32, u32> = BTreeMap::new();
    let mut cursor = quasar_voxel::Cursor::default();
    let total = (stride * stride * stride) as u32;
    for y in -p..cells as i32 + p {
        for z in -p..cells as i32 + p {
            for x in -p..cells as i32 + p {
                counts.clear();
                let base = origin + IVec3::new(x, y, z) * stride;
                let mut filled = 0;
                for dy in 0..stride {
                    for dz in 0..stride {
                        for dx in 0..stride {
                            let v = volume.get_with(
                                &mut cursor,
                                channel,
                                base + IVec3::new(dx, dy, dz),
                            );
                            if v != empty {
                                filled += 1;
                                *counts.entry(v.to_raw().to_u32()).or_insert(0) += 1;
                            }
                        }
                    }
                }
                let value = if filled * 2 >= total {
                    counts
                        .iter()
                        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
                        .map_or(empty.to_raw(), |(v, _)| <T::Raw as Raw>::from_u32(*v))
                } else {
                    empty.to_raw()
                };
                values.push(value);
            }
        }
    }
    out.fill_with(edge, pad, origin, values);
}
