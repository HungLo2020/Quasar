//! Surface Nets extraction for density channels, with seam skirts.
//!
//! Density samples sit at lattice points (voxel coordinates); positive is
//! solid. A chunk owns the sign-change edges whose base point lies in
//! `[0, edge)³`, so neighbouring chunks meet without gaps or duplicates.
//! Each surface cell gets one vertex at the mean of its edge crossings.
//!
//! LOD nodes border coarser or finer neighbours whose surfaces do not line up
//! exactly. [`SmoothOptions::seam_faces`] adds skirts: boundary edges on
//! those faces are extruded into the solid side so any crack between levels
//! shows skirt, never background.
use crate::quad::{MeshSink, SmoothVertex};
use quasar_math::{IVec3, Vec3};
use quasar_voxel::{Face, Layer, Raw, Snapshot};
use std::collections::HashMap;

/// Options for one smooth extraction.
#[derive(Clone, Copy, Debug)]
pub struct SmoothOptions {
    /// Output units per voxel (2^lod for LOD nodes).
    pub scale: f32,
    pub layer: Layer,
    /// Faces (bit per [`Face`]) that border a different LOD.
    pub seam_faces: u8,
    /// Skirt depth in output units (defaults to one cell when 0).
    pub skirt_depth: f32,
}

impl Default for SmoothOptions {
    fn default() -> Self {
        Self {
            scale: 1.0,
            layer: Layer::Opaque,
            seam_faces: 0,
            skirt_depth: 0.0,
        }
    }
}

/// Counts from one extraction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SmoothStats {
    pub vertices: usize,
    pub triangles: usize,
    pub skirt_triangles: usize,
}

/// Reusable Surface Nets extractor.
#[derive(Debug, Default)]
pub struct SurfaceNetsExtractor {
    values: Vec<f32>,
    rows: Vec<u64>,
    cell_vertex: Vec<u32>,
    vertices: Vec<SmoothVertex>,
    indices: Vec<u32>,
    cell_of_vertex: Vec<IVec3>,
}

const NONE: u32 = u32::MAX;
const CORNERS: [IVec3; 8] = [
    IVec3::new(0, 0, 0),
    IVec3::new(1, 0, 0),
    IVec3::new(0, 1, 0),
    IVec3::new(1, 1, 0),
    IVec3::new(0, 0, 1),
    IVec3::new(1, 0, 1),
    IVec3::new(0, 1, 1),
    IVec3::new(1, 1, 1),
];
const EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

impl SurfaceNetsExtractor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Extracts the surface of a padded density snapshot (`pad >= 1`; the
    /// `u8` values are `i8` densities) with optional per-point materials.
    pub fn extract<M: Raw>(
        &mut self,
        density: &Snapshot<u8>,
        materials: Option<&Snapshot<M>>,
        options: SmoothOptions,
        sink: &mut dyn MeshSink,
    ) -> SmoothStats {
        let pad = density.pad() as i32;
        assert!(pad >= 1, "smooth extraction needs a padded snapshot");
        let n = density.edge() as i32 - 2 * pad;
        let e = density.edge() as i32;
        // Fast path: no sign change anywhere.
        let first = density.data()[0] as i8 >= 0;
        if density.data().iter().all(|v| (*v as i8 >= 0) == first) {
            return SmoothStats::default();
        }
        // Half a quantisation step of bias: solid is still `v >= 0`, but no
        // sample is exactly zero, so crossings never sit on lattice points
        // (which would collapse neighbouring vertices into degenerate faces).
        self.values.clear();
        self.values
            .extend(density.data().iter().map(|v| f32::from(*v as i8) + 0.5));
        let values_all = &self.values;
        // Flat index of chunk-local point p (snapshot order: x, z, y).
        let index = |p: IVec3| ((p.x + pad) + e * ((p.z + pad) + e * (p.y + pad))) as usize;
        let offsets: [usize; 8] = CORNERS.map(|c| (c.x + e * (c.z + e * c.y)) as usize);
        let mut edge_count: HashMap<(u32, u32), u32> = HashMap::new();
        let track_edges = options.seam_faces != 0;
        // Cells -1..n-1 (n + 1 per axis).
        let cells = (n + 1) as usize;
        self.cell_vertex.clear();
        self.cell_vertex.resize(cells * cells * cells, NONE);
        self.vertices.clear();
        self.indices.clear();
        self.cell_of_vertex.clear();
        let cell_index = |c: IVec3| {
            (((c.y + 1) as usize * cells + (c.z + 1) as usize) * cells) + (c.x + 1) as usize
        };
        // Sign bits per lattice row: bit j = point x = j - 1 is solid.
        assert!(
            n + 2 <= 64,
            "smooth extraction supports chunks up to 62 cells"
        );
        let span = (n + 2) as usize;
        self.rows.clear();
        for y in -1..=n {
            for z in -1..=n {
                let start = index(IVec3::new(-1, y, z));
                let mut bits = 0u64;
                for j in 0..span {
                    bits |= u64::from(values_all[start + j] > 0.0) << j;
                }
                self.rows.push(bits);
            }
        }
        let row_bits = |y: i32, z: i32| self.rows[(y + 1) as usize * span + (z + 1) as usize];
        let cells_mask = (1u64 << (n + 1)) - 1;
        for cy in -1..n {
            for cz in -1..n {
                let (a, b, c, d4) = (
                    row_bits(cy, cz),
                    row_bits(cy + 1, cz),
                    row_bits(cy, cz + 1),
                    row_bits(cy + 1, cz + 1),
                );
                let any = a | b | c | d4;
                let all = a & b & c & d4;
                // Cell x uses points x and x + 1 (bits j and j + 1, j = x + 1).
                let mut mixed = ((any | (any >> 1)) & !(all & (all >> 1))) & cells_mask;
                if mixed == 0 {
                    continue;
                }
                let row = index(IVec3::new(-1, cy, cz));
                while mixed != 0 {
                    let j = mixed.trailing_zeros() as i32;
                    mixed &= mixed - 1;
                    let cx = j - 1;
                    let base = row + j as usize;
                    let mut mask = 0u8;
                    for (i, off) in offsets.iter().enumerate() {
                        mask |= u8::from(values_all[base + off] > 0.0) << i;
                    }
                    if mask == 0 || mask == 0xff {
                        continue;
                    }
                    let cell = IVec3::new(cx, cy, cz);
                    let values: [f32; 8] = std::array::from_fn(|i| values_all[base + offsets[i]]);
                    let mut sum = Vec3::ZERO;
                    let mut count = 0.0;
                    for (a, b) in EDGES {
                        let (va, vb) = (values[a], values[b]);
                        if (va > 0.0) != (vb > 0.0) {
                            let t = va / (va - vb);
                            sum += CORNERS[a].as_vec3() + (CORNERS[b] - CORNERS[a]).as_vec3() * t;
                            count += 1.0;
                        }
                    }
                    let local = sum / count;
                    // Gradient of the trilinear field at the vertex: outward
                    // normal points to decreasing density.
                    let gx = (values[1] - values[0]) * (1.0 - local.y) * (1.0 - local.z)
                        + (values[3] - values[2]) * local.y * (1.0 - local.z)
                        + (values[5] - values[4]) * (1.0 - local.y) * local.z
                        + (values[7] - values[6]) * local.y * local.z;
                    let gy = (values[2] - values[0]) * (1.0 - local.x) * (1.0 - local.z)
                        + (values[3] - values[1]) * local.x * (1.0 - local.z)
                        + (values[6] - values[4]) * (1.0 - local.x) * local.z
                        + (values[7] - values[5]) * local.x * local.z;
                    let gz = (values[4] - values[0]) * (1.0 - local.x) * (1.0 - local.y)
                        + (values[5] - values[1]) * local.x * (1.0 - local.y)
                        + (values[6] - values[2]) * (1.0 - local.x) * local.y
                        + (values[7] - values[3]) * local.x * local.y;
                    let normal = (-Vec3::new(gx, gy, gz)).normalize_or(Vec3::Y);
                    let (dominant, secondary, blend) = match materials {
                        None => (0, 0, 0),
                        Some(m) => {
                            let mut counts: [(u16, u8); 8] = [(0, 0); 8];
                            let mut used = 0;
                            for (i, off) in offsets.iter().enumerate() {
                                if mask & (1 << i) == 0 {
                                    continue;
                                }
                                let id = m.data()[base + off].to_u32() as u16;
                                match counts[..used].iter_mut().find(|(c, _)| *c == id) {
                                    Some(entry) => entry.1 += 1,
                                    None => {
                                        counts[used] = (id, 1);
                                        used += 1;
                                    }
                                }
                            }
                            let slice = &mut counts[..used];
                            slice.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                            let (d0, c0) = slice[0];
                            let (d1, c1) = slice.get(1).copied().unwrap_or((d0, 0));
                            (d0, d1, ((u32::from(c1) * 255) / u32::from(c0 + c1)) as u8)
                        }
                    };
                    self.cell_vertex[cell_index(cell)] = self.vertices.len() as u32;
                    self.cell_of_vertex.push(cell);
                    self.vertices.push(SmoothVertex {
                        position: (cell.as_vec3() + local) * options.scale,
                        normal,
                        materials: [dominant, secondary],
                        blend,
                    });
                    // Owned edges leave this cell's minimum corner (the
                    // lattice point `cell`) along +X/+Y/+Z. Their other three
                    // cells precede this one in iteration order.
                    if cx < 0 || cy < 0 || cz < 0 {
                        continue;
                    }
                    let solid0 = mask & 1 != 0;
                    for (axis, bit) in [(0usize, 2u8), (1, 4), (2, 16)] {
                        if (mask & bit != 0) == solid0 {
                            continue;
                        }
                        let (du, dv) = match axis {
                            0 => (IVec3::Y, IVec3::Z),
                            1 => (IVec3::Z, IVec3::X),
                            _ => (IVec3::X, IVec3::Y),
                        };
                        let quad = [cell - du - dv, cell - dv, cell, cell - du]
                            .map(|c| self.cell_vertex[cell_index(c)]);
                        if quad.contains(&NONE) {
                            continue;
                        }
                        // Solid at the base point: face points along +axis.
                        let order = if solid0 {
                            [quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]]
                        } else {
                            [quad[0], quad[2], quad[1], quad[0], quad[3], quad[2]]
                        };
                        self.indices.extend_from_slice(&order);
                        if track_edges {
                            for i in 0..4 {
                                let (a, b) = (quad[i], quad[(i + 1) % 4]);
                                *edge_count.entry((a.min(b), a.max(b))).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }
        }
        let triangles = self.indices.len() / 3;
        let mut skirt_triangles = 0;
        if track_edges {
            let depth = if options.skirt_depth > 0.0 {
                options.skirt_depth
            } else {
                options.scale
            };
            let on_seam = |cell: IVec3| {
                Face::ALL.into_iter().any(|face| {
                    options.seam_faces & face.bit() != 0 && {
                        let c = cell[face.axis()];
                        if face.is_positive() {
                            c == n - 1
                        } else {
                            c <= 0
                        }
                    }
                })
            };
            let mut boundary: Vec<(u32, u32)> = edge_count
                .into_iter()
                .filter(|(_, count)| *count == 1)
                .map(|(edge, _)| edge)
                .filter(|(a, b)| {
                    on_seam(self.cell_of_vertex[*a as usize])
                        && on_seam(self.cell_of_vertex[*b as usize])
                })
                .collect();
            boundary.sort_unstable();
            for (a, b) in boundary {
                let va = self.vertices[a as usize];
                let vb = self.vertices[b as usize];
                let a2 = self.vertices.len() as u32;
                self.vertices.push(SmoothVertex {
                    position: va.position - va.normal * depth,
                    ..va
                });
                let b2 = a2 + 1;
                self.vertices.push(SmoothVertex {
                    position: vb.position - vb.normal * depth,
                    ..vb
                });
                // Both windings: skirts are seen from either side of a crack.
                self.indices
                    .extend_from_slice(&[a, b, b2, a, b2, a2, a, b2, b, a, a2, b2]);
                skirt_triangles += 4;
            }
        }
        if !self.indices.is_empty() {
            sink.smooth(options.layer, &self.vertices, &self.indices);
        }
        SmoothStats {
            vertices: self.vertices.len(),
            triangles,
            skirt_triangles,
        }
    }
}
