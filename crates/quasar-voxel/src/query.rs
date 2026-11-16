//! Raycasts, density hits, AABB overlap/sweep and ordered region iteration.
//!
//! All coordinates are grid-local voxel units: voxel `v` occupies the unit
//! cube `[v, v + 1)`. Density samples sit at integer lattice points.
use crate::chunk::ChannelId;
use crate::edit::DENSITY_SCALE;
use crate::layout::{ChunkLayout, Face};
use crate::storage::{Raw, Value};
use crate::volume::{Cursor, Volume};
use quasar_math::{IVec3, Vec3};

/// Which axis steps first when a ray crosses several cell boundaries at the
/// same distance (exact float ties). The default steps X, then Y, then Z.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TieBreak(pub [usize; 3]);

impl Default for TieBreak {
    fn default() -> Self {
        Self([0, 1, 2])
    }
}

/// Result of testing one cell during a raycast.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CellHit {
    Miss,
    /// Hit at distance `t` with surface normal face.
    Hit {
        t: f32,
        face: Option<Face>,
    },
    /// Stop the ray without a hit (e.g. leaving loaded data).
    Stop,
}

/// A ray hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    pub voxel: IVec3,
    /// Face of `voxel` the ray entered through (`None` if it started inside).
    pub face: Option<Face>,
    /// Distance along the normalised direction.
    pub t: f32,
    pub point: Vec3,
}

impl RayHit {
    /// The empty cell in front of the hit face: where a placed block goes.
    pub fn adjacent(&self) -> IVec3 {
        self.voxel + self.face.map_or(IVec3::ZERO, Face::normal)
    }
}

/// Information about the cell being visited.
#[derive(Clone, Copy, Debug)]
pub struct CellVisit {
    pub voxel: IVec3,
    pub entry_face: Option<Face>,
    pub t_enter: f32,
    pub t_exit: f32,
}

/// DDA traversal (Amanatides–Woo) through unit cells. `visit` decides hits;
/// traversal is exact and deterministic under `tie`.
pub fn traverse(
    origin: Vec3,
    direction: Vec3,
    max_distance: f32,
    tie: TieBreak,
    mut visit: impl FnMut(&CellVisit) -> CellHit,
) -> Option<RayHit> {
    let length = direction.length();
    if !(length.is_finite() && length > 0.0 && origin.is_finite() && max_distance.is_finite()) {
        return None;
    }
    let dir = direction / length;
    let mut cell = origin.floor().as_ivec3();
    let step = IVec3::new(sign(dir.x), sign(dir.y), sign(dir.z));
    let next_boundary = |o: f32, c: i32, s: i32| {
        if s > 0 {
            (c + 1) as f32 - o
        } else {
            o - c as f32
        }
    };
    let mut t_max = Vec3::new(
        axis_t(next_boundary(origin.x, cell.x, step.x), dir.x),
        axis_t(next_boundary(origin.y, cell.y, step.y), dir.y),
        axis_t(next_boundary(origin.z, cell.z, step.z), dir.z),
    );
    let t_delta = Vec3::new(axis_t(1.0, dir.x), axis_t(1.0, dir.y), axis_t(1.0, dir.z));
    let mut t = 0.0f32;
    let mut entry: Option<Face> = None;
    loop {
        let t_exit = t_max.min_element();
        let info = CellVisit {
            voxel: cell,
            entry_face: entry,
            t_enter: t,
            t_exit,
        };
        match visit(&info) {
            CellHit::Miss => {}
            CellHit::Stop => return None,
            CellHit::Hit { t: hit, face } => {
                return Some(RayHit {
                    voxel: cell,
                    face,
                    t: hit,
                    point: origin + dir * hit,
                });
            }
        }
        // Pick the axis with the smallest t_max; exact ties follow `tie`.
        let mut axis = tie.0[0];
        for &candidate in &tie.0[1..] {
            if t_max[candidate] < t_max[axis] {
                axis = candidate;
            }
        }
        t = t_max[axis];
        if t > max_distance {
            return None;
        }
        cell[axis] += step[axis];
        t_max[axis] += t_delta[axis];
        entry = Face::from_index(axis * 2 + usize::from(step[axis] < 0));
    }
}

fn sign(v: f32) -> i32 {
    if v > 0.0 {
        1
    } else if v < 0.0 {
        -1
    } else {
        0
    }
}

fn axis_t(distance: f32, d: f32) -> f32 {
    if d == 0.0 {
        f32::INFINITY
    } else {
        distance / d.abs()
    }
}

impl<L: ChunkLayout> Volume<L> {
    /// First voxel along the ray for which `solid(voxel, value)` holds.
    /// Chunks whose channel is uniform and not solid are skipped quickly.
    pub fn raycast<T: Value>(
        &self,
        channel: ChannelId<T>,
        origin: Vec3,
        direction: Vec3,
        max_distance: f32,
        tie: TieBreak,
        mut solid: impl FnMut(IVec3, T) -> bool,
    ) -> Option<RayHit> {
        let mut cursor = Cursor::default();
        let mut skip_chunk: Option<IVec3> = None;
        traverse(origin, direction, max_distance, tie, |visit| {
            let chunk = L::chunk_of(visit.voxel);
            if skip_chunk == Some(chunk) {
                return CellHit::Miss;
            }
            let uniform = match self.chunk(chunk) {
                None => Some(<T::Raw as Raw>::from_u32(self.default_raw(channel.index()))),
                Some(c) => c.storage(channel).uniform(),
            };
            if let Some(raw) = uniform
                && !solid(visit.voxel, T::from_raw(raw))
            {
                skip_chunk = Some(chunk);
                return CellHit::Miss;
            }
            skip_chunk = None;
            let value = self.get_with(&mut cursor, channel, visit.voxel);
            if solid(visit.voxel, value) {
                CellHit::Hit {
                    t: visit.t_enter,
                    face: visit.entry_face,
                }
            } else {
                CellHit::Miss
            }
        })
    }

    /// Trilinear density (in voxels, positive inside) at a lattice-space point.
    pub fn sample_density(&self, channel: ChannelId<i8>, p: Vec3) -> f32 {
        let base = p.floor();
        let f = p - base;
        let b = base.as_ivec3();
        let d = |x: i32, y: i32, z: i32| f32::from(self.get(channel, b + IVec3::new(x, y, z)));
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let x00 = lerp(d(0, 0, 0), d(1, 0, 0), f.x);
        let x10 = lerp(d(0, 1, 0), d(1, 1, 0), f.x);
        let x01 = lerp(d(0, 0, 1), d(1, 0, 1), f.x);
        let x11 = lerp(d(0, 1, 1), d(1, 1, 1), f.x);
        lerp(lerp(x00, x10, f.y), lerp(x01, x11, f.y), f.z) / DENSITY_SCALE
    }

    /// Density gradient (outward surface normal is `-gradient`).
    pub fn density_normal(&self, channel: ChannelId<i8>, p: Vec3) -> Vec3 {
        let e = 0.5;
        let g = Vec3::new(
            self.sample_density(channel, p + Vec3::X * e)
                - self.sample_density(channel, p - Vec3::X * e),
            self.sample_density(channel, p + Vec3::Y * e)
                - self.sample_density(channel, p - Vec3::Y * e),
            self.sample_density(channel, p + Vec3::Z * e)
                - self.sample_density(channel, p - Vec3::Z * e),
        );
        (-g).normalize_or_zero()
    }

    /// First crossing of the density surface along the ray, refined by
    /// bisection; `point` is on the surface and `face` is the dominant axis
    /// of the surface normal. Steps `step` voxels at a time (0.25 is
    /// accurate for typical brushes).
    pub fn raycast_density(
        &self,
        channel: ChannelId<i8>,
        origin: Vec3,
        direction: Vec3,
        max_distance: f32,
        step: f32,
    ) -> Option<(RayHit, Vec3)> {
        let dir = direction.normalize_or_zero();
        if dir == Vec3::ZERO || step.is_nan() || step <= 0.0 {
            return None;
        }
        let mut t = 0.0;
        let mut previous = self.sample_density(channel, origin);
        if previous >= 0.0 {
            let normal = self.density_normal(channel, origin);
            return Some((
                RayHit {
                    voxel: origin.floor().as_ivec3(),
                    face: None,
                    t: 0.0,
                    point: origin,
                },
                normal,
            ));
        }
        while t < max_distance {
            let next = (t + step).min(max_distance);
            let value = self.sample_density(channel, origin + dir * next);
            if value >= 0.0 {
                let (mut lo, mut hi) = (t, next);
                let mut lo_value = previous;
                for _ in 0..16 {
                    let mid = (lo + hi) * 0.5;
                    let v = self.sample_density(channel, origin + dir * mid);
                    if (v >= 0.0) == (lo_value >= 0.0) {
                        lo = mid;
                        lo_value = v;
                    } else {
                        hi = mid;
                    }
                }
                let point = origin + dir * hi;
                let normal = self.density_normal(channel, point);
                let axis = normal.abs().max_position();
                let face = Face::from_index(axis * 2 + usize::from(normal[axis] > 0.0));
                return Some((
                    RayHit {
                        voxel: (point - normal * 0.01).floor().as_ivec3(),
                        face,
                        t: hi,
                        point,
                    },
                    normal,
                ));
            }
            previous = value;
            t = next;
        }
        None
    }

    /// Whether any solid cell overlaps the box `[min, max)`.
    pub fn overlaps<T: Value>(
        &self,
        channel: ChannelId<T>,
        min: Vec3,
        max: Vec3,
        mut solid: impl FnMut(IVec3, T) -> bool,
    ) -> bool {
        let mut cursor = Cursor::default();
        let lo = min.floor().as_ivec3();
        let hi = (max - Vec3::splat(1e-5)).floor().as_ivec3();
        for y in lo.y..=hi.y {
            for z in lo.z..=hi.z {
                for x in lo.x..=hi.x {
                    let v = IVec3::new(x, y, z);
                    if solid(v, self.get_with(&mut cursor, channel, v)) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Sweeps the box `[min, max)` by `delta` and returns the first contact
    /// fraction in `[0, 1]` with the face of the solid cell that was hit.
    pub fn sweep_aabb<T: Value>(
        &self,
        channel: ChannelId<T>,
        min: Vec3,
        max: Vec3,
        delta: Vec3,
        mut solid: impl FnMut(IVec3, T) -> bool,
    ) -> Option<(f32, Face)> {
        let lo = (min.min(min + delta)).floor().as_ivec3();
        let hi = (max.max(max + delta)).ceil().as_ivec3();
        let mut cursor = Cursor::default();
        let mut best: Option<(f32, Face)> = None;
        for y in lo.y..hi.y {
            for z in lo.z..hi.z {
                for x in lo.x..hi.x {
                    let v = IVec3::new(x, y, z);
                    if !solid(v, self.get_with(&mut cursor, channel, v)) {
                        continue;
                    }
                    // Minkowski: expand the cell by the box and ray-cast its centre.
                    let cmin = v.as_vec3() - (max - min) * 0.5;
                    let cmax = v.as_vec3() + Vec3::ONE + (max - min) * 0.5;
                    let start = (min + max) * 0.5;
                    let mut t_enter = 0.0f32;
                    let mut t_exit = 1.0f32;
                    let mut face = None;
                    let mut hit = true;
                    for axis in 0..3 {
                        let (s, d) = (start[axis], delta[axis]);
                        if d.abs() < 1e-12 {
                            if s <= cmin[axis] || s >= cmax[axis] {
                                hit = false;
                                break;
                            }
                            continue;
                        }
                        let mut t0 = (cmin[axis] - s) / d;
                        let mut t1 = (cmax[axis] - s) / d;
                        let mut f = Face::from_index(axis * 2).unwrap();
                        if t0 > t1 {
                            std::mem::swap(&mut t0, &mut t1);
                            f = Face::from_index(axis * 2 + 1).unwrap();
                        }
                        if t0 > t_enter {
                            t_enter = t0;
                            face = Some(f);
                        }
                        t_exit = t_exit.min(t1);
                        if t_enter > t_exit {
                            hit = false;
                            break;
                        }
                    }
                    if hit
                        && let Some(face) = face
                        && best.is_none_or(|(t, _)| t_enter < t)
                    {
                        best = Some((t_enter, face));
                    }
                }
            }
        }
        best
    }

    /// Visits every voxel in the inclusive box in `y`, `z`, `x` order.
    pub fn for_each_in_box<T: Value>(
        &self,
        channel: ChannelId<T>,
        min: IVec3,
        max: IVec3,
        mut visit: impl FnMut(IVec3, T),
    ) {
        let mut cursor = Cursor::default();
        let (lo, hi) = (min.min(max), min.max(max));
        for y in lo.y..=hi.y {
            for z in lo.z..=hi.z {
                for x in lo.x..=hi.x {
                    let v = IVec3::new(x, y, z);
                    visit(v, self.get_with(&mut cursor, channel, v));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Schema;
    use crate::edit::{Brush, BrushOp, BrushShape, EditBatch, EditOptions};
    use crate::layout::Cube16;
    use crate::storage::StorageKind;

    fn blocks() -> (Volume<Cube16>, ChannelId<u16>) {
        let mut schema = Schema::new();
        let block = schema.channel("block", StorageKind::Paletted, 0u16);
        (Volume::new(schema), block)
    }

    #[test]
    fn raycasts_hit_the_first_solid_cell_and_report_the_entry_face() {
        let (mut volume, block) = blocks();
        volume.set(block, IVec3::new(40, 2, 2), 1);
        volume.set(block, IVec3::new(-20, 2, 2), 1);
        let hit = volume
            .raycast(
                block,
                Vec3::new(0.5, 2.5, 2.5),
                Vec3::X,
                100.0,
                TieBreak::default(),
                |_, v| v != 0,
            )
            .unwrap();
        assert_eq!(hit.voxel, IVec3::new(40, 2, 2));
        assert_eq!(hit.face, Some(Face::NegX));
        assert!((hit.t - 39.5).abs() < 1e-4);
        assert_eq!(hit.adjacent(), IVec3::new(39, 2, 2));
        let back = volume
            .raycast(
                block,
                Vec3::new(0.5, 2.5, 2.5),
                -Vec3::X,
                100.0,
                TieBreak::default(),
                |_, v| v != 0,
            )
            .unwrap();
        assert_eq!(back.face, Some(Face::PosX));
        assert!(
            volume
                .raycast(
                    block,
                    Vec3::new(0.5, 2.5, 2.5),
                    Vec3::Y,
                    100.0,
                    TieBreak::default(),
                    |_, v| v != 0
                )
                .is_none()
        );
        assert!(
            traverse(Vec3::ZERO, Vec3::ZERO, 1.0, TieBreak::default(), |_| {
                CellHit::Miss
            })
            .is_none()
        );
    }

    #[test]
    fn ties_follow_the_policy_and_traversal_matches_brute_force() {
        // A diagonal ray through a lattice corner crosses x and y at once.
        let mut visited = Vec::new();
        traverse(
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(1.0, 1.0, 0.0),
            2.0,
            TieBreak([1, 0, 2]),
            |v| {
                visited.push(v.voxel);
                CellHit::Miss
            },
        );
        assert_eq!(
            visited[1],
            IVec3::new(0, 1, 0),
            "Y steps first under this policy"
        );
        let mut default_order = Vec::new();
        traverse(
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(1.0, 1.0, 0.0),
            2.0,
            TieBreak::default(),
            |v| {
                default_order.push(v.voxel);
                CellHit::Miss
            },
        );
        assert_eq!(default_order[1], IVec3::new(1, 0, 0));
        // Every visited cell actually intersects the ray segment.
        let mut rng = quasar_core::Rng::seed_from_u64(5);
        for _ in 0..200 {
            let o = Vec3::new(
                rng.range_f32(-5.0..5.0),
                rng.range_f32(-5.0..5.0),
                rng.range_f32(-5.0..5.0),
            );
            let d = Vec3::new(
                rng.range_f32(-1.0..1.0),
                rng.range_f32(-1.0..1.0),
                rng.range_f32(-1.0..1.0),
            );
            let mut prev: Option<IVec3> = None;
            traverse(o, d, 10.0, TieBreak::default(), |v| {
                if let Some(p) = prev {
                    assert_eq!((v.voxel - p).abs().element_sum(), 1, "face-adjacent steps");
                }
                prev = Some(v.voxel);
                CellHit::Miss
            });
        }
    }

    #[test]
    fn density_rays_find_the_surface() {
        let mut schema = Schema::new();
        let density = schema.channel("density", StorageKind::Dense, -127i8);
        let mut volume = Volume::<Cube16>::new(schema);
        let mut batch = EditBatch::new();
        batch.brush(Brush::new(
            BrushShape::Sphere {
                center: Vec3::new(20.0, 0.0, 0.0),
                radius: 6.0,
            },
            BrushOp::Add,
            density,
        ));
        volume.apply(&batch, EditOptions::default());
        let (hit, normal) = volume
            .raycast_density(density, Vec3::ZERO, Vec3::X, 50.0, 0.25)
            .unwrap();
        assert!((hit.point.x - 14.0).abs() < 0.2, "{:?}", hit.point);
        assert!(normal.x < -0.9, "{normal}");
        assert_eq!(hit.face, Some(Face::NegX));
        assert!(
            volume
                .raycast_density(density, Vec3::ZERO, -Vec3::X, 50.0, 0.25)
                .is_none()
        );
    }

    #[test]
    fn boxes_overlap_and_sweep_against_solid_cells() {
        let (mut volume, block) = blocks();
        for x in 0..4 {
            for z in 0..4 {
                volume.set(block, IVec3::new(x, 0, z), 1);
            }
        }
        let solid = |_: IVec3, v: u16| v != 0;
        assert!(volume.overlaps(
            block,
            Vec3::new(1.0, 0.5, 1.0),
            Vec3::new(1.5, 1.5, 1.5),
            solid
        ));
        assert!(!volume.overlaps(
            block,
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(1.5, 2.0, 1.5),
            solid
        ));
        let (t, face) = volume
            .sweep_aabb(
                block,
                Vec3::new(1.0, 3.0, 1.0),
                Vec3::new(1.6, 4.8, 1.6),
                Vec3::new(0.0, -5.0, 0.0),
                solid,
            )
            .unwrap();
        assert!((t - 0.4).abs() < 1e-4, "{t}");
        assert_eq!(face, Face::PosY);
        assert!(
            volume
                .sweep_aabb(
                    block,
                    Vec3::new(10.0, 3.0, 1.0),
                    Vec3::new(10.6, 4.8, 1.6),
                    Vec3::new(0.0, -5.0, 0.0),
                    solid
                )
                .is_none()
        );
        let mut count = 0;
        let mut order = Vec::new();
        volume.for_each_in_box(
            block,
            IVec3::new(0, 0, 0),
            IVec3::new(1, 0, 1),
            |v, value| {
                count += usize::from(value != 0);
                order.push(v);
            },
        );
        assert_eq!(count, 4);
        assert_eq!(order[1], IVec3::new(1, 0, 0), "x fastest");
    }
}
