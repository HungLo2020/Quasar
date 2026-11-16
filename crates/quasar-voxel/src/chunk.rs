//! Channel schemas and chunks.
//!
//! A [`Schema`] lists a volume's channels: per-voxel attributes, each with
//! its own value type, storage strategy, default and palette policy. Games
//! define the channels they need (block state, light, density, material,
//! orientation, fluid level, ...) and access them through typed
//! [`ChannelId`] handles. Accessors are monomorphised per value type.
use crate::layout::ChunkLayout;
use crate::storage::{FastPalette, PalettePolicy, Raw, Storage, StorageKind, Value};
use quasar_math::UVec3;
use std::marker::PhantomData;
use std::sync::Arc;

/// Raw width of a channel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RawKind {
    U8,
    U16,
    U32,
}

/// One channel's definition.
#[derive(Clone, Debug)]
pub struct ChannelDef {
    pub name: String,
    pub raw: RawKind,
    pub kind: StorageKind,
    /// Raw default value of absent voxels and new chunks.
    pub default: u32,
    pub policy: Arc<dyn PalettePolicy>,
    /// Derived channels (light, caches) are not part of edit deltas.
    pub derived: bool,
}

/// Typed handle to a channel of a [`Schema`].
#[derive(Debug)]
pub struct ChannelId<T> {
    index: u16,
    _value: PhantomData<fn() -> T>,
}
impl<T> Clone for ChannelId<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for ChannelId<T> {}
impl<T> PartialEq for ChannelId<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}
impl<T> Eq for ChannelId<T> {}
impl<T> std::hash::Hash for ChannelId<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.index.hash(state);
    }
}

impl<T> ChannelId<T> {
    pub fn index(self) -> usize {
        usize::from(self.index)
    }
    /// Re-types a raw index. Accessors panic if the raw width does not match.
    pub fn from_index(index: usize) -> Self {
        Self {
            index: index as u16,
            _value: PhantomData,
        }
    }
}

/// Channel list of a volume.
#[derive(Clone, Debug, Default)]
pub struct Schema {
    channels: Vec<ChannelDef>,
}

fn raw_kind<R: Raw>() -> RawKind {
    match R::BITS {
        8 => RawKind::U8,
        16 => RawKind::U16,
        _ => RawKind::U32,
    }
}

impl Schema {
    pub fn new() -> Self {
        Self::default()
    }
    /// Adds a channel with Quasar's fast palette policy.
    pub fn channel<T: Value>(&mut self, name: &str, kind: StorageKind, default: T) -> ChannelId<T> {
        self.channel_with_policy(name, kind, default, Arc::new(FastPalette))
    }
    /// Adds a channel with an explicit palette policy.
    pub fn channel_with_policy<T: Value>(
        &mut self,
        name: &str,
        kind: StorageKind,
        default: T,
        policy: Arc<dyn PalettePolicy>,
    ) -> ChannelId<T> {
        assert!(self.channels.len() < u16::MAX as usize, "too many channels");
        assert!(
            kind != StorageKind::Nibble || <T::Raw as Raw>::BITS == 8,
            "nibble channels must be 8-bit"
        );
        self.channels.push(ChannelDef {
            name: name.to_owned(),
            raw: raw_kind::<T::Raw>(),
            kind,
            default: default.to_raw().to_u32(),
            policy,
            derived: false,
        });
        ChannelId::from_index(self.channels.len() - 1)
    }
    /// Adds a derived channel (excluded from edit deltas; e.g. light).
    pub fn derived_channel<T: Value>(
        &mut self,
        name: &str,
        kind: StorageKind,
        default: T,
    ) -> ChannelId<T> {
        let id = self.channel(name, kind, default);
        self.channels[id.index()].derived = true;
        id
    }
    pub fn channels(&self) -> &[ChannelDef] {
        &self.channels
    }
    pub fn len(&self) -> usize {
        self.channels.len()
    }
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }
    /// Channel by name.
    pub fn find<T: Value>(&self, name: &str) -> Option<ChannelId<T>> {
        let index = self.channels.iter().position(|c| c.name == name)?;
        (self.channels[index].raw == raw_kind::<T::Raw>()).then(|| ChannelId::from_index(index))
    }
    pub fn def<T>(&self, id: ChannelId<T>) -> &ChannelDef {
        &self.channels[id.index()]
    }
    pub(crate) fn empty_data(&self) -> Box<[ChannelData]> {
        self.channels
            .iter()
            .map(|c| match c.raw {
                RawKind::U8 => ChannelData::U8(Storage::Uniform(c.default as u8)),
                RawKind::U16 => ChannelData::U16(Storage::Uniform(c.default as u16)),
                RawKind::U32 => ChannelData::U32(Storage::Uniform(c.default)),
            })
            .collect()
    }
}

/// Type-erased storage of one channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelData {
    U8(Storage<u8>),
    U16(Storage<u16>),
    U32(Storage<u32>),
}

impl ChannelData {
    pub fn get_u32(&self, index: usize) -> u32 {
        match self {
            Self::U8(s) => u32::from(s.get(index)),
            Self::U16(s) => u32::from(s.get(index)),
            Self::U32(s) => s.get(index),
        }
    }
    pub fn uniform_u32(&self) -> Option<u32> {
        match self {
            Self::U8(s) => s.uniform().map(u32::from),
            Self::U16(s) => s.uniform().map(u32::from),
            Self::U32(s) => s.uniform(),
        }
    }
    pub(crate) fn set_u32(
        &mut self,
        len: usize,
        index: usize,
        value: u32,
        def: &ChannelDef,
    ) -> u32 {
        match self {
            Self::U8(s) => u32::from(s.set(len, index, value as u8, def.kind, &*def.policy)),
            Self::U16(s) => u32::from(s.set(len, index, value as u16, def.kind, &*def.policy)),
            Self::U32(s) => s.set(len, index, value, def.kind, &*def.policy),
        }
    }
    pub fn memory_bytes(&self) -> usize {
        match self {
            Self::U8(s) => s.memory_bytes(),
            Self::U16(s) => s.memory_bytes(),
            Self::U32(s) => s.memory_bytes(),
        }
    }
    pub(crate) fn compact(&mut self, len: usize, def: &ChannelDef) {
        match self {
            Self::U8(s) => s.compact(len, def.kind, &*def.policy),
            Self::U16(s) => s.compact(len, def.kind, &*def.policy),
            Self::U32(s) => s.compact(len, def.kind, &*def.policy),
        }
    }
    pub fn for_each_u32(&self, len: usize, mut visit: impl FnMut(usize, u32)) {
        match self {
            Self::U8(s) => s.for_each(len, |i, v| visit(i, u32::from(v))),
            Self::U16(s) => s.for_each(len, |i, v| visit(i, u32::from(v))),
            Self::U32(s) => s.for_each(len, visit),
        }
    }
}

/// One chunk: every channel's storage plus bookkeeping.
#[derive(Clone, Debug)]
pub struct Chunk<L: ChunkLayout> {
    channels: Box<[ChannelData]>,
    revision: u64,
    edited: bool,
    _layout: PhantomData<L>,
}

impl<L: ChunkLayout> Chunk<L> {
    /// A chunk with every channel at its default.
    pub fn new(schema: &Schema) -> Self {
        Self {
            channels: schema.empty_data(),
            revision: 0,
            edited: false,
            _layout: PhantomData,
        }
    }
    /// Increments whenever any voxel changes.
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Whether non-derived channels were edited since creation or the last
    /// [`Self::clear_edited`] (i.e. the chunk differs from its source).
    pub fn is_edited(&self) -> bool {
        self.edited
    }
    pub fn clear_edited(&mut self) {
        self.edited = false;
    }
    pub(crate) fn mark_edited(&mut self) {
        self.edited = true;
    }
    pub(crate) fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn channel_data(&self, index: usize) -> &ChannelData {
        &self.channels[index]
    }
    /// Typed storage of a channel.
    #[inline]
    pub fn storage<T: Value>(&self, id: ChannelId<T>) -> &Storage<T::Raw> {
        <T::Raw as Raw>::data(&self.channels[id.index()]).expect("channel raw width mismatch")
    }
    #[inline]
    pub fn get<T: Value>(&self, id: ChannelId<T>, local: UVec3) -> T {
        T::from_raw(self.storage(id).get(L::index_of(local)))
    }
    #[inline]
    pub fn get_index<T: Value>(&self, id: ChannelId<T>, index: usize) -> T {
        T::from_raw(self.storage(id).get(index))
    }
    /// Sets one voxel; returns the previous value. Does not mark edits;
    /// prefer [`crate::Volume::set`] for tracked changes.
    pub fn set<T: Value>(
        &mut self,
        schema: &Schema,
        id: ChannelId<T>,
        local: UVec3,
        value: T,
    ) -> T {
        self.set_index(schema, id, L::index_of(local), value)
    }
    pub fn set_index<T: Value>(
        &mut self,
        schema: &Schema,
        id: ChannelId<T>,
        index: usize,
        value: T,
    ) -> T {
        let def = &schema.channels()[id.index()];
        let storage = <T::Raw as Raw>::data_mut(&mut self.channels[id.index()])
            .expect("channel raw width mismatch");
        let old = storage.set(L::VOLUME, index, value.to_raw(), def.kind, &*def.policy);
        if old != value.to_raw() {
            self.bump();
        }
        T::from_raw(old)
    }
    pub(crate) fn set_raw(
        &mut self,
        schema: &Schema,
        channel: usize,
        index: usize,
        value: u32,
    ) -> u32 {
        let def = &schema.channels()[channel];
        let old = self.channels[channel].set_u32(L::VOLUME, index, value, def);
        if old != value {
            self.bump();
        }
        old
    }
    /// Replaces a channel from a full array in index order, choosing the
    /// most compact storage (uniform when all values match). The fast path
    /// for generators.
    pub fn write_all<T: Value>(&mut self, schema: &Schema, id: ChannelId<T>, values: &[T]) {
        assert_eq!(
            values.len(),
            L::VOLUME,
            "write_all needs one value per voxel"
        );
        let def = &schema.channels()[id.index()];
        let first = values[0].to_raw();
        let storage = if values.iter().all(|v| v.to_raw() == first) {
            Storage::Uniform(first)
        } else {
            let raw: Vec<T::Raw> = values.iter().map(|v| v.to_raw()).collect();
            let mut storage = match def.kind {
                StorageKind::Dense => Storage::Dense(raw.clone().into_boxed_slice()),
                _ => Storage::Uniform(first),
            };
            if def.kind != StorageKind::Dense {
                for (i, v) in raw.iter().enumerate() {
                    storage.set(L::VOLUME, i, *v, def.kind, &*def.policy);
                }
            }
            storage
        };
        self.channels[id.index()] = <T::Raw as Raw>::wrap(storage);
        self.bump();
    }
    /// Fills a channel with one value.
    pub fn fill<T: Value>(&mut self, id: ChannelId<T>, value: T) {
        self.channels[id.index()] = <T::Raw as Raw>::wrap(Storage::Uniform(value.to_raw()));
        self.bump();
    }
    /// Re-encodes every channel compactly.
    pub fn compact(&mut self, schema: &Schema) {
        for (data, def) in self.channels.iter_mut().zip(schema.channels()) {
            data.compact(L::VOLUME, def);
        }
    }
    pub fn memory_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.channels.len() * std::mem::size_of::<ChannelData>()
            + self
                .channels
                .iter()
                .map(ChannelData::memory_bytes)
                .sum::<usize>()
    }
    pub(crate) fn data_mut(&mut self) -> &mut [ChannelData] {
        &mut self.channels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_chunks_stay_small() {
        assert!(std::mem::size_of::<ChannelData>() <= 32);
        let mut schema = Schema::new();
        schema.channel("block", StorageKind::Paletted, 0u16);
        let chunk = Chunk::<crate::layout::Cube16>::new(&schema);
        assert!(chunk.memory_bytes() <= 64, "{}", chunk.memory_bytes());
    }
}
