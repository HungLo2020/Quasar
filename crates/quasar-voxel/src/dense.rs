//! Dense read views: a flat, bounds-checked copy of one channel over a box.
//!
//! [`Volume::get`] hashes the chunk coordinate and unpacks a paletted value
//! on every call, which is the right trade for sparse, unbounded worlds but
//! slow for small fixed maps queried thousands of times a tick (collision,
//! line of sight, "nearest ore" scans). A [`DenseView`] copies one channel of
//! a box into a flat array once, reads in O(1) with no hashing, and
//! [`DenseView::refresh`] re-copies only the chunks whose revision changed.
//!
//! ```
//! use quasar_math::IVec3;
//! use quasar_voxel::{Cube16, Schema, StorageKind, Volume};
//!
//! let mut schema = Schema::new();
//! let block = schema.channel("block", StorageKind::Paletted, 0u16);
//! let mut volume = Volume::<Cube16>::new(schema);
//! volume.set(block, IVec3::new(3, 0, 4), 7);
//!
//! let mut view = volume.dense_view(block, IVec3::ZERO, IVec3::new(120, 1, 120));
//! assert_eq!(view.get(IVec3::new(3, 0, 4)), 7);
//! volume.set(block, IVec3::new(9, 0, 9), 2);
//! assert_eq!(view.refresh(&volume), 1); // one chunk changed
//! assert_eq!(view.get_xyz(9, 0, 9), 2);
//! ```
use crate::chunk::ChannelId;
use crate::layout::ChunkLayout;
use crate::storage::{Raw, Value};
use crate::volume::Volume;
use quasar_math::IVec3;

/// See the module docs.
#[derive(Clone, Debug)]
pub struct DenseView<T: Value> {
    channel: ChannelId<T>,
    min: IVec3,
    size: IVec3,
    values: Vec<T>,
    outside: T,
    /// Covered chunk coordinates and the revision copied (`None` = absent).
    chunks: Vec<(IVec3, Option<u64>)>,
}

impl<L: ChunkLayout> Volume<L> {
    /// Copies `id` over the box `[min, max)` into a [`DenseView`]. Reads
    /// outside the box return the channel default.
    pub fn dense_view<T: Value>(&self, id: ChannelId<T>, min: IVec3, max: IVec3) -> DenseView<T> {
        let size = (max - min).max(IVec3::ZERO);
        let outside = T::from_raw(<T::Raw as Raw>::from_u32(self.default_raw(id.index())));
        let count = size.x as usize * size.y as usize * size.z as usize;
        let mut view = DenseView {
            channel: id,
            min,
            size,
            values: vec![outside; count],
            outside,
            chunks: Vec::new(),
        };
        if count == 0 {
            return view;
        }
        let lo = L::chunk_of(min);
        let hi = L::chunk_of(max - IVec3::ONE);
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    let coord = IVec3::new(x, y, z);
                    let revision = view.copy_chunk(self, coord);
                    view.chunks.push((coord, revision));
                }
            }
        }
        view
    }
}

impl<T: Value> DenseView<T> {
    pub fn min(&self) -> IVec3 {
        self.min
    }
    pub fn size(&self) -> IVec3 {
        self.size
    }
    pub fn max(&self) -> IVec3 {
        self.min + self.size
    }
    pub fn channel(&self) -> ChannelId<T> {
        self.channel
    }
    /// Values in x-fastest, then y, then z order.
    pub fn values(&self) -> &[T] {
        &self.values
    }
    pub fn contains(&self, voxel: IVec3) -> bool {
        let local = voxel - self.min;
        local.cmpge(IVec3::ZERO).all() && local.cmplt(self.size).all()
    }
    /// Flat index of `voxel` in [`Self::values`], if inside the box.
    #[inline]
    pub fn index_of(&self, voxel: IVec3) -> Option<usize> {
        let local = voxel - self.min;
        if local.x < 0
            || local.y < 0
            || local.z < 0
            || local.x >= self.size.x
            || local.y >= self.size.y
            || local.z >= self.size.z
        {
            return None;
        }
        Some(
            local.x as usize
                + self.size.x as usize
                    * (local.y as usize + self.size.y as usize * local.z as usize),
        )
    }
    /// The voxel's value, or the channel default outside the box.
    #[inline]
    pub fn get(&self, voxel: IVec3) -> T {
        match self.index_of(voxel) {
            Some(index) => self.values[index],
            None => self.outside,
        }
    }
    #[inline]
    pub fn get_xyz(&self, x: i32, y: i32, z: i32) -> T {
        self.get(IVec3::new(x, y, z))
    }
    /// Mirrors a write the game also made to the volume, so the view stays
    /// current without a refresh. Returns `false` outside the box.
    pub fn set(&mut self, voxel: IVec3, value: T) -> bool {
        match self.index_of(voxel) {
            Some(index) => {
                self.values[index] = value;
                true
            }
            None => false,
        }
    }
    /// Re-copies chunks whose revision (or presence) changed since the view
    /// was built or last refreshed, returning how many were copied. A chunk
    /// replaced wholesale by one with the same revision is not detected;
    /// rebuild the view after swapping chunks in.
    pub fn refresh<L: ChunkLayout>(&mut self, volume: &Volume<L>) -> usize {
        let mut copied = 0;
        for i in 0..self.chunks.len() {
            let (coord, seen) = self.chunks[i];
            let now = volume.chunk(coord).map(|c| c.revision());
            if now != seen {
                self.chunks[i].1 = self.copy_chunk(volume, coord);
                copied += 1;
            }
        }
        copied
    }

    fn copy_chunk<L: ChunkLayout>(&mut self, volume: &Volume<L>, coord: IVec3) -> Option<u64> {
        let edge = L::EDGE as i32;
        let base = coord * edge;
        let lo = base.max(self.min);
        let hi = (base + IVec3::splat(edge)).min(self.min + self.size);
        let row = |view: &Self, y: i32, z: i32| {
            view.index_of(IVec3::new(lo.x, y, z))
                .expect("inside the box")
        };
        let Some(chunk) = volume.chunk(coord) else {
            for z in lo.z..hi.z {
                for y in lo.y..hi.y {
                    let start = row(self, y, z);
                    self.values[start..start + (hi.x - lo.x) as usize].fill(self.outside);
                }
            }
            return None;
        };
        let data = chunk.channel_data(self.channel.index());
        if let Some(uniform) = data.uniform_u32() {
            let value = T::from_raw(<T::Raw as Raw>::from_u32(uniform));
            for z in lo.z..hi.z {
                for y in lo.y..hi.y {
                    let start = row(self, y, z);
                    self.values[start..start + (hi.x - lo.x) as usize].fill(value);
                }
            }
        } else {
            for z in lo.z..hi.z {
                for y in lo.y..hi.y {
                    let start = row(self, y, z);
                    for x in lo.x..hi.x {
                        let local = IVec3::new(x, y, z) - base;
                        let index = L::index(local.x as u32, local.y as u32, local.z as u32);
                        self.values[start + (x - lo.x) as usize] =
                            chunk.get_index(self.channel, index);
                    }
                }
            }
        }
        Some(chunk.revision())
    }
}

#[cfg(test)]
mod tests {
    use crate::{Cube16, Schema, StorageKind, Volume};
    use quasar_math::IVec3;

    #[test]
    fn dense_views_match_point_reads_and_refresh_changed_chunks() {
        let mut schema = Schema::new();
        let block = schema.channel("block", StorageKind::Paletted, 5u16);
        let mut volume = Volume::<Cube16>::new(schema);
        for i in 0..400 {
            let v = IVec3::new((i * 37) % 70 - 10, (i * 11) % 3, (i * 53) % 60 - 5);
            volume.set(block, v, (i % 9) as u16);
        }
        let (min, max) = (IVec3::new(-8, 0, -3), IVec3::new(50, 3, 48));
        let mut view = volume.dense_view(block, min, max);
        let check = |view: &super::DenseView<u16>, volume: &Volume<Cube16>| {
            for z in min.z - 2..max.z + 2 {
                for y in -1..4 {
                    for x in min.x - 2..max.x + 2 {
                        let v = IVec3::new(x, y, z);
                        let expect = if view.contains(v) {
                            volume.get(block, v)
                        } else {
                            5
                        };
                        assert_eq!(view.get(v), expect, "{v}");
                    }
                }
            }
        };
        check(&view, &volume);
        assert_eq!(view.refresh(&volume), 0);
        volume.set(block, IVec3::new(20, 1, 20), 42);
        volume.set(block, IVec3::new(21, 1, 20), 43); // same chunk
        volume.set(block, IVec3::new(-100, 0, 0), 1); // outside the box
        assert_eq!(view.refresh(&volume), 1);
        check(&view, &volume);
        volume.remove_chunk(IVec3::new(1, 0, 1));
        assert_eq!(view.refresh(&volume), 1);
        check(&view, &volume);
        assert!(view.set(IVec3::new(0, 0, 0), 9));
        assert!(!view.set(IVec3::new(0, 9, 0), 9));
        let empty = volume.dense_view(block, IVec3::ONE, IVec3::ONE);
        assert!(empty.values().is_empty());
        assert_eq!(empty.get(IVec3::ONE), 5);
    }
}
