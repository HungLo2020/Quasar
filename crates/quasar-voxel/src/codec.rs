//! Deterministic binary encodings for chunk deltas, edit batches and changes.
//!
//! Encodings depend only on voxel values and order (never on in-memory
//! storage choices), so the same content always produces the same bytes on
//! every platform. They suit replication and replay; games with their own
//! save formats can ignore them.
use crate::chunk::{ChannelData, Chunk, RawKind, Schema};
use crate::edit::{Brush, BrushOp, BrushShape, Change, EditBatch, EditOp};
use crate::layout::{ChunkLayout, IndexOrder};
use crate::storage::Storage;
use quasar_math::{IVec3, Vec3};

const CHUNK_MAGIC: &[u8; 4] = b"QVC1";
const BATCH_MAGIC: &[u8; 4] = b"QVE1";
const CHANGES_MAGIC: &[u8; 4] = b"QVD1";

/// Decoding failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodecError {
    BadMagic,
    Truncated,
    LayoutMismatch,
    SchemaMismatch,
    Invalid(&'static str),
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "voxel codec: {self:?}")
    }
}
impl std::error::Error for CodecError {}

struct Writer(Vec<u8>);
impl Writer {
    fn u(&mut self, mut v: u64) {
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                self.0.push(byte);
                return;
            }
            self.0.push(byte | 0x80);
        }
    }
    fn i(&mut self, v: i64) {
        self.u(((v << 1) ^ (v >> 63)) as u64);
    }
    fn f(&mut self, v: f32) {
        self.0.extend(v.to_le_bytes());
    }
    fn ivec(&mut self, v: IVec3) {
        self.i(i64::from(v.x));
        self.i(i64::from(v.y));
        self.i(i64::from(v.z));
    }
    fn vec(&mut self, v: Vec3) {
        self.f(v.x);
        self.f(v.y);
        self.f(v.z);
    }
}

struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, CodecError> {
        let (&b, rest) = self.0.split_first().ok_or(CodecError::Truncated)?;
        self.0 = rest;
        Ok(b)
    }
    fn u(&mut self) -> Result<u64, CodecError> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.byte()?;
            v |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(CodecError::Invalid("varint too long"))
    }
    fn i(&mut self) -> Result<i64, CodecError> {
        let v = self.u()?;
        Ok(((v >> 1) as i64) ^ -((v & 1) as i64))
    }
    fn i32(&mut self) -> Result<i32, CodecError> {
        i32::try_from(self.i()?).map_err(|_| CodecError::Invalid("i32 out of range"))
    }
    fn u32(&mut self) -> Result<u32, CodecError> {
        u32::try_from(self.u()?).map_err(|_| CodecError::Invalid("u32 out of range"))
    }
    fn f(&mut self) -> Result<f32, CodecError> {
        if self.0.len() < 4 {
            return Err(CodecError::Truncated);
        }
        let (bytes, rest) = self.0.split_at(4);
        self.0 = rest;
        Ok(f32::from_le_bytes(bytes.try_into().unwrap()))
    }
    fn ivec(&mut self) -> Result<IVec3, CodecError> {
        Ok(IVec3::new(self.i32()?, self.i32()?, self.i32()?))
    }
    fn vec(&mut self) -> Result<Vec3, CodecError> {
        Ok(Vec3::new(self.f()?, self.f()?, self.f()?))
    }
    fn magic(&mut self, magic: &[u8; 4]) -> Result<(), CodecError> {
        for &m in magic {
            if self.byte().map_err(|_| CodecError::BadMagic)? != m {
                return Err(CodecError::BadMagic);
            }
        }
        Ok(())
    }
}

fn raw_tag(kind: RawKind) -> u8 {
    match kind {
        RawKind::U8 => 0,
        RawKind::U16 => 1,
        RawKind::U32 => 2,
    }
}

/// Encodes a chunk: per channel, uniform values directly, otherwise a
/// first-use palette plus run lengths in index order. Derived channels are
/// skipped unless `include_derived`.
pub fn encode_chunk<L: ChunkLayout>(
    schema: &Schema,
    chunk: &Chunk<L>,
    include_derived: bool,
) -> Vec<u8> {
    let mut w = Writer(Vec::with_capacity(64));
    w.0.extend(CHUNK_MAGIC);
    w.u(u64::from(L::LOG2));
    for axis in <L::Order as IndexOrder>::AXES {
        w.u(axis as u64);
    }
    w.u(schema.len() as u64);
    for (index, def) in schema.channels().iter().enumerate() {
        w.0.push(raw_tag(def.raw));
        if def.derived && !include_derived {
            w.0.push(2);
            continue;
        }
        let data = chunk.channel_data(index);
        if let Some(value) = data.uniform_u32() {
            w.0.push(0);
            w.u(u64::from(value));
            continue;
        }
        w.0.push(1);
        let mut palette: Vec<u32> = Vec::new();
        let mut runs: Vec<(u32, u32)> = Vec::new();
        data.for_each_u32(L::VOLUME, |_, value| {
            let slot = match palette.iter().position(|p| *p == value) {
                Some(slot) => slot as u32,
                None => {
                    palette.push(value);
                    (palette.len() - 1) as u32
                }
            };
            match runs.last_mut() {
                Some((s, n)) if *s == slot => *n += 1,
                _ => runs.push((slot, 1)),
            }
        });
        w.u(palette.len() as u64);
        for p in &palette {
            w.u(u64::from(*p));
        }
        w.u(runs.len() as u64);
        for (slot, n) in runs {
            w.u(u64::from(slot));
            w.u(u64::from(n));
        }
    }
    w.0
}

/// Decodes [`encode_chunk`] output for the same layout and schema shape.
/// The chunk is marked edited when any non-derived channel differs from its
/// default.
pub fn decode_chunk<L: ChunkLayout>(schema: &Schema, bytes: &[u8]) -> Result<Chunk<L>, CodecError> {
    let mut r = Reader(bytes);
    r.magic(CHUNK_MAGIC)?;
    if r.u()? != u64::from(L::LOG2) {
        return Err(CodecError::LayoutMismatch);
    }
    for axis in <L::Order as IndexOrder>::AXES {
        if r.u()? != axis as u64 {
            return Err(CodecError::LayoutMismatch);
        }
    }
    if r.u()? as usize != schema.len() {
        return Err(CodecError::SchemaMismatch);
    }
    let mut chunk = Chunk::<L>::new(schema);
    let mut edited = false;
    for (index, def) in schema.channels().iter().enumerate() {
        if r.byte()? != raw_tag(def.raw) {
            return Err(CodecError::SchemaMismatch);
        }
        match r.byte()? {
            2 => {}
            0 => {
                let value = r.u32()?;
                edited |= !def.derived && value != def.default;
                chunk.data_mut()[index] = uniform_data(def.raw, value);
            }
            1 => {
                let palette_len = r.u()? as usize;
                if palette_len > L::VOLUME {
                    return Err(CodecError::Invalid("palette larger than chunk"));
                }
                let palette = (0..palette_len)
                    .map(|_| r.u32())
                    .collect::<Result<Vec<_>, _>>()?;
                let runs = r.u()? as usize;
                let mut values = Vec::with_capacity(L::VOLUME);
                for _ in 0..runs {
                    let slot = r.u()? as usize;
                    let n = r.u()? as usize;
                    let value = *palette
                        .get(slot)
                        .ok_or(CodecError::Invalid("palette index"))?;
                    if values.len() + n > L::VOLUME {
                        return Err(CodecError::Invalid("runs exceed chunk volume"));
                    }
                    values.extend(std::iter::repeat_n(value, n));
                }
                if values.len() != L::VOLUME {
                    return Err(CodecError::Invalid("runs do not cover the chunk"));
                }
                edited |= !def.derived && values.iter().any(|v| *v != def.default);
                let data = &mut chunk.data_mut()[index];
                for (i, v) in values.iter().enumerate() {
                    if *v != def.default {
                        data.set_u32(L::VOLUME, i, *v, def);
                    }
                }
            }
            _ => return Err(CodecError::Invalid("storage tag")),
        }
    }
    if !r.0.is_empty() {
        return Err(CodecError::Invalid("trailing bytes"));
    }
    if edited {
        chunk.mark_edited();
    }
    Ok(chunk)
}

fn uniform_data(raw: RawKind, value: u32) -> ChannelData {
    match raw {
        RawKind::U8 => ChannelData::U8(Storage::Uniform(value as u8)),
        RawKind::U16 => ChannelData::U16(Storage::Uniform(value as u16)),
        RawKind::U32 => ChannelData::U32(Storage::Uniform(value)),
    }
}

/// Encodes an edit batch.
pub fn encode_batch(batch: &EditBatch) -> Vec<u8> {
    let mut w = Writer(Vec::new());
    w.0.extend(BATCH_MAGIC);
    w.u(batch.ops.len() as u64);
    for op in &batch.ops {
        match op {
            EditOp::Set {
                channel,
                voxel,
                value,
            } => {
                w.0.push(0);
                w.u(u64::from(*channel));
                w.ivec(*voxel);
                w.u(u64::from(*value));
            }
            EditOp::Fill {
                channel,
                min,
                max,
                value,
            } => {
                w.0.push(1);
                w.u(u64::from(*channel));
                w.ivec(*min);
                w.ivec(*max);
                w.u(u64::from(*value));
            }
            EditOp::Brush(brush) => {
                w.0.push(2);
                match brush.shape {
                    BrushShape::Sphere { center, radius } => {
                        w.0.push(0);
                        w.vec(center);
                        w.f(radius);
                    }
                    BrushShape::Box { min, max } => {
                        w.0.push(1);
                        w.vec(min);
                        w.vec(max);
                    }
                    BrushShape::Capsule { a, b, radius } => {
                        w.0.push(2);
                        w.vec(a);
                        w.vec(b);
                        w.f(radius);
                    }
                }
                w.0.push(match brush.op {
                    BrushOp::Add => 0,
                    BrushOp::Subtract => 1,
                    BrushOp::Paint => 2,
                });
                w.u(u64::from(brush.density));
                match brush.material {
                    None => w.0.push(0),
                    Some((channel, value)) => {
                        w.0.push(1);
                        w.u(u64::from(channel));
                        w.u(u64::from(value));
                    }
                }
            }
        }
    }
    w.0
}

pub fn decode_batch(bytes: &[u8]) -> Result<EditBatch, CodecError> {
    let mut r = Reader(bytes);
    r.magic(BATCH_MAGIC)?;
    let count = r.u()? as usize;
    let mut ops = Vec::with_capacity(count.min(1 << 16));
    let channel = |r: &mut Reader<'_>| -> Result<u16, CodecError> {
        u16::try_from(r.u()?).map_err(|_| CodecError::Invalid("channel"))
    };
    for _ in 0..count {
        ops.push(match r.byte()? {
            0 => EditOp::Set {
                channel: channel(&mut r)?,
                voxel: r.ivec()?,
                value: r.u32()?,
            },
            1 => EditOp::Fill {
                channel: channel(&mut r)?,
                min: r.ivec()?,
                max: r.ivec()?,
                value: r.u32()?,
            },
            2 => {
                let shape = match r.byte()? {
                    0 => BrushShape::Sphere {
                        center: r.vec()?,
                        radius: r.f()?,
                    },
                    1 => BrushShape::Box {
                        min: r.vec()?,
                        max: r.vec()?,
                    },
                    2 => BrushShape::Capsule {
                        a: r.vec()?,
                        b: r.vec()?,
                        radius: r.f()?,
                    },
                    _ => return Err(CodecError::Invalid("brush shape")),
                };
                let op = match r.byte()? {
                    0 => BrushOp::Add,
                    1 => BrushOp::Subtract,
                    2 => BrushOp::Paint,
                    _ => return Err(CodecError::Invalid("brush op")),
                };
                let density = channel(&mut r)?;
                let material = match r.byte()? {
                    0 => None,
                    1 => Some((channel(&mut r)?, r.u32()?)),
                    _ => return Err(CodecError::Invalid("brush material")),
                };
                EditOp::Brush(Brush {
                    shape,
                    op,
                    density,
                    material,
                })
            }
            _ => return Err(CodecError::Invalid("edit op")),
        });
    }
    if !r.0.is_empty() {
        return Err(CodecError::Invalid("trailing bytes"));
    }
    Ok(EditBatch { ops })
}

/// Encodes change events (e.g. for replication of authoritative results).
pub fn encode_changes(changes: &[Change]) -> Vec<u8> {
    let mut w = Writer(Vec::new());
    w.0.extend(CHANGES_MAGIC);
    w.u(changes.len() as u64);
    for c in changes {
        w.u(u64::from(c.channel));
        w.ivec(c.voxel);
        w.u(u64::from(c.old));
        w.u(u64::from(c.new));
    }
    w.0
}

pub fn decode_changes(bytes: &[u8]) -> Result<Vec<Change>, CodecError> {
    let mut r = Reader(bytes);
    r.magic(CHANGES_MAGIC)?;
    let count = r.u()? as usize;
    let mut out = Vec::with_capacity(count.min(1 << 16));
    for _ in 0..count {
        out.push(Change {
            channel: u16::try_from(r.u()?).map_err(|_| CodecError::Invalid("channel"))?,
            voxel: r.ivec()?,
            old: r.u32()?,
            new: r.u32()?,
        });
    }
    if !r.0.is_empty() {
        return Err(CodecError::Invalid("trailing bytes"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::EditOptions;
    use crate::layout::{Cube16, Cube32};
    use crate::storage::StorageKind;
    use crate::volume::Volume;

    #[test]
    fn chunks_round_trip_deterministically() {
        let mut schema = Schema::new();
        let block = schema.channel("block", StorageKind::Paletted, 0u16);
        let density = schema.channel("density", StorageKind::Dense, -127i8);
        let light = schema.derived_channel("light", StorageKind::Nibble, 0u8);
        let mut volume = Volume::<Cube16>::new(schema.clone());
        let mut rng = quasar_core::Rng::seed_from_u64(1);
        for _ in 0..500 {
            let p = IVec3::new(
                rng.range_i32(0..16),
                rng.range_i32(0..16),
                rng.range_i32(0..16),
            );
            volume.set(block, p, rng.range_u32(0..40) as u16);
            volume.set(density, p, rng.range_i32(-127..127) as i8);
            volume.set(light, p, rng.range_u32(0..16) as u8);
        }
        let chunk = volume.chunk(IVec3::ZERO).unwrap();
        let bytes = encode_chunk(&schema, chunk, true);
        let decoded = decode_chunk::<Cube16>(&schema, &bytes).unwrap();
        assert!(decoded.is_edited());
        for i in 0..Cube16::VOLUME {
            assert_eq!(decoded.get_index(block, i), chunk.get_index(block, i));
            assert_eq!(decoded.get_index(density, i), chunk.get_index(density, i));
            assert_eq!(decoded.get_index(light, i), chunk.get_index(light, i));
        }
        assert_eq!(encode_chunk(&schema, &decoded, true), bytes, "stable bytes");
        let mut compacted = chunk.clone();
        compacted.compact(&schema);
        assert_eq!(
            encode_chunk(&schema, &compacted, true),
            bytes,
            "independent of storage form"
        );
        let delta = encode_chunk(&schema, chunk, false);
        let without_light = decode_chunk::<Cube16>(&schema, &delta).unwrap();
        assert_eq!(without_light.storage(light).uniform(), Some(0));
        assert_eq!(
            decode_chunk::<Cube32>(&schema, &bytes).unwrap_err(),
            CodecError::LayoutMismatch
        );
        assert!(decode_chunk::<Cube16>(&schema, &bytes[..bytes.len() - 1]).is_err());
        assert_eq!(
            decode_chunk::<Cube16>(&schema, b"nope").unwrap_err(),
            CodecError::BadMagic
        );
    }

    #[test]
    fn batches_and_changes_round_trip_and_replay_identically() {
        let mut schema = Schema::new();
        let block = schema.channel("block", StorageKind::Paletted, 0u16);
        let density = schema.channel("density", StorageKind::Dense, -127i8);
        let mut batch = EditBatch::new();
        batch
            .set(block, IVec3::new(-5, 3, 2), 9)
            .fill(block, IVec3::ZERO, IVec3::new(2, 0, 1), 4);
        batch.brush(
            Brush::new(
                BrushShape::Sphere {
                    center: Vec3::splat(4.0),
                    radius: 3.0,
                },
                BrushOp::Add,
                density,
            )
            .with_material(block, 7u16),
        );
        batch.brush(Brush::new(
            BrushShape::Capsule {
                a: Vec3::ZERO,
                b: Vec3::X * 5.0,
                radius: 1.5,
            },
            BrushOp::Subtract,
            density,
        ));
        let bytes = encode_batch(&batch);
        let decoded = decode_batch(&bytes).unwrap();
        assert_eq!(decoded, batch);
        let mut a = Volume::<Cube16>::new(schema.clone());
        let mut b = Volume::<Cube16>::new(schema);
        let ra = a.apply(&batch, EditOptions::default());
        let rb = b.apply(&decoded, EditOptions::default());
        assert_eq!(ra, rb);
        let changes = decode_changes(&encode_changes(&ra.changes)).unwrap();
        assert_eq!(changes, ra.changes);
        assert!(decode_batch(&bytes[..bytes.len() - 2]).is_err());
    }
}
