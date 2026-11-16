//! Generic flood-fill propagation for light-like values.
//!
//! The engine spreads small integer levels (block light, sky light, fluid
//! levels, heat, oxygen) through a `u8` channel. [`PropagationRules`] decide
//! the cost of entering each cell (blocked, free, or attenuating) and which
//! cells emit; [`QueueDiscipline`] fixes the visit order, so a game can match
//! a reference implementation's observable update order. Results are
//! deterministic for a given discipline.
use crate::chunk::ChannelId;
use crate::edit::dirty_for;
use crate::layout::{ChunkLayout, Face};
use crate::volume::{Cursor, Volume};
use quasar_math::IVec3;
use std::collections::VecDeque;

/// Order in which queued cells are processed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum QueueDiscipline {
    /// Highest level first; FIFO within a level (Dijkstra order).
    #[default]
    BucketFifo,
    /// Highest level first; LIFO within a level.
    BucketLifo,
    /// One FIFO queue (breadth first, re-queues on improvement).
    Fifo,
}

/// Game rules for one propagated channel.
pub trait PropagationRules<L: ChunkLayout> {
    /// Highest level (15 for 4-bit light).
    fn max_level(&self) -> u8 {
        15
    }
    /// Cost of moving into `to` travelling along `travel`: `None` blocks,
    /// `Some(0)` passes without loss (e.g. sky light falling straight down).
    /// Read the volume through `cursor` (`Volume::get_with`): consecutive
    /// queries are usually in the same chunk.
    fn cost(&self, volume: &Volume<L>, cursor: &mut Cursor, to: IVec3, travel: Face) -> Option<u8>;
    /// Level a cell emits by itself (0 for none).
    fn emission(&self, volume: &Volume<L>, cursor: &mut Cursor, at: IVec3) -> u8;
}

/// Changes made by one propagation call.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PropagationStats {
    pub changed: usize,
    /// Chunks whose level data (or halo) changed, sorted.
    pub dirty: Vec<IVec3>,
}

/// Reusable propagation queues.
#[derive(Debug, Default)]
pub struct Propagator {
    discipline: QueueDiscipline,
    buckets: Vec<VecDeque<(IVec3, u8)>>,
    fifo: VecDeque<(IVec3, u8)>,
    removal: VecDeque<(IVec3, u8)>,
    dirty: Vec<IVec3>,
    /// Chunk of the last interior write (skips duplicate dirty entries).
    last_interior: Option<IVec3>,
}

impl Propagator {
    pub fn new(discipline: QueueDiscipline) -> Self {
        Self {
            discipline,
            ..Self::default()
        }
    }
    pub fn discipline(&self) -> QueueDiscipline {
        self.discipline
    }
    fn push(&mut self, cell: IVec3, level: u8) {
        match self.discipline {
            QueueDiscipline::Fifo => self.fifo.push_back((cell, level)),
            _ => {
                let bucket = level as usize;
                if self.buckets.len() <= bucket {
                    self.buckets.resize_with(bucket + 1, VecDeque::new);
                }
                self.buckets[bucket].push_back((cell, level));
            }
        }
    }
    fn pop(&mut self) -> Option<(IVec3, u8)> {
        match self.discipline {
            QueueDiscipline::Fifo => self.fifo.pop_front(),
            QueueDiscipline::BucketFifo => {
                self.buckets.iter_mut().rev().find_map(VecDeque::pop_front)
            }
            QueueDiscipline::BucketLifo => {
                self.buckets.iter_mut().rev().find_map(VecDeque::pop_back)
            }
        }
    }
    fn write<L: ChunkLayout>(
        &mut self,
        volume: &mut Volume<L>,
        cursor: &mut Cursor,
        channel: ChannelId<u8>,
        cell: IVec3,
        level: u8,
        stats: &mut PropagationStats,
    ) -> bool {
        match volume.set_loaded_with(cursor, channel, cell, level) {
            Some(old) if old != level => {
                stats.changed += 1;
                // Interior cells only dirty their own chunk: skip repeats
                // (propagation writes long runs within one chunk).
                let local = L::local_of(cell).as_ivec3();
                let edge = L::EDGE as i32 - 1;
                if local.min_element() > 0 && local.max_element() < edge {
                    let chunk = L::chunk_of(cell);
                    if self.last_interior != Some(chunk) {
                        self.last_interior = Some(chunk);
                        self.dirty.push(chunk);
                    }
                } else {
                    dirty_for::<L>(cell, 1, &mut self.dirty);
                }
                true
            }
            Some(_) => true,
            None => false,
        }
    }
    fn spread<L: ChunkLayout, R: PropagationRules<L>>(
        &mut self,
        volume: &mut Volume<L>,
        channel: ChannelId<u8>,
        rules: &R,
        stats: &mut PropagationStats,
    ) {
        let mut cursor = Cursor::default();
        let mut rules_cursor = Cursor::default();
        while let Some((cell, level)) = self.pop() {
            if volume.get_with(&mut cursor, channel, cell) != level || level == 0 {
                continue; // stale entry
            }
            for face in Face::ALL {
                let to = cell + face.normal();
                if !volume.contains_with(&mut cursor, L::chunk_of(to)) {
                    continue;
                }
                let Some(cost) = rules.cost(volume, &mut rules_cursor, to, face) else {
                    continue;
                };
                let next = level.saturating_sub(cost);
                if next == 0 || next <= volume.get_with(&mut cursor, channel, to) {
                    continue;
                }
                if self.write(volume, &mut cursor, channel, to, next, stats) {
                    self.push(to, next);
                }
            }
        }
    }
    fn finish(&mut self, mut stats: PropagationStats) -> PropagationStats {
        self.last_interior = None;
        self.dirty.sort_by_key(|c| (c.x, c.y, c.z));
        self.dirty.dedup();
        stats.dirty = std::mem::take(&mut self.dirty);
        stats
    }

    /// Raises levels at `seeds` (keeping higher existing levels) and spreads
    /// them. Only loaded chunks are touched. A seed equal to the cell's
    /// current level spreads again without a write, which floods newly loaded
    /// neighbours from existing light (seed the border cells facing them).
    pub fn add<L: ChunkLayout, R: PropagationRules<L>>(
        &mut self,
        volume: &mut Volume<L>,
        channel: ChannelId<u8>,
        seeds: &[(IVec3, u8)],
        rules: &R,
    ) -> PropagationStats {
        let mut stats = PropagationStats::default();
        let mut cursor = Cursor::default();
        let max = rules.max_level();
        for &(cell, level) in seeds {
            let level = level.min(max);
            let current = volume.get_with(&mut cursor, channel, cell);
            if level > current {
                if self.write(volume, &mut cursor, channel, cell, level, &mut stats) {
                    self.push(cell, level);
                }
            } else if level == current && level > 0 && volume.contains(L::chunk_of(cell)) {
                self.push(cell, level);
            }
        }
        self.spread(volume, channel, rules, &mut stats);
        self.finish(stats)
    }

    /// Recomputes levels after `cells` changed (opacity or emission edits):
    /// removes light that depended on them, then refills from surviving
    /// neighbours and emitters. Equivalent to a full rebuild of the affected
    /// region.
    pub fn update<L: ChunkLayout, R: PropagationRules<L>>(
        &mut self,
        volume: &mut Volume<L>,
        channel: ChannelId<u8>,
        cells: &[IVec3],
        rules: &R,
    ) -> PropagationStats {
        let mut stats = PropagationStats::default();
        let mut cursor = Cursor::default();
        let mut rules_cursor = Cursor::default();
        let mut relight: Vec<(IVec3, u8)> = Vec::new();
        for &cell in cells {
            let old = volume.get_with(&mut cursor, channel, cell);
            if old > 0 && self.write(volume, &mut cursor, channel, cell, 0, &mut stats) {
                self.removal.push_back((cell, old));
            }
            relight.push((cell, 0));
        }
        while let Some((cell, old)) = self.removal.pop_front() {
            for face in Face::ALL {
                let to = cell + face.normal();
                if !volume.contains(L::chunk_of(to)) {
                    continue;
                }
                let level = volume.get_with(&mut cursor, channel, to);
                if level == 0 {
                    continue;
                }
                let dependent = rules
                    .cost(volume, &mut rules_cursor, to, face)
                    .is_some_and(|cost| level <= old.saturating_sub(cost));
                if dependent {
                    self.write(volume, &mut cursor, channel, to, 0, &mut stats);
                    self.removal.push_back((to, level));
                    relight.push((to, 0));
                } else {
                    relight.push((to, level));
                }
            }
        }
        // Refill: surviving neighbours keep their level; every touched cell
        // that emits re-seeds itself.
        for (cell, level) in relight {
            let emitted = rules
                .emission(volume, &mut rules_cursor, cell)
                .min(rules.max_level());
            let current = volume.get_with(&mut cursor, channel, cell);
            let seed = level.max(emitted).max(current);
            if seed > 0 {
                if seed > current {
                    self.write(volume, &mut cursor, channel, cell, seed, &mut stats);
                }
                self.push(cell, seed);
            }
        }
        self.spread(volume, channel, rules, &mut stats);
        self.finish(stats)
    }
}

/// Sky seeds for columns: walks down from `top_y` in every column of the
/// inclusive `x`/`z` range and yields `(cell, max)` until `opaque` holds.
pub fn sky_seeds<L: ChunkLayout>(
    volume: &Volume<L>,
    x: std::ops::RangeInclusive<i32>,
    z: std::ops::RangeInclusive<i32>,
    top_y: i32,
    bottom_y: i32,
    max: u8,
    mut opaque: impl FnMut(&Volume<L>, IVec3) -> bool,
) -> Vec<(IVec3, u8)> {
    let mut seeds = Vec::new();
    for cz in z {
        for cx in x.clone() {
            let mut y = top_y;
            while y >= bottom_y {
                let cell = IVec3::new(cx, y, cz);
                if opaque(volume, cell) {
                    break;
                }
                seeds.push((cell, max));
                y -= 1;
            }
        }
    }
    seeds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Schema;
    use crate::layout::Cube16;
    use crate::storage::StorageKind;

    struct Light {
        block: ChannelId<u16>,
        sky: bool,
    }
    impl PropagationRules<Cube16> for Light {
        fn cost(
            &self,
            volume: &Volume<Cube16>,
            cursor: &mut Cursor,
            to: IVec3,
            travel: Face,
        ) -> Option<u8> {
            let b = volume.get_with(cursor, self.block, to);
            if b == 1 {
                None
            } else if self.sky && travel == Face::NegY {
                Some(0)
            } else {
                Some(1)
            }
        }
        fn emission(&self, volume: &Volume<Cube16>, cursor: &mut Cursor, at: IVec3) -> u8 {
            if volume.get_with(cursor, self.block, at) == 2 {
                14
            } else {
                0
            }
        }
    }

    fn world() -> (Volume<Cube16>, ChannelId<u16>, ChannelId<u8>) {
        let mut schema = Schema::new();
        let block = schema.channel("block", StorageKind::Paletted, 0u16);
        let light = schema.derived_channel("light", StorageKind::Nibble, 0u8);
        let mut volume = Volume::new(schema);
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1 {
                    volume.ensure_chunk(IVec3::new(x, y, z));
                }
            }
        }
        (volume, block, light)
    }

    /// Reference: Dijkstra over every loaded cell from all emitters.
    fn reference(volume: &Volume<Cube16>, rules: &Light, light: ChannelId<u8>) -> Vec<(IVec3, u8)> {
        let mut copy = volume.clone();
        for coord in copy.chunk_coords() {
            copy.chunk_mut(coord).unwrap().fill(light, 0);
        }
        let mut seeds = Vec::new();
        for y in -16..32 {
            for z in -16..32 {
                for x in -16..32 {
                    let p = IVec3::new(x, y, z);
                    let e = rules.emission(&copy, &mut Cursor::default(), p);
                    if e > 0 {
                        seeds.push((p, e));
                    }
                }
            }
        }
        Propagator::new(QueueDiscipline::BucketFifo).add(&mut copy, light, &seeds, rules);
        let mut out = Vec::new();
        for y in -16..32 {
            for z in -16..32 {
                for x in -16..32 {
                    let p = IVec3::new(x, y, z);
                    out.push((p, copy.get(light, p)));
                }
            }
        }
        out
    }

    #[test]
    fn light_spreads_with_attenuation_and_walls_block_it() {
        let (mut volume, block, light) = world();
        for y in -16..32 {
            for z in -16..32 {
                volume.set(block, IVec3::new(4, y, z), 1);
            }
        }
        volume.set(block, IVec3::ZERO, 2);
        let rules = Light { block, sky: false };
        let mut engine = Propagator::new(QueueDiscipline::BucketFifo);
        let stats = engine.add(&mut volume, light, &[(IVec3::ZERO, 14)], &rules);
        assert_eq!(volume.get(light, IVec3::ZERO), 14);
        assert_eq!(volume.get(light, IVec3::new(3, 0, 0)), 11);
        assert_eq!(volume.get(light, IVec3::new(0, 0, -5)), 9);
        assert_eq!(volume.get(light, IVec3::new(5, 0, 0)), 0, "wall blocks");
        assert!(stats.changed > 100 && stats.dirty.contains(&IVec3::new(-1, 0, 0)));
    }

    #[test]
    fn updates_match_a_full_rebuild_for_every_discipline() {
        for discipline in [
            QueueDiscipline::BucketFifo,
            QueueDiscipline::BucketLifo,
            QueueDiscipline::Fifo,
        ] {
            let (mut volume, block, light) = world();
            let rules = Light { block, sky: false };
            let mut rng = quasar_core::Rng::seed_from_u64(11);
            let mut engine = Propagator::new(discipline);
            for step in 0..60 {
                let p = IVec3::new(
                    rng.range_i32(-12..28),
                    rng.range_i32(-12..28),
                    rng.range_i32(-12..28),
                );
                let kind = if step % 4 == 0 {
                    2
                } else {
                    rng.range_u32(0..2) as u16
                };
                volume.set(block, p, kind);
                engine.update(&mut volume, light, &[p], &rules);
            }
            let expected = reference(&volume, &rules, light);
            for (p, level) in expected {
                assert_eq!(volume.get(light, p), level, "{discipline:?} at {p}");
            }
        }
    }

    #[test]
    fn sky_light_falls_without_loss_and_shadows_under_roofs() {
        let (mut volume, block, light) = world();
        for x in -2..=2 {
            for z in -2..=2 {
                volume.set(block, IVec3::new(x, 10, z), 1);
            }
        }
        let rules = Light { block, sky: true };
        let seeds = sky_seeds(&volume, -16..=31, -16..=31, 31, -16, 15, |v, p| {
            v.get(block, p) == 1
        });
        let mut engine = Propagator::new(QueueDiscipline::BucketFifo);
        engine.add(&mut volume, light, &seeds, &rules);
        assert_eq!(
            volume.get(light, IVec3::new(10, -10, 10)),
            15,
            "open sky reaches down"
        );
        let under = volume.get(light, IVec3::new(0, 5, 0));
        assert!(
            under > 0 && under < 15,
            "roof shades but light leaks around: {under}"
        );
        // Removing the roof restores full sky light below it.
        let roof: Vec<IVec3> = (-2..=2)
            .flat_map(|x| (-2..=2).map(move |z| IVec3::new(x, 10, z)))
            .collect();
        for p in &roof {
            volume.set(block, *p, 0);
        }
        let mut changed: Vec<IVec3> = roof.clone();
        changed.extend(roof.iter().map(|p| *p + IVec3::Y));
        let seeds: Vec<_> = roof.iter().map(|p| (*p, 0)).collect();
        let _ = seeds;
        engine.update(&mut volume, light, &changed, &rules);
        assert_eq!(volume.get(light, IVec3::new(0, 5, 0)), 15);
    }

    #[test]
    fn equal_seeds_flood_newly_loaded_chunks() {
        let (mut volume, block, light) = world();
        let rules = Light { block, sky: false };
        let mut engine = Propagator::new(QueueDiscipline::BucketFifo);
        // Light the loaded region, then load a neighbour chunk that starts dark.
        engine.add(&mut volume, light, &[(IVec3::new(31, 0, 0), 14)], &rules);
        assert_eq!(volume.get(light, IVec3::new(32, 0, 0)), 0, "not loaded yet");
        volume.ensure_chunk(IVec3::new(2, 0, 0));
        let border: Vec<(IVec3, u8)> = (-16..32)
            .flat_map(|y| (-16..32).map(move |z| IVec3::new(31, y, z)))
            .map(|p| (p, volume.get(light, p)))
            .collect();
        let stats = engine.add(&mut volume, light, &border, &rules);
        assert_eq!(volume.get(light, IVec3::new(32, 0, 0)), 13);
        assert_eq!(volume.get(light, IVec3::new(35, 0, 0)), 10);
        assert!(stats.dirty.contains(&IVec3::new(2, 0, 0)));
    }
}
