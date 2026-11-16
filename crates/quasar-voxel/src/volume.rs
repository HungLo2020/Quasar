//! Chunk maps, procedural sources with edit deltas, and padded snapshots.
use crate::chunk::{ChannelId, Chunk, Schema};
use crate::layout::ChunkLayout;
use crate::storage::{Raw, Value};
use quasar_math::{IVec3, UVec3};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;

/// Small, fast, deterministic hasher for integer keys (FxHash). Map lookups
/// never affect outputs; ordered iteration goes through sorted keys.
#[derive(Default, Clone, Copy)]
pub struct FxHasher(u64);
impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.add(u64::from(*b));
        }
    }
    #[inline]
    fn write_u32(&mut self, v: u32) {
        self.add(u64::from(v));
    }
    #[inline]
    fn write_i32(&mut self, v: i32) {
        self.add(u64::from(v as u32));
    }
    #[inline]
    fn write_u64(&mut self, v: u64) {
        self.add(v);
    }
    #[inline]
    fn write_usize(&mut self, v: usize) {
        self.add(v as u64);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}
impl FxHasher {
    #[inline]
    fn add(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}
/// `HashMap` keyed with [`FxHasher`].
pub type FxHashMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;

/// Fills new chunks procedurally. Implementations must be deterministic for
/// a given coordinate and LOD (seed them with `quasar_core::Rng`).
pub trait VoxelSource<L: ChunkLayout>: Send + Sync {
    /// Fills `chunk` (already at schema defaults) for chunk coordinate
    /// `coord`. `lod` is 0 for full resolution.
    fn generate(&self, schema: &Schema, coord: IVec3, lod: u32, chunk: &mut Chunk<L>);
}

/// Cached chunk slot of a [`Volume`] for repeated nearby access.
#[derive(Clone, Copy, Debug, Default)]
pub struct Cursor {
    key: Option<(IVec3, u32)>,
}

/// A chunk map over one [`Schema`].
#[derive(Clone)]
pub struct Volume<L: ChunkLayout> {
    schema: Arc<Schema>,
    slots: Vec<Option<(IVec3, Chunk<L>)>>,
    free: Vec<u32>,
    index: FxHashMap<IVec3, u32>,
    source: Option<Arc<dyn VoxelSource<L>>>,
    defaults: Box<[u32]>,
}

impl<L: ChunkLayout> std::fmt::Debug for Volume<L> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Volume")
            .field("chunks", &self.index.len())
            .field("edge", &L::EDGE)
            .finish()
    }
}

impl<L: ChunkLayout> Volume<L> {
    pub fn new(schema: Schema) -> Self {
        Self::with_schema(Arc::new(schema))
    }
    pub fn with_schema(schema: Arc<Schema>) -> Self {
        let defaults = schema.channels().iter().map(|c| c.default).collect();
        Self {
            schema,
            slots: Vec::new(),
            free: Vec::new(),
            index: FxHashMap::default(),
            source: None,
            defaults,
        }
    }
    /// Sets the procedural source used by [`Self::ensure_chunk`].
    pub fn with_source(mut self, source: Arc<dyn VoxelSource<L>>) -> Self {
        self.source = Some(source);
        self
    }
    pub fn set_source(&mut self, source: Option<Arc<dyn VoxelSource<L>>>) {
        self.source = source;
    }
    pub fn source(&self) -> Option<&Arc<dyn VoxelSource<L>>> {
        self.source.as_ref()
    }
    pub fn schema(&self) -> &Arc<Schema> {
        &self.schema
    }
    pub fn chunk_count(&self) -> usize {
        self.index.len()
    }
    pub fn contains(&self, coord: IVec3) -> bool {
        self.index.contains_key(&coord)
    }
    #[inline]
    pub fn chunk(&self, coord: IVec3) -> Option<&Chunk<L>> {
        let slot = *self.index.get(&coord)?;
        self.slots[slot as usize].as_ref().map(|(_, c)| c)
    }
    /// [`Self::contains`] reusing a cursor's cached chunk.
    #[inline]
    pub fn contains_with(&self, cursor: &mut Cursor, coord: IVec3) -> bool {
        self.chunk_cached(cursor, coord).is_some()
    }
    #[inline]
    pub fn chunk_mut(&mut self, coord: IVec3) -> Option<&mut Chunk<L>> {
        let slot = *self.index.get(&coord)?;
        self.slots[slot as usize].as_mut().map(|(_, c)| c)
    }
    #[inline]
    fn chunk_cached(&self, cursor: &mut Cursor, coord: IVec3) -> Option<&Chunk<L>> {
        if let Some((key, slot)) = cursor.key
            && key == coord
            && let Some((k, chunk)) = self.slots.get(slot as usize).and_then(Option::as_ref)
            && *k == coord
        {
            return Some(chunk);
        }
        let slot = *self.index.get(&coord)?;
        cursor.key = Some((coord, slot));
        self.slots[slot as usize].as_ref().map(|(_, c)| c)
    }
    /// Inserts or replaces a chunk; returns the previous one.
    pub fn insert_chunk(&mut self, coord: IVec3, chunk: Chunk<L>) -> Option<Chunk<L>> {
        if let Some(&slot) = self.index.get(&coord) {
            let previous = self.slots[slot as usize].replace((coord, chunk));
            return previous.map(|(_, c)| c);
        }
        let slot = match self.free.pop() {
            Some(slot) => {
                self.slots[slot as usize] = Some((coord, chunk));
                slot
            }
            None => {
                self.slots.push(Some((coord, chunk)));
                (self.slots.len() - 1) as u32
            }
        };
        self.index.insert(coord, slot);
        None
    }
    pub fn remove_chunk(&mut self, coord: IVec3) -> Option<Chunk<L>> {
        let slot = self.index.remove(&coord)?;
        self.free.push(slot);
        self.slots[slot as usize].take().map(|(_, c)| c)
    }
    /// The chunk at `coord`, generating it from the source (or defaults)
    /// when absent. Returns whether it was created.
    pub fn ensure_chunk(&mut self, coord: IVec3) -> bool {
        if self.contains(coord) {
            return false;
        }
        let chunk = self.generate(coord, 0);
        self.insert_chunk(coord, chunk);
        true
    }
    /// Builds (without inserting) the source's chunk for `coord` at `lod`.
    pub fn generate(&self, coord: IVec3, lod: u32) -> Chunk<L> {
        let mut chunk = Chunk::new(&self.schema);
        if let Some(source) = &self.source {
            source.generate(&self.schema, coord, lod, &mut chunk);
        }
        chunk.clear_edited();
        chunk
    }
    /// Generates every missing chunk in `coords` on the job pool and inserts
    /// them in the given order (deterministic regardless of scheduling).
    #[cfg(feature = "jobs")]
    pub fn ensure_chunks_parallel(
        &mut self,
        pool: &quasar_jobs::JobPool,
        coords: &[IVec3],
    ) -> Result<usize, quasar_jobs::JobError> {
        let mut handles = Vec::new();
        for &coord in coords {
            if self.contains(coord) {
                continue;
            }
            let schema = self.schema.clone();
            let source = self.source.clone();
            handles.push((
                coord,
                pool.spawn(move |_| {
                    let mut chunk = Chunk::<L>::new(&schema);
                    if let Some(source) = source {
                        source.generate(&schema, coord, 0, &mut chunk);
                    }
                    chunk.clear_edited();
                    Ok::<_, quasar_jobs::JobError>(chunk)
                })?,
            ));
        }
        let created = handles.len();
        for (coord, handle) in handles {
            let chunk = handle.wait()?;
            self.insert_chunk(coord, chunk);
        }
        Ok(created)
    }
    /// Loaded chunk coordinates in sorted (deterministic) order.
    pub fn chunk_coords(&self) -> Vec<IVec3> {
        let mut coords: Vec<IVec3> = self.index.keys().copied().collect();
        coords.sort_by_key(|c| (c.x, c.y, c.z));
        coords
    }
    /// Coordinates of chunks whose non-derived channels differ from the
    /// source: the edit delta to persist or replicate (sorted).
    pub fn edited_chunks(&self) -> Vec<IVec3> {
        let mut coords: Vec<IVec3> = self
            .slots
            .iter()
            .flatten()
            .filter(|(_, c)| c.is_edited())
            .map(|(k, _)| *k)
            .collect();
        coords.sort_by_key(|c| (c.x, c.y, c.z));
        coords
    }
    /// Drops unedited chunks for which `keep` is false (they regenerate
    /// from the source on demand). Returns how many were evicted.
    pub fn evict_generated(&mut self, mut keep: impl FnMut(IVec3) -> bool) -> usize {
        let doomed: Vec<IVec3> = self
            .chunk_coords()
            .into_iter()
            .filter(|c| !self.chunk(*c).is_some_and(Chunk::is_edited) && !keep(*c))
            .collect();
        for coord in &doomed {
            self.remove_chunk(*coord);
        }
        doomed.len()
    }
    /// Reads a voxel; absent chunks read as the channel default.
    #[inline]
    pub fn get<T: Value>(&self, id: ChannelId<T>, voxel: IVec3) -> T {
        match self.chunk(L::chunk_of(voxel)) {
            Some(chunk) => chunk.get(id, L::local_of(voxel)),
            None => T::from_raw(<T::Raw as Raw>::from_u32(self.defaults[id.index()])),
        }
    }
    /// [`Self::get`] reusing a cursor's cached chunk.
    #[inline]
    pub fn get_with<T: Value>(&self, cursor: &mut Cursor, id: ChannelId<T>, voxel: IVec3) -> T {
        match self.chunk_cached(cursor, L::chunk_of(voxel)) {
            Some(chunk) => chunk.get(id, L::local_of(voxel)),
            None => T::from_raw(<T::Raw as Raw>::from_u32(self.defaults[id.index()])),
        }
    }
    /// Raw read of any channel by index.
    pub fn get_raw(&self, channel: usize, voxel: IVec3) -> u32 {
        match self.chunk(L::chunk_of(voxel)) {
            Some(chunk) => chunk
                .channel_data(channel)
                .get_u32(L::index_of(L::local_of(voxel))),
            None => self.defaults[channel],
        }
    }
    /// Default raw value of a channel.
    pub fn default_raw(&self, channel: usize) -> u32 {
        self.defaults[channel]
    }
    /// Writes a voxel (creating the chunk from the source if needed) and
    /// returns the previous value. Non-derived channel changes mark the
    /// chunk as edited.
    pub fn set<T: Value>(&mut self, id: ChannelId<T>, voxel: IVec3, value: T) -> T {
        T::from_raw(<T::Raw as Raw>::from_u32(self.set_raw(
            id.index(),
            voxel,
            value.to_raw().to_u32(),
        )))
    }
    /// Raw write of any channel.
    pub fn set_raw(&mut self, channel: usize, voxel: IVec3, value: u32) -> u32 {
        let coord = L::chunk_of(voxel);
        self.ensure_chunk(coord);
        let derived = self.schema.channels()[channel].derived;
        let schema = self.schema.clone();
        let chunk = self.chunk_mut(coord).expect("ensured");
        let old = chunk.set_raw(&schema, channel, L::index_of(L::local_of(voxel)), value);
        if old != value && !derived {
            chunk.mark_edited();
        }
        old
    }
    /// Writes a voxel only if its chunk is loaded (no generation). Returns
    /// the previous value, or `None` for an absent chunk.
    pub fn set_loaded<T: Value>(&mut self, id: ChannelId<T>, voxel: IVec3, value: T) -> Option<T> {
        if !self.contains(L::chunk_of(voxel)) {
            return None;
        }
        Some(self.set(id, voxel, value))
    }
    /// Writes a voxel of a loaded chunk through a cursor (no generation);
    /// `None` when the chunk is absent. The fast path for propagation.
    #[inline]
    pub fn set_loaded_with<T: Value>(
        &mut self,
        cursor: &mut Cursor,
        id: ChannelId<T>,
        voxel: IVec3,
        value: T,
    ) -> Option<T> {
        let coord = L::chunk_of(voxel);
        let slot = match cursor.key {
            Some((key, slot))
                if key == coord
                    && self
                        .slots
                        .get(slot as usize)
                        .and_then(Option::as_ref)
                        .is_some_and(|(k, _)| *k == coord) =>
            {
                slot
            }
            _ => {
                let slot = *self.index.get(&coord)?;
                cursor.key = Some((coord, slot));
                slot
            }
        };
        let derived = self.schema.channels()[id.index()].derived;
        let schema = &self.schema;
        let (_, chunk) = self.slots[slot as usize].as_mut()?;
        let old = chunk.set(schema, id, L::local_of(voxel), value);
        if old != value && !derived {
            chunk.mark_edited();
        }
        Some(old)
    }
    /// Approximate heap bytes used by loaded chunks.
    pub fn memory_bytes(&self) -> usize {
        self.slots
            .iter()
            .flatten()
            .map(|(_, c)| c.memory_bytes())
            .sum::<usize>()
            + self.index.capacity() * 24
    }
    /// Copies a padded region around chunk `coord` into `out`. See [`Snapshot`].
    pub fn snapshot<T: Value>(
        &self,
        id: ChannelId<T>,
        coord: IVec3,
        pad: u32,
        out: &mut Snapshot<T::Raw>,
    ) {
        out.fill_from(self, id, coord, pad);
    }
    /// Chunk-local view used by extractors that read several channels.
    pub fn neighbourhood(&self, coord: IVec3) -> [Option<&Chunk<L>>; 27] {
        std::array::from_fn(|i| {
            let d = IVec3::new(i as i32 % 3 - 1, (i as i32 / 3) % 3 - 1, i as i32 / 9 - 1);
            self.chunk(coord + d)
        })
    }
}

/// A chunk plus a border of `pad` voxels from its neighbours, decoded into a
/// flat array (`x` fastest, then `z`, then `y`). Reusable: buffers are kept
/// between fills, so steady-state extraction does not allocate.
#[derive(Clone, Debug, Default)]
pub struct Snapshot<R: Raw> {
    edge: usize,
    pad: u32,
    coord: IVec3,
    data: Vec<R>,
    scratch: Vec<R>,
}

impl<R: Raw> Snapshot<R> {
    pub fn new() -> Self {
        Self {
            edge: 0,
            pad: 0,
            coord: IVec3::ZERO,
            data: Vec::new(),
            scratch: Vec::new(),
        }
    }
    /// Padded edge (`chunk edge + 2 * pad`).
    pub fn edge(&self) -> usize {
        self.edge
    }
    pub fn pad(&self) -> u32 {
        self.pad
    }
    pub fn coord(&self) -> IVec3 {
        self.coord
    }
    pub fn data(&self) -> &[R] {
        &self.data
    }
    /// Replaces the contents with externally produced data (e.g. LOD samples),
    /// laid out like [`Self::index`]. `edge` includes the padding.
    pub fn fill_with(&mut self, edge: usize, pad: u32, coord: IVec3, data: Vec<R>) {
        assert_eq!(
            data.len(),
            edge * edge * edge,
            "snapshot data must be edge³"
        );
        assert!(edge > 2 * pad as usize, "edge must exceed the padding");
        self.edge = edge;
        self.pad = pad;
        self.coord = coord;
        self.data = data;
    }
    /// Flat index of padded coordinates (`0..edge` each).
    #[inline(always)]
    pub fn index(&self, x: usize, y: usize, z: usize) -> usize {
        x + self.edge * (z + self.edge * y)
    }
    /// Value at chunk-local coordinates (`-pad..chunk_edge + pad`).
    #[inline(always)]
    pub fn at(&self, x: i32, y: i32, z: i32) -> R {
        let p = self.pad as i32;
        self.data[self.index((x + p) as usize, (y + p) as usize, (z + p) as usize)]
    }
    fn fill_from<L: ChunkLayout, T: Value<Raw = R>>(
        &mut self,
        volume: &Volume<L>,
        id: ChannelId<T>,
        coord: IVec3,
        pad: u32,
    ) {
        assert!(pad as usize <= L::EDGE, "pad larger than a chunk");
        let edge = L::EDGE + 2 * pad as usize;
        self.edge = edge;
        self.pad = pad;
        self.coord = coord;
        self.data.resize(edge * edge * edge, R::default());
        let default = R::from_u32(volume.default_raw(id.index()));
        let p = pad as i32;
        let n = L::EDGE as i32;
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    // Overlap of this neighbour with the padded box, in
                    // chunk-local coordinates of the centre chunk.
                    let range = |d: i32| -> (i32, i32) {
                        match d {
                            -1 => (-p, 0),
                            0 => (0, n),
                            _ => (n, n + p),
                        }
                    };
                    let (x0, x1) = range(dx);
                    let (y0, y1) = range(dy);
                    let (z0, z1) = range(dz);
                    if x0 == x1 || y0 == y1 || z0 == z1 {
                        continue;
                    }
                    let neighbour = volume.chunk(coord + IVec3::new(dx, dy, dz));
                    let uniform = match neighbour {
                        None => Some(default),
                        Some(chunk) => chunk.storage(id).uniform(),
                    };
                    let offset = IVec3::new(dx, dy, dz) * n;
                    if let Some(value) = uniform {
                        for y in y0..y1 {
                            for z in z0..z1 {
                                let start = self.index(
                                    (x0 + p) as usize,
                                    (y + p) as usize,
                                    (z + p) as usize,
                                );
                                self.data[start..start + (x1 - x0) as usize].fill(value);
                            }
                        }
                        continue;
                    }
                    let chunk = neighbour.expect("non-uniform implies present");
                    let storage = chunk.storage(id);
                    let contiguous_x = <L::Order as crate::layout::IndexOrder>::AXES[0] == 0;
                    if dx == 0 && dy == 0 && dz == 0 || !contiguous_x {
                        self.scratch.resize(L::VOLUME, R::default());
                        storage.decode_into(&mut self.scratch);
                        for y in y0..y1 {
                            for z in z0..z1 {
                                let ly = (y - offset.y) as u32;
                                let lz = (z - offset.z) as u32;
                                let dst = self.index(
                                    (x0 + p) as usize,
                                    (y + p) as usize,
                                    (z + p) as usize,
                                );
                                if contiguous_x {
                                    let src = L::index((x0 - offset.x) as u32, ly, lz);
                                    let len = (x1 - x0) as usize;
                                    self.data[dst..dst + len]
                                        .copy_from_slice(&self.scratch[src..src + len]);
                                } else {
                                    for x in x0..x1 {
                                        let src = L::index((x - offset.x) as u32, ly, lz);
                                        self.data[dst + (x - x0) as usize] = self.scratch[src];
                                    }
                                }
                            }
                        }
                    } else {
                        // Thin border slabs: read only the needed voxels.
                        for y in y0..y1 {
                            for z in z0..z1 {
                                let dst = self.index(
                                    (x0 + p) as usize,
                                    (y + p) as usize,
                                    (z + p) as usize,
                                );
                                for x in x0..x1 {
                                    let local = UVec3::new(
                                        (x - offset.x) as u32,
                                        (y - offset.y) as u32,
                                        (z - offset.z) as u32,
                                    );
                                    self.data[dst + (x - x0) as usize] =
                                        storage.get(L::index_of(local));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Cube8, Cube16};
    use crate::storage::StorageKind;

    struct Stripes;
    impl VoxelSource<Cube8> for Stripes {
        fn generate(&self, schema: &Schema, coord: IVec3, _lod: u32, chunk: &mut Chunk<Cube8>) {
            let id = schema.find::<u16>("block").unwrap();
            let values: Vec<u16> = (0..Cube8::VOLUME)
                .map(|i| {
                    let p = Cube8::origin(coord) + Cube8::local(i).as_ivec3();
                    (p.y.rem_euclid(3) == 0) as u16 * (1 + (p.x & 1) as u16)
                })
                .collect();
            chunk.write_all(schema, id, &values);
        }
    }

    fn schema() -> (Schema, ChannelId<u16>, ChannelId<u8>) {
        let mut schema = Schema::new();
        let block = schema.channel("block", StorageKind::Paletted, 0u16);
        let light = schema.derived_channel("light", StorageKind::Nibble, 0u8);
        (schema, block, light)
    }

    #[test]
    fn volumes_read_defaults_write_and_track_edits() {
        let (schema, block, light) = schema();
        let mut volume = Volume::<Cube16>::new(schema);
        assert_eq!(volume.get(block, IVec3::new(5, -3, 9)), 0);
        assert_eq!(volume.set(block, IVec3::new(5, -3, 9), 7), 0);
        assert_eq!(volume.get(block, IVec3::new(5, -3, 9)), 7);
        assert_eq!(volume.chunk_count(), 1);
        assert_eq!(volume.edited_chunks(), vec![IVec3::new(0, -1, 0)]);
        volume.set(light, IVec3::new(40, 0, 0), 9);
        assert_eq!(
            volume.edited_chunks().len(),
            1,
            "derived channels are not deltas"
        );
        assert_eq!(volume.evict_generated(|_| false), 1);
        assert!(volume.set_loaded(block, IVec3::new(100, 0, 0), 1).is_none());
        let mut cursor = Cursor::default();
        assert_eq!(volume.get_with(&mut cursor, block, IVec3::new(5, -3, 9)), 7);
        assert_eq!(volume.get_with(&mut cursor, block, IVec3::new(6, -3, 9)), 0);
        assert!(volume.memory_bytes() > 0);
    }

    #[test]
    fn sources_generate_and_snapshots_match_point_reads() {
        let (schema, block, _) = schema();
        let mut volume = Volume::<Cube8>::new(schema).with_source(Arc::new(Stripes));
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1 {
                    volume.ensure_chunk(IVec3::new(x, y, z));
                }
            }
        }
        // Make the centre and one neighbour non-trivial.
        volume.set(block, IVec3::new(3, 3, 3), 9);
        volume.set(block, IVec3::new(8, 0, 0), 5);
        volume.remove_chunk(IVec3::new(-1, -1, -1));
        assert!(volume.edited_chunks().len() == 2);
        for pad in [1u32, 2] {
            let mut snap = Snapshot::new();
            volume.snapshot(block, IVec3::ZERO, pad, &mut snap);
            assert_eq!(snap.edge(), 8 + 2 * pad as usize);
            let p = pad as i32;
            for y in -p..8 + p {
                for z in -p..8 + p {
                    for x in -p..8 + p {
                        assert_eq!(
                            snap.at(x, y, z),
                            volume.get(block, IVec3::new(x, y, z)),
                            "{x} {y} {z}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn snapshots_handle_non_x_fastest_orders() {
        use crate::layout::{Cube, Yzx};
        let mut schema = Schema::new();
        let block = schema.channel("b", StorageKind::Dense, 0u8);
        let mut volume = Volume::<Cube<3, Yzx>>::new(schema);
        for i in 0..200 {
            let p = IVec3::new(i % 11 - 1, (i * 7) % 10 - 1, (i * 3) % 9 - 1);
            volume.set(block, p, (i % 250) as u8 + 1);
        }
        let mut snap = Snapshot::new();
        volume.snapshot(block, IVec3::ZERO, 1, &mut snap);
        for y in -1..9 {
            for z in -1..9 {
                for x in -1..9 {
                    assert_eq!(snap.at(x, y, z), volume.get(block, IVec3::new(x, y, z)));
                }
            }
        }
    }
}
