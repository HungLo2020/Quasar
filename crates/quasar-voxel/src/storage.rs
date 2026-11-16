//! Per-channel, per-chunk storage strategies and their policies.
//!
//! Every channel of every chunk is one [`Storage`]: uniform (no per-voxel
//! memory), dense, paletted, nibble-packed or sparse. Paletted storage takes a
//! [`PalettePolicy`] that decides bit widths, palette-vs-direct encoding and
//! word packing, so a game can reproduce an external format exactly (for
//! example non-spanning 64-bit words with a minimum width) while Quasar's
//! default favours speed (power-of-two widths).
use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;

/// Raw integer types channels are stored as.
pub trait Raw:
    Copy + Eq + Ord + Hash + Default + Debug + Send + Sync + 'static + sealed::Sealed
{
    const BITS: u32;
    fn to_u32(self) -> u32;
    fn from_u32(value: u32) -> Self;
    #[doc(hidden)]
    fn data(data: &crate::chunk::ChannelData) -> Option<&Storage<Self>>;
    #[doc(hidden)]
    fn data_mut(data: &mut crate::chunk::ChannelData) -> Option<&mut Storage<Self>>;
    #[doc(hidden)]
    fn wrap(storage: Storage<Self>) -> crate::chunk::ChannelData;
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for u8 {}
    impl Sealed for u16 {}
    impl Sealed for u32 {}
}

macro_rules! raw {
    ($t:ty, $variant:ident) => {
        impl Raw for $t {
            const BITS: u32 = <$t>::BITS;
            #[inline(always)]
            fn to_u32(self) -> u32 {
                self as u32
            }
            #[inline(always)]
            fn from_u32(value: u32) -> Self {
                value as $t
            }
            fn data(data: &crate::chunk::ChannelData) -> Option<&Storage<Self>> {
                match data {
                    crate::chunk::ChannelData::$variant(storage) => Some(storage),
                    _ => None,
                }
            }
            fn data_mut(data: &mut crate::chunk::ChannelData) -> Option<&mut Storage<Self>> {
                match data {
                    crate::chunk::ChannelData::$variant(storage) => Some(storage),
                    _ => None,
                }
            }
            fn wrap(storage: Storage<Self>) -> crate::chunk::ChannelData {
                crate::chunk::ChannelData::$variant(storage)
            }
        }
    };
}
raw!(u8, U8);
raw!(u16, U16);
raw!(u32, U32);

/// Typed channel values. Signed and boolean values map onto raw integers.
pub trait Value: Copy + Debug + PartialEq + Send + Sync + 'static {
    type Raw: Raw;
    fn to_raw(self) -> Self::Raw;
    fn from_raw(raw: Self::Raw) -> Self;
}
macro_rules! identity_value {
    ($($t:ty),*) => {$(
        impl Value for $t {
            type Raw = $t;
            #[inline(always)]
            fn to_raw(self) -> $t { self }
            #[inline(always)]
            fn from_raw(raw: $t) -> $t { raw }
        }
    )*};
}
identity_value!(u8, u16, u32);
impl Value for i8 {
    type Raw = u8;
    #[inline(always)]
    fn to_raw(self) -> u8 {
        self as u8
    }
    #[inline(always)]
    fn from_raw(raw: u8) -> i8 {
        raw as i8
    }
}
impl Value for bool {
    type Raw = u8;
    #[inline(always)]
    fn to_raw(self) -> u8 {
        u8::from(self)
    }
    #[inline(always)]
    fn from_raw(raw: u8) -> bool {
        raw != 0
    }
}

/// How packed indices are laid out in 64-bit words.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BitPacking {
    /// `64 / bits` entries per word; leftover high bits stay zero.
    #[default]
    NonSpanning,
    /// Entries are packed back to back and may cross word boundaries.
    Spanning,
}

/// Encoding chosen for a paletted chunk with a given number of distinct values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaletteLayout {
    /// Indices into a first-use-ordered palette, `bits` per entry.
    Indexed { bits: u8 },
    /// Raw values stored directly, `bits` per entry (no palette).
    Direct { bits: u8 },
}

/// Decides paletted encodings. Palette order is always first use; policies
/// choose widths, the switch to direct storage and word packing.
pub trait PalettePolicy: Send + Sync + Debug {
    /// Layout for `distinct` values (`>= 2`) of a raw type `raw_bits` wide.
    fn layout(&self, distinct: usize, raw_bits: u32) -> PaletteLayout;
    fn packing(&self) -> BitPacking {
        BitPacking::NonSpanning
    }
}

fn bits_for(count: usize) -> u8 {
    (usize::BITS - (count.max(2) - 1).leading_zeros()) as u8
}

/// Quasar's default: power-of-two widths (1, 2, 4, 8, 16) so entries never
/// span words and decoding is shift-and-mask; direct storage past 8 bits of
/// palette.
#[derive(Clone, Copy, Debug, Default)]
pub struct FastPalette;

impl PalettePolicy for FastPalette {
    fn layout(&self, distinct: usize, raw_bits: u32) -> PaletteLayout {
        let bits = bits_for(distinct).next_power_of_two();
        if bits > 8 {
            PaletteLayout::Direct {
                bits: raw_bits.next_power_of_two().min(32) as u8,
            }
        } else {
            PaletteLayout::Indexed { bits }
        }
    }
}

/// Threshold policy expressing common external formats: indexed widths are
/// `max(min_bits, ceil(log2(distinct)))` up to `max_indexed_bits`, after which
/// values are stored directly with `direct_bits`.
#[derive(Clone, Copy, Debug)]
pub struct ThresholdPalette {
    pub min_bits: u8,
    pub max_indexed_bits: u8,
    pub direct_bits: u8,
    pub packing: BitPacking,
}

impl PalettePolicy for ThresholdPalette {
    fn layout(&self, distinct: usize, _raw_bits: u32) -> PaletteLayout {
        let bits = bits_for(distinct).max(self.min_bits);
        if bits > self.max_indexed_bits {
            PaletteLayout::Direct {
                bits: self.direct_bits,
            }
        } else {
            PaletteLayout::Indexed { bits }
        }
    }
    fn packing(&self) -> BitPacking {
        self.packing
    }
}

/// Bit-packed fixed-width entries in 64-bit words.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackedWords {
    bits: u8,
    packing: BitPacking,
    per_word: u32,
    words: Vec<u64>,
}

impl PackedWords {
    pub fn new(len: usize, bits: u8, packing: BitPacking) -> Self {
        assert!((1..=32).contains(&bits), "packed width must be 1..=32");
        let per_word = 64 / u32::from(bits);
        let words = match packing {
            BitPacking::NonSpanning => len.div_ceil(per_word as usize),
            BitPacking::Spanning => (len * bits as usize).div_ceil(64),
        };
        Self {
            bits,
            packing,
            per_word,
            words: vec![0; words],
        }
    }
    pub fn bits(&self) -> u8 {
        self.bits
    }
    pub fn packing(&self) -> BitPacking {
        self.packing
    }
    /// Backing words, for zero-copy serialisation to external formats.
    pub fn words(&self) -> &[u64] {
        &self.words
    }
    pub fn from_words(len: usize, bits: u8, packing: BitPacking, words: Vec<u64>) -> Option<Self> {
        let expected = Self::new(len, bits, packing);
        (expected.words.len() == words.len()).then_some(Self { words, ..expected })
    }
    #[inline(always)]
    fn mask(&self) -> u64 {
        (1u64 << self.bits) - 1
    }
    #[inline]
    pub fn get(&self, index: usize) -> u32 {
        let mask = self.mask();
        match self.packing {
            BitPacking::NonSpanning => {
                let word = index / self.per_word as usize;
                let shift = (index % self.per_word as usize) * self.bits as usize;
                ((self.words[word] >> shift) & mask) as u32
            }
            BitPacking::Spanning => {
                let bit = index * self.bits as usize;
                let word = bit / 64;
                let shift = bit % 64;
                let mut value = self.words[word] >> shift;
                if shift + self.bits as usize > 64 {
                    value |= self.words[word + 1] << (64 - shift);
                }
                (value & mask) as u32
            }
        }
    }
    #[inline]
    pub fn set(&mut self, index: usize, value: u32) {
        let mask = self.mask();
        let value = u64::from(value) & mask;
        match self.packing {
            BitPacking::NonSpanning => {
                let word = index / self.per_word as usize;
                let shift = (index % self.per_word as usize) * self.bits as usize;
                self.words[word] = (self.words[word] & !(mask << shift)) | (value << shift);
            }
            BitPacking::Spanning => {
                let bit = index * self.bits as usize;
                let word = bit / 64;
                let shift = bit % 64;
                self.words[word] = (self.words[word] & !(mask << shift)) | (value << shift);
                if shift + self.bits as usize > 64 {
                    let high = 64 - shift;
                    let next_mask = mask >> high;
                    self.words[word + 1] = (self.words[word + 1] & !next_mask) | (value >> high);
                }
            }
        }
    }
    /// Decodes `len` entries in order, word by word (no per-entry division).
    pub fn for_each(&self, len: usize, mut visit: impl FnMut(usize, u32)) {
        let mask = self.mask();
        match self.packing {
            BitPacking::NonSpanning => {
                let mut index = 0;
                'words: for &word in &self.words {
                    let mut bits = word;
                    for _ in 0..self.per_word {
                        if index == len {
                            break 'words;
                        }
                        visit(index, (bits & mask) as u32);
                        bits >>= self.bits;
                        index += 1;
                    }
                }
            }
            BitPacking::Spanning => {
                for index in 0..len {
                    visit(index, self.get(index));
                }
            }
        }
    }
}

/// Palette plus packed entries (or direct values).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paletted<R: Raw> {
    palette: Vec<R>,
    lookup: Option<HashMap<R, u32>>,
    direct: bool,
    packed: PackedWords,
}

const LOOKUP_THRESHOLD: usize = 16;

impl<R: Raw> Paletted<R> {
    fn with_layout(
        len: usize,
        palette: Vec<R>,
        layout: PaletteLayout,
        packing: BitPacking,
    ) -> Self {
        let (bits, direct) = match layout {
            PaletteLayout::Indexed { bits } => (bits, false),
            PaletteLayout::Direct { bits } => (bits, true),
        };
        let mut result = Self {
            palette: if direct { Vec::new() } else { palette },
            lookup: None,
            direct,
            packed: PackedWords::new(len, bits, packing),
        };
        result.rebuild_lookup();
        result
    }
    fn rebuild_lookup(&mut self) {
        self.lookup = (!self.direct && self.palette.len() > LOOKUP_THRESHOLD).then(|| {
            let mut map = HashMap::with_capacity(self.palette.len());
            for (i, value) in self.palette.iter().enumerate() {
                map.entry(*value).or_insert(i as u32);
            }
            map
        });
    }
    /// First-use-ordered palette (empty in direct mode).
    pub fn palette(&self) -> &[R] {
        &self.palette
    }
    pub fn is_direct(&self) -> bool {
        self.direct
    }
    pub fn packed(&self) -> &PackedWords {
        &self.packed
    }
    /// Builds from an external palette and packed words (no validation of
    /// semantics beyond indices being in range).
    pub fn from_parts(
        len: usize,
        palette: Vec<R>,
        direct: bool,
        packed: PackedWords,
    ) -> Option<Self> {
        if !direct {
            let mut ok = true;
            packed.for_each(len, |_, index| ok &= (index as usize) < palette.len());
            if !ok {
                return None;
            }
        }
        let mut result = Self {
            palette: if direct { Vec::new() } else { palette },
            lookup: None,
            direct,
            packed,
        };
        result.rebuild_lookup();
        Some(result)
    }
    #[inline]
    pub fn get(&self, index: usize) -> R {
        let raw = self.packed.get(index);
        if self.direct {
            R::from_u32(raw)
        } else {
            self.palette[raw as usize]
        }
    }
    fn find(&self, value: R) -> Option<u32> {
        match &self.lookup {
            Some(map) => map.get(&value).copied(),
            None => self
                .palette
                .iter()
                .position(|entry| *entry == value)
                .map(|i| i as u32),
        }
    }
    fn capacity(&self) -> usize {
        1usize << self.packed.bits
    }
    fn set(&mut self, len: usize, index: usize, value: R, policy: &dyn PalettePolicy) -> R {
        let old = self.get(index);
        if old == value {
            return old;
        }
        if self.direct {
            self.packed.set(index, value.to_u32());
            return old;
        }
        let slot = match self.find(value) {
            Some(slot) => slot,
            None => {
                if self.palette.len() == self.capacity() {
                    self.grow(len, policy);
                    if self.direct {
                        self.packed.set(index, value.to_u32());
                        return old;
                    }
                }
                self.palette.push(value);
                let slot = (self.palette.len() - 1) as u32;
                if let Some(map) = &mut self.lookup {
                    map.insert(value, slot);
                } else if self.palette.len() > LOOKUP_THRESHOLD {
                    self.rebuild_lookup();
                }
                slot
            }
        };
        self.packed.set(index, slot);
        old
    }
    fn grow(&mut self, len: usize, policy: &dyn PalettePolicy) {
        let layout = policy.layout(self.palette.len() + 1, R::BITS);
        let mut next = Self::with_layout(len, self.palette.clone(), layout, policy.packing());
        let direct = next.direct;
        let palette = &self.palette;
        self.packed.for_each(len, |i, slot| {
            let value = palette[slot as usize];
            next.packed
                .set(i, if direct { value.to_u32() } else { slot });
        });
        *self = next;
    }
    fn for_each(&self, len: usize, mut visit: impl FnMut(usize, R)) {
        if self.direct {
            self.packed
                .for_each(len, |i, raw| visit(i, R::from_u32(raw)));
        } else {
            let palette = &self.palette;
            self.packed
                .for_each(len, |i, slot| visit(i, palette[slot as usize]));
        }
    }
}

/// Sorted sparse entries over a default value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sparse<R: Raw> {
    pub default: R,
    entries: Vec<(u32, R)>,
}

impl<R: Raw> Sparse<R> {
    pub fn entries(&self) -> &[(u32, R)] {
        &self.entries
    }
    fn get(&self, index: usize) -> R {
        match self.entries.binary_search_by_key(&(index as u32), |e| e.0) {
            Ok(i) => self.entries[i].1,
            Err(_) => self.default,
        }
    }
    fn set(&mut self, index: usize, value: R) -> R {
        match self.entries.binary_search_by_key(&(index as u32), |e| e.0) {
            Ok(i) => {
                let old = self.entries[i].1;
                if value == self.default {
                    self.entries.remove(i);
                } else {
                    self.entries[i].1 = value;
                }
                old
            }
            Err(i) => {
                if value != self.default {
                    self.entries.insert(i, (index as u32, value));
                }
                self.default
            }
        }
    }
}

/// How a channel stores non-uniform chunks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageKind {
    /// Palette or direct packed words, decided by the channel's policy.
    Paletted,
    /// One raw value per voxel.
    Dense,
    /// 4-bit values (`u8` channels only; values are masked to 0..=15).
    Nibble,
    /// Sorted entries over the uniform value; for rare data.
    Sparse,
}

/// One channel of one chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Storage<R: Raw> {
    Uniform(R),
    Dense(Box<[R]>),
    /// Boxed so uniform and dense channels stay small (16 bytes).
    Paletted(Box<Paletted<R>>),
    Nibble(Box<[u8]>),
    Sparse(Box<Sparse<R>>),
}

impl<R: Raw> Storage<R> {
    /// The value if every voxel has it (cheap; does not scan).
    #[inline]
    pub fn uniform(&self) -> Option<R> {
        match self {
            Self::Uniform(value) => Some(*value),
            Self::Sparse(sparse) if sparse.entries.is_empty() => Some(sparse.default),
            _ => None,
        }
    }
    #[inline]
    pub fn get(&self, index: usize) -> R {
        match self {
            Self::Uniform(value) => *value,
            Self::Dense(values) => values[index],
            Self::Paletted(paletted) => paletted.get(index),
            Self::Nibble(bytes) => {
                R::from_u32(u32::from((bytes[index >> 1] >> ((index & 1) * 4)) & 15))
            }
            Self::Sparse(sparse) => sparse.get(index),
        }
    }
    /// Sets one voxel, promoting a uniform chunk to `kind`. Returns the old value.
    pub fn set(
        &mut self,
        len: usize,
        index: usize,
        value: R,
        kind: StorageKind,
        policy: &dyn PalettePolicy,
    ) -> R {
        if let Self::Uniform(current) = self {
            let current = *current;
            if current == value {
                return value;
            }
            *self = Self::promoted(len, current, value, kind, policy);
        }
        match self {
            Self::Uniform(_) => unreachable!(),
            Self::Dense(values) => std::mem::replace(&mut values[index], value),
            Self::Paletted(paletted) => paletted.set(len, index, value, policy),
            Self::Nibble(bytes) => {
                let shift = (index & 1) * 4;
                let byte = &mut bytes[index >> 1];
                let old = (*byte >> shift) & 15;
                let nibble = (value.to_u32() & 15) as u8;
                *byte = (*byte & !(15 << shift)) | (nibble << shift);
                R::from_u32(u32::from(old))
            }
            Self::Sparse(sparse) => sparse.set(index, value),
        }
    }
    fn promoted(
        len: usize,
        current: R,
        value: R,
        kind: StorageKind,
        policy: &dyn PalettePolicy,
    ) -> Self {
        let _ = value;
        match kind {
            StorageKind::Dense => Self::Dense(vec![current; len].into_boxed_slice()),
            StorageKind::Nibble => {
                let nibble = (current.to_u32() & 15) as u8;
                Self::Nibble(vec![nibble | (nibble << 4); len.div_ceil(2)].into_boxed_slice())
            }
            StorageKind::Sparse => Self::Sparse(Box::new(Sparse {
                default: current,
                entries: Vec::new(),
            })),
            StorageKind::Paletted => {
                let layout = policy.layout(2, R::BITS);
                let mut paletted =
                    Paletted::with_layout(len, vec![current], layout, policy.packing());
                if paletted.direct {
                    let raw = current.to_u32();
                    for i in 0..len {
                        paletted.packed.set(i, raw);
                    }
                }
                Self::Paletted(Box::new(paletted))
            }
        }
    }
    /// Sets every voxel to `value` (becomes uniform).
    pub fn fill(&mut self, value: R) {
        *self = Self::Uniform(value);
    }
    /// Visits every voxel in index order.
    pub fn for_each(&self, len: usize, mut visit: impl FnMut(usize, R)) {
        match self {
            Self::Uniform(value) => (0..len).for_each(|i| visit(i, *value)),
            Self::Dense(values) => values.iter().enumerate().for_each(|(i, v)| visit(i, *v)),
            Self::Paletted(paletted) => paletted.for_each(len, visit),
            Self::Nibble(bytes) => {
                for i in 0..len {
                    visit(
                        i,
                        R::from_u32(u32::from((bytes[i >> 1] >> ((i & 1) * 4)) & 15)),
                    );
                }
            }
            Self::Sparse(sparse) => {
                let mut next = 0;
                for &(index, value) in &sparse.entries {
                    for i in next..index as usize {
                        visit(i, sparse.default);
                    }
                    visit(index as usize, value);
                    next = index as usize + 1;
                }
                for i in next..len {
                    visit(i, sparse.default);
                }
            }
        }
    }
    /// Decodes every voxel into `out` (len = chunk volume).
    pub fn decode_into(&self, out: &mut [R]) {
        match self {
            Self::Uniform(value) => out.fill(*value),
            Self::Dense(values) => out.copy_from_slice(values),
            _ => {
                let len = out.len();
                self.for_each(len, |i, v| out[i] = v);
            }
        }
    }
    /// Rebuilds the storage from its current contents: uniform when all
    /// voxels match; otherwise the palette is reduced to values in use
    /// (first-use order by index) and widths are recomputed.
    pub fn compact(&mut self, len: usize, kind: StorageKind, policy: &dyn PalettePolicy) {
        if self.uniform().is_some() {
            if let Some(value) = self.uniform() {
                *self = Self::Uniform(value);
            }
            return;
        }
        let mut values = vec![R::default(); len];
        self.decode_into(&mut values);
        if values.iter().all(|v| *v == values[0]) {
            *self = Self::Uniform(values[0]);
            return;
        }
        if kind != StorageKind::Paletted {
            return;
        }
        let mut distinct: Vec<R> = Vec::new();
        let mut seen = HashMap::new();
        for value in &values {
            seen.entry(*value).or_insert_with(|| {
                distinct.push(*value);
                distinct.len() - 1
            });
        }
        let layout = policy.layout(distinct.len(), R::BITS);
        let mut paletted = Paletted::with_layout(len, distinct, layout, policy.packing());
        for (i, value) in values.iter().enumerate() {
            let raw = if paletted.direct {
                value.to_u32()
            } else {
                seen[value] as u32
            };
            paletted.packed.set(i, raw);
        }
        *self = Self::Paletted(Box::new(paletted));
    }
    /// Approximate heap bytes used.
    pub fn memory_bytes(&self) -> usize {
        match self {
            Self::Uniform(_) => 0,
            Self::Dense(values) => values.len() * std::mem::size_of::<R>(),
            Self::Paletted(p) => {
                std::mem::size_of::<Paletted<R>>()
                    + p.packed.words.len() * 8
                    + p.palette.len() * std::mem::size_of::<R>()
                    + p.lookup
                        .as_ref()
                        .map_or(0, |m| m.capacity() * (std::mem::size_of::<R>() + 4))
            }
            Self::Nibble(bytes) => bytes.len(),
            Self::Sparse(s) => {
                std::mem::size_of::<Sparse<R>>() + s.entries.len() * (4 + std::mem::size_of::<R>())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEN: usize = 4096;

    fn exercise(kind: StorageKind, policy: &dyn PalettePolicy) {
        let mut storage = Storage::<u16>::Uniform(0);
        let mut reference = vec![0u16; LEN];
        let mut rng = quasar_core::Rng::seed_from_u64(3);
        for step in 0..20_000 {
            let i = rng.range_usize(0..LEN);
            let distinct = if step < 10_000 { 6 } else { 700 };
            let v = rng.range_u32(0..distinct) as u16;
            let v = if kind == StorageKind::Nibble {
                v & 15
            } else {
                v
            };
            let old = storage.set(LEN, i, v, kind, policy);
            assert_eq!(old, reference[i]);
            reference[i] = v;
        }
        for (i, v) in reference.iter().enumerate() {
            assert_eq!(storage.get(i), *v);
        }
        let mut decoded = vec![0u16; LEN];
        storage.decode_into(&mut decoded);
        assert_eq!(decoded, reference);
        storage.compact(LEN, kind, policy);
        storage.decode_into(&mut decoded);
        assert_eq!(decoded, reference);
    }

    #[test]
    fn every_storage_kind_matches_a_reference_array() {
        for kind in [
            StorageKind::Paletted,
            StorageKind::Dense,
            StorageKind::Sparse,
        ] {
            exercise(kind, &FastPalette);
        }
        exercise(
            StorageKind::Paletted,
            &ThresholdPalette {
                min_bits: 4,
                max_indexed_bits: 8,
                direct_bits: 15,
                packing: BitPacking::NonSpanning,
            },
        );
        exercise(
            StorageKind::Paletted,
            &ThresholdPalette {
                min_bits: 3,
                max_indexed_bits: 9,
                direct_bits: 13,
                packing: BitPacking::Spanning,
            },
        );
        let mut nibble = Storage::<u8>::Uniform(3);
        nibble.set(LEN, 5, 12, StorageKind::Nibble, &FastPalette);
        assert_eq!((nibble.get(4), nibble.get(5)), (3, 12));
    }

    #[test]
    fn palettes_grow_in_first_use_order_with_policy_widths() {
        let policy = ThresholdPalette {
            min_bits: 4,
            max_indexed_bits: 8,
            direct_bits: 12,
            packing: BitPacking::NonSpanning,
        };
        let mut storage = Storage::<u16>::Uniform(7);
        storage.set(LEN, 0, 9, StorageKind::Paletted, &policy);
        storage.set(LEN, 1, 3, StorageKind::Paletted, &policy);
        let Storage::Paletted(p) = &storage else {
            panic!()
        };
        assert_eq!(p.palette(), &[7, 9, 3]);
        assert_eq!(p.packed().bits(), 4, "minimum width");
        assert_eq!(p.packed().words().len(), LEN / 16, "16 entries per word");
        for v in 10..30u16 {
            storage.set(LEN, v as usize, v, StorageKind::Paletted, &policy);
        }
        let Storage::Paletted(p) = &storage else {
            panic!()
        };
        assert_eq!(p.packed().bits(), 5);
        assert_eq!(
            p.packed().words().len(),
            LEN.div_ceil(12),
            "non-spanning: 12 per word"
        );
        for v in 0..300u16 {
            storage.set(
                LEN,
                100 + v as usize,
                1000 + v,
                StorageKind::Paletted,
                &policy,
            );
        }
        let Storage::Paletted(p) = &storage else {
            panic!()
        };
        assert!(p.is_direct());
        assert_eq!(p.packed().bits(), 12);
        assert_eq!(storage.get(150), 1050);
        assert_eq!(storage.get(0), 9);
    }

    #[test]
    fn packed_words_round_trip_both_packings() {
        for packing in [BitPacking::NonSpanning, BitPacking::Spanning] {
            for bits in [1u8, 3, 5, 7, 13, 17, 32] {
                let mut words = PackedWords::new(1000, bits, packing);
                let mask = if bits == 32 {
                    u32::MAX
                } else {
                    (1 << bits) - 1
                };
                for i in 0..1000 {
                    words.set(i, (i as u32).wrapping_mul(2_654_435_761) & mask);
                }
                for i in 0..1000 {
                    assert_eq!(words.get(i), (i as u32).wrapping_mul(2_654_435_761) & mask);
                }
                let mut seen = 0;
                words.for_each(1000, |i, v| {
                    assert_eq!(v, (i as u32).wrapping_mul(2_654_435_761) & mask);
                    seen += 1;
                });
                assert_eq!(seen, 1000);
                let rebuilt =
                    PackedWords::from_words(1000, bits, packing, words.words().to_vec()).unwrap();
                assert_eq!(rebuilt, words);
            }
        }
        assert_eq!(
            PackedWords::new(4096, 5, BitPacking::Spanning)
                .words()
                .len(),
            320
        );
    }

    #[test]
    fn fast_policy_uses_power_of_two_widths() {
        assert_eq!(
            FastPalette.layout(2, 16),
            PaletteLayout::Indexed { bits: 1 }
        );
        assert_eq!(
            FastPalette.layout(5, 16),
            PaletteLayout::Indexed { bits: 4 }
        );
        assert_eq!(
            FastPalette.layout(200, 16),
            PaletteLayout::Indexed { bits: 8 }
        );
        assert_eq!(
            FastPalette.layout(300, 16),
            PaletteLayout::Direct { bits: 16 }
        );
        assert_eq!(
            FastPalette.layout(300, 8),
            PaletteLayout::Direct { bits: 8 }
        );
    }

    #[test]
    fn signed_and_bool_values_map_to_raw() {
        assert_eq!(i8::from_raw((-5i8).to_raw()), -5);
        assert!(bool::from_raw(true.to_raw()));
        assert_eq!(Storage::<u8>::Uniform(4).uniform(), Some(4));
    }
}
