//! Ordered edit batches, density brushes, change events and dirtiness.
//!
//! Edits are data ([`EditBatch`]) so they can be validated, replicated over
//! the network, recorded for replay and applied deterministically. Applying a
//! batch returns every change in application order plus the sorted set of
//! chunks whose padded snapshots are affected.
use crate::chunk::ChannelId;
use crate::layout::ChunkLayout;
use crate::volume::Volume;
use quasar_math::{IVec3, Vec3};

/// Density units per voxel. Density channels store signed distance to the
/// surface in `1/DENSITY_SCALE` voxels, clamped to `i8`: positive is inside
/// (solid), negative is outside, the surface is at 0.
pub const DENSITY_SCALE: f32 = 16.0;

/// Quantises a signed distance (in voxels, positive inside).
#[inline]
pub fn quantize_density(distance_inside: f32) -> i8 {
    (distance_inside * DENSITY_SCALE)
        .round()
        .clamp(-127.0, 127.0) as i8
}

/// Shape of a density brush, in grid-local voxel coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BrushShape {
    Sphere { center: Vec3, radius: f32 },
    Box { min: Vec3, max: Vec3 },
    Capsule { a: Vec3, b: Vec3, radius: f32 },
}

impl BrushShape {
    /// Signed distance from `p` to the shape surface, positive inside.
    pub fn inside_distance(&self, p: Vec3) -> f32 {
        match *self {
            Self::Sphere { center, radius } => radius - (p - center).length(),
            Self::Box { min, max } => {
                let center = (min + max) * 0.5;
                let half = (max - min) * 0.5;
                let q = (p - center).abs() - half;
                -(q.max(Vec3::ZERO).length() + q.max_element().min(0.0))
            }
            Self::Capsule { a, b, radius } => {
                let ab = b - a;
                let t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
                radius - (p - (a + ab * t)).length()
            }
        }
    }
    /// Voxel bounds touched (inclusive), with a margin for the smooth band.
    pub fn bounds(&self, margin: f32) -> (IVec3, IVec3) {
        let (min, max) = match *self {
            Self::Sphere { center, radius } => (center - radius, center + radius),
            Self::Box { min, max } => (min, max),
            Self::Capsule { a, b, radius } => (a.min(b) - radius, a.max(b) + radius),
        };
        (
            (min - margin).floor().as_ivec3(),
            (max + margin).ceil().as_ivec3(),
        )
    }
}

/// What a brush does to density.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BrushOp {
    /// Union: material appears inside the shape.
    Add,
    /// Difference: material is removed inside the shape.
    Subtract,
    /// Only changes the material channel where already solid.
    Paint,
}

/// A density brush on a density channel, optionally writing a material.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Brush {
    pub shape: BrushShape,
    pub op: BrushOp,
    pub density: u16,
    /// `(channel, raw value)` written where the brush adds or paints.
    pub material: Option<(u16, u32)>,
}

impl Brush {
    pub fn new(shape: BrushShape, op: BrushOp, density: ChannelId<i8>) -> Self {
        Self {
            shape,
            op,
            density: density.index() as u16,
            material: None,
        }
    }
    pub fn with_material<T: crate::storage::Value>(
        mut self,
        channel: ChannelId<T>,
        value: T,
    ) -> Self {
        use crate::storage::Raw;
        self.material = Some((channel.index() as u16, value.to_raw().to_u32()));
        self
    }
}

/// One edit operation.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum EditOp {
    /// Sets one voxel of a channel to a raw value.
    Set {
        channel: u16,
        voxel: IVec3,
        value: u32,
    },
    /// Sets every voxel in an inclusive box.
    Fill {
        channel: u16,
        min: IVec3,
        max: IVec3,
        value: u32,
    },
    Brush(Brush),
}

/// An ordered list of edits.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EditBatch {
    pub ops: Vec<EditOp>,
}

impl EditBatch {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set<T: crate::storage::Value>(
        &mut self,
        channel: ChannelId<T>,
        voxel: IVec3,
        value: T,
    ) -> &mut Self {
        use crate::storage::Raw;
        self.ops.push(EditOp::Set {
            channel: channel.index() as u16,
            voxel,
            value: value.to_raw().to_u32(),
        });
        self
    }
    pub fn fill<T: crate::storage::Value>(
        &mut self,
        channel: ChannelId<T>,
        min: IVec3,
        max: IVec3,
        value: T,
    ) -> &mut Self {
        use crate::storage::Raw;
        self.ops.push(EditOp::Fill {
            channel: channel.index() as u16,
            min: min.min(max),
            max: min.max(max),
            value: value.to_raw().to_u32(),
        });
        self
    }
    pub fn brush(&mut self, brush: Brush) -> &mut Self {
        self.ops.push(EditOp::Brush(brush));
        self
    }
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
    pub fn len(&self) -> usize {
        self.ops.len()
    }
}

/// One changed voxel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Change {
    pub channel: u16,
    pub voxel: IVec3,
    pub old: u32,
    pub new: u32,
}

/// Options for [`Volume::apply`].
#[derive(Clone, Copy, Debug)]
pub struct EditOptions {
    /// Neighbouring chunks are dirtied when a change is within `halo` voxels
    /// of their shared border (the padding consumers read). Default 1.
    pub halo: u32,
    /// Record per-voxel changes (disable for huge fills when only dirtiness
    /// is needed).
    pub record_changes: bool,
    /// Only edit loaded chunks; edits elsewhere are skipped.
    pub loaded_only: bool,
}

impl Default for EditOptions {
    fn default() -> Self {
        Self {
            halo: 1,
            record_changes: true,
            loaded_only: false,
        }
    }
}

/// Outcome of applying a batch.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditResult {
    /// Every changed voxel in application order.
    pub changes: Vec<Change>,
    /// Chunks needing re-extraction (changed chunks plus halo neighbours), sorted.
    pub dirty: Vec<IVec3>,
    /// Voxels whose value changed (counted even when not recorded).
    pub changed: usize,
}

pub(crate) fn dirty_for<L: ChunkLayout>(voxel: IVec3, halo: u32, out: &mut Vec<IVec3>) {
    let chunk = L::chunk_of(voxel);
    let local = L::local_of(voxel).as_ivec3();
    let h = halo as i32;
    let n = L::EDGE as i32;
    let range =
        |v: i32| -> (i32, i32) { (if v < h { -1 } else { 0 }, if v >= n - h { 1 } else { 0 }) };
    let (x0, x1) = range(local.x);
    let (y0, y1) = range(local.y);
    let (z0, z1) = range(local.z);
    for dz in z0..=z1 {
        for dy in y0..=y1 {
            for dx in x0..=x1 {
                out.push(chunk + IVec3::new(dx, dy, dz));
            }
        }
    }
}

impl<L: ChunkLayout> Volume<L> {
    /// Applies a batch in order. See [`EditResult`].
    pub fn apply(&mut self, batch: &EditBatch, options: EditOptions) -> EditResult {
        let mut result = EditResult::default();
        let mut dirty = Vec::new();
        let mut write =
            |volume: &mut Self, channel: u16, voxel: IVec3, value: u32, result: &mut EditResult| {
                if options.loaded_only && !volume.contains(L::chunk_of(voxel)) {
                    return;
                }
                let old = volume.set_raw(channel as usize, voxel, value);
                if old != value {
                    result.changed += 1;
                    if options.record_changes {
                        result.changes.push(Change {
                            channel,
                            voxel,
                            old,
                            new: value,
                        });
                    }
                    dirty_for::<L>(voxel, options.halo, &mut dirty);
                }
            };
        for op in &batch.ops {
            match op {
                EditOp::Set {
                    channel,
                    voxel,
                    value,
                } => write(self, *channel, *voxel, *value, &mut result),
                EditOp::Fill {
                    channel,
                    min,
                    max,
                    value,
                } => {
                    for y in min.y..=max.y {
                        for z in min.z..=max.z {
                            for x in min.x..=max.x {
                                write(self, *channel, IVec3::new(x, y, z), *value, &mut result);
                            }
                        }
                    }
                }
                EditOp::Brush(brush) => {
                    let band = 127.0 / DENSITY_SCALE;
                    let (min, max) = brush.shape.bounds(band.min(2.0));
                    for y in min.y..=max.y {
                        for z in min.z..=max.z {
                            for x in min.x..=max.x {
                                let voxel = IVec3::new(x, y, z);
                                let inside = brush.shape.inside_distance(voxel.as_vec3());
                                let current =
                                    self.get_raw(brush.density as usize, voxel) as u8 as i8;
                                let shaped = quantize_density(inside);
                                let next = match brush.op {
                                    BrushOp::Add => current.max(shaped),
                                    BrushOp::Subtract => current.min(shaped.saturating_neg()),
                                    BrushOp::Paint => current,
                                };
                                if next != current {
                                    write(
                                        self,
                                        brush.density,
                                        voxel,
                                        next as u8 as u32,
                                        &mut result,
                                    );
                                }
                                if let Some((channel, material)) = brush.material {
                                    let paint = match brush.op {
                                        BrushOp::Add => shaped > current,
                                        BrushOp::Paint => inside >= 0.0 && current >= 0,
                                        BrushOp::Subtract => false,
                                    };
                                    if paint {
                                        write(self, channel, voxel, material, &mut result);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        dirty.sort_by_key(|c| (c.x, c.y, c.z));
        dirty.dedup();
        result.dirty = dirty;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Schema;
    use crate::layout::Cube16;
    use crate::storage::StorageKind;

    #[test]
    fn batches_apply_in_order_and_report_dirty_neighbours() {
        let mut schema = Schema::new();
        let block = schema.channel("block", StorageKind::Paletted, 0u16);
        let mut volume = Volume::<Cube16>::new(schema);
        let mut batch = EditBatch::new();
        batch
            .set(block, IVec3::new(5, 5, 5), 1)
            .set(block, IVec3::new(5, 5, 5), 2)
            .set(block, IVec3::new(15, 0, 5), 3)
            .fill(block, IVec3::new(0, 20, 0), IVec3::new(1, 20, 1), 4);
        let result = volume.apply(&batch, EditOptions::default());
        assert_eq!(result.changed, 7);
        assert_eq!(
            result.changes[0],
            Change {
                channel: 0,
                voxel: IVec3::new(5, 5, 5),
                old: 0,
                new: 1
            }
        );
        assert_eq!(result.changes[1].old, 1);
        assert_eq!(volume.get(block, IVec3::new(5, 5, 5)), 2);
        assert!(
            result.dirty.contains(&IVec3::new(1, 0, 0)),
            "x border neighbour"
        );
        assert!(
            result.dirty.contains(&IVec3::new(0, -1, 0)),
            "y = 0 border neighbour"
        );
        assert!(
            result.dirty.contains(&IVec3::new(-1, 1, -1)),
            "corner of the fill"
        );
        let mut sorted = result.dirty.clone();
        sorted.sort_by_key(|c| (c.x, c.y, c.z));
        assert_eq!(sorted, result.dirty);
        // No-op edits change nothing.
        let again = volume.apply(&batch, EditOptions::default());
        assert_eq!(
            again.changed, 2,
            "replaying 2 -> 1 -> 2 changes the cell twice"
        );
    }

    #[test]
    fn brushes_add_and_subtract_density_with_materials() {
        let mut schema = Schema::new();
        let density = schema.channel("density", StorageKind::Dense, -127i8);
        let material = schema.channel("material", StorageKind::Paletted, 0u8);
        let mut volume = Volume::<Cube16>::new(schema);
        let sphere = BrushShape::Sphere {
            center: Vec3::splat(8.0),
            radius: 5.0,
        };
        let mut batch = EditBatch::new();
        batch.brush(Brush::new(sphere, BrushOp::Add, density).with_material(material, 3u8));
        volume.apply(&batch, EditOptions::default());
        assert!(volume.get(density, IVec3::splat(8)) > 0);
        assert!(volume.get(density, IVec3::new(8, 8, 15)) < 0);
        assert_eq!(volume.get(material, IVec3::splat(8)), 3);
        let surface = volume.get(density, IVec3::new(13, 8, 8));
        assert!(surface.abs() <= 1, "surface at the radius: {surface}");
        let mut carve = EditBatch::new();
        carve.brush(Brush::new(
            BrushShape::Sphere {
                center: Vec3::splat(8.0),
                radius: 2.0,
            },
            BrushOp::Subtract,
            density,
        ));
        volume.apply(&carve, EditOptions::default());
        assert!(volume.get(density, IVec3::splat(8)) < 0, "hollowed");
        assert!(
            volume.get(density, IVec3::new(8, 8, 12)) > 0,
            "shell remains"
        );
        let boxy = BrushShape::Box {
            min: Vec3::ZERO,
            max: Vec3::splat(2.0),
        };
        assert!(boxy.inside_distance(Vec3::splat(1.0)) > 0.0);
        let capsule = BrushShape::Capsule {
            a: Vec3::ZERO,
            b: Vec3::X * 4.0,
            radius: 1.0,
        };
        assert!(capsule.inside_distance(Vec3::new(2.0, 0.5, 0.0)) > 0.0);
        assert_eq!(quantize_density(100.0), 127);
    }

    #[test]
    fn loaded_only_edits_skip_absent_chunks() {
        let mut schema = Schema::new();
        let block = schema.channel("block", StorageKind::Paletted, 0u16);
        let mut volume = Volume::<Cube16>::new(schema);
        let mut batch = EditBatch::new();
        batch.set(block, IVec3::new(100, 0, 0), 1);
        let result = volume.apply(
            &batch,
            EditOptions {
                loaded_only: true,
                ..Default::default()
            },
        );
        assert_eq!(result.changed, 0);
        assert_eq!(volume.chunk_count(), 0);
    }
}
