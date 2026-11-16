//! Connected components and grid splitting.
//!
//! After cells are removed from a dynamic grid, [`split_check`] floods from
//! the removed cells' neighbours and reports the separate pieces, so a game
//! can move all but one into new grids with [`Volume::extract_cells`].
use crate::layout::{ChunkLayout, Face};
use crate::volume::Volume;
use quasar_math::IVec3;
use std::collections::{HashSet, VecDeque};

/// Pieces found by a flood fill.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Components {
    /// Cell groups, largest first (ties by smallest cell), each sorted.
    pub groups: Vec<Vec<IVec3>>,
    /// The flood fill hit `max_cells` and stopped early.
    pub truncated: bool,
}

fn key(c: &IVec3) -> (i32, i32, i32) {
    (c.x, c.y, c.z)
}

/// Floods from each seed over cells where `solid` holds, moving across a face
/// only when `connected(volume, cell, face)` holds. Seeds that are not solid
/// are ignored; seeds in one component produce one group.
pub fn components<L: ChunkLayout>(
    volume: &Volume<L>,
    seeds: &[IVec3],
    max_cells: usize,
    mut solid: impl FnMut(&Volume<L>, IVec3) -> bool,
    mut connected: impl FnMut(&Volume<L>, IVec3, Face) -> bool,
) -> Components {
    let mut visited: HashSet<IVec3> = HashSet::new();
    let mut result = Components::default();
    let mut total = 0usize;
    let mut sorted_seeds = seeds.to_vec();
    sorted_seeds.sort_by_key(key);
    sorted_seeds.dedup();
    for seed in sorted_seeds {
        if visited.contains(&seed) || !solid(volume, seed) {
            continue;
        }
        let mut group = Vec::new();
        let mut queue = VecDeque::from([seed]);
        visited.insert(seed);
        while let Some(cell) = queue.pop_front() {
            group.push(cell);
            total += 1;
            if total >= max_cells {
                result.truncated = true;
                break;
            }
            for face in Face::ALL {
                let next = cell + face.normal();
                if !visited.contains(&next) && solid(volume, next) && connected(volume, cell, face)
                {
                    visited.insert(next);
                    queue.push_back(next);
                }
            }
        }
        group.sort_by_key(key);
        result.groups.push(group);
        if result.truncated {
            break;
        }
    }
    result.groups.sort_by(|a, b| {
        b.len()
            .cmp(&a.len())
            .then_with(|| key(&a[0]).cmp(&key(&b[0])))
    });
    result
}

/// Splits after removals: floods from the solid face-neighbours of
/// `removed`. Returns `Some` with two or more groups when the structure fell
/// apart, `None` when it is still one piece (or nothing remains).
pub fn split_check<L: ChunkLayout>(
    volume: &Volume<L>,
    removed: &[IVec3],
    max_cells: usize,
    mut solid: impl FnMut(&Volume<L>, IVec3) -> bool,
    connected: impl FnMut(&Volume<L>, IVec3, Face) -> bool,
) -> Option<Components> {
    let mut seeds = Vec::new();
    for cell in removed {
        for face in Face::ALL {
            let next = *cell + face.normal();
            if solid(volume, next) {
                seeds.push(next);
            }
        }
    }
    let found = components(volume, &seeds, max_cells, solid, connected);
    (found.groups.len() > 1 && !found.truncated).then_some(found)
}

impl<L: ChunkLayout> Volume<L> {
    /// Moves every channel's value at `cells` into a new volume with the
    /// same schema (and no source), resetting the cells here to defaults.
    pub fn extract_cells(&mut self, cells: &[IVec3]) -> Volume<L> {
        let mut out = Volume::with_schema(self.schema().clone());
        let channels = self.schema().len();
        for cell in cells {
            for channel in 0..channels {
                let value = self.get_raw(channel, *cell);
                let default = self.default_raw(channel);
                if value != default {
                    out.set_raw(channel, *cell, value);
                    self.set_raw(channel, *cell, default);
                } else if !out.contains(L::chunk_of(*cell)) {
                    out.ensure_chunk(L::chunk_of(*cell));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{AIR, BlockCatalog, BlockChannels, BlockDef};
    use crate::chunk::Schema;
    use crate::edit::{EditBatch, EditOptions};
    use crate::layout::Cube16;
    use crate::orient::Orientation;
    use crate::storage::StorageKind;

    #[test]
    fn cutting_a_bar_splits_it_into_two_pieces() {
        let mut schema = Schema::new();
        let block = schema.channel("block", StorageKind::Paletted, AIR);
        let orientation = schema.channel("orientation", StorageKind::Paletted, 0u8);
        let part = schema.channel("part", StorageKind::Sparse, 0u32);
        let ch = BlockChannels {
            block,
            orientation,
            part,
        };
        let mut volume = Volume::<Cube16>::new(schema);
        let mut catalog = BlockCatalog::new();
        let armor = catalog.add(BlockDef::cube("armor", 1));
        let mut batch = EditBatch::new();
        for x in 0..10 {
            catalog
                .place(
                    &mut batch,
                    &volume,
                    ch,
                    armor,
                    Orientation::IDENTITY,
                    IVec3::new(x, 0, 0),
                )
                .unwrap();
        }
        volume.apply(&batch, EditOptions::default());
        let solid = |v: &Volume<Cube16>, p: IVec3| v.get(block, p) != AIR;
        let connected = |v: &Volume<Cube16>, p: IVec3, f: Face| catalog.connected(v, ch, p, f);
        let mut removal = EditBatch::new();
        catalog
            .remove(&mut removal, &volume, ch, IVec3::new(2, 0, 0))
            .unwrap();
        volume.apply(&removal, EditOptions::default());
        let split = split_check(&volume, &[IVec3::new(2, 0, 0)], 10_000, solid, connected).unwrap();
        assert_eq!(split.groups.len(), 2);
        assert_eq!(split.groups[0].len(), 7, "largest first");
        assert_eq!(
            split.groups[1],
            vec![IVec3::new(0, 0, 0), IVec3::new(1, 0, 0)]
        );
        // Removing an end cell leaves one piece.
        let mut tail = EditBatch::new();
        catalog
            .remove(&mut tail, &volume, ch, IVec3::new(9, 0, 0))
            .unwrap();
        volume.apply(&tail, EditOptions::default());
        assert!(split_check(&volume, &[IVec3::new(9, 0, 0)], 10_000, solid, connected).is_none());
        let piece = volume.extract_cells(&split.groups[1]);
        assert_eq!(piece.get(block, IVec3::ZERO), armor);
        assert_eq!(volume.get(block, IVec3::ZERO), AIR);
        let truncated = components(&volume, &[IVec3::new(5, 0, 0)], 3, solid, connected);
        assert!(truncated.truncated);
    }
}
