//! Per-chunk draw records and indirect command arrays.
//!
//! Commands use wgpu's indirect layouts (`DrawIndirectArgs` /
//! `DrawIndexedIndirectArgs`), so the bytes upload directly into an indirect
//! buffer for `multi_draw_*_indirect`, or drive a per-draw loop where
//! multi-draw is unavailable.
use quasar_math::{IVec3, Vec3};
use quasar_voxel::{GridId, Layer};

/// Layout of a non-indexed indirect draw (`DrawIndirectArgs` in the GPU API).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct DrawIndirect {
    pub vertex_count: u32,
    pub instance_count: u32,
    pub first_vertex: u32,
    pub first_instance: u32,
}

/// Layout of an indexed indirect draw (`DrawIndexedIndirectArgs` in the GPU API).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct DrawIndexedIndirect {
    pub index_count: u32,
    pub instance_count: u32,
    pub first_index: u32,
    pub base_vertex: i32,
    pub first_instance: u32,
}

/// Identity of one drawable piece of voxel geometry.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DrawKey {
    pub grid: GridId,
    /// Chunk or LOD node coordinate.
    pub chunk: IVec3,
    /// LOD level (0 for chunks).
    pub level: u32,
    pub layer: Layer,
    /// Geometry stream (game-defined, e.g. 0 = packed quads, 1 = model, 2 = smooth).
    pub stream: u8,
}

impl DrawKey {
    fn order(&self) -> (GridId, (i32, i32, i32), u32, Layer, u8) {
        (
            self.grid,
            (self.chunk.x, self.chunk.y, self.chunk.z),
            self.level,
            self.layer,
            self.stream,
        )
    }
}
impl Ord for DrawKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.order().cmp(&other.order())
    }
}
impl PartialOrd for DrawKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Geometry placement and bounds for one key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawRecord {
    pub key: DrawKey,
    pub page: u32,
    /// First element (vertex, quad or index) in the page.
    pub first: u32,
    pub count: u32,
    /// Base vertex for indexed draws.
    pub base_vertex: i32,
    /// Local bounds in grid voxel units.
    pub min: Vec3,
    pub max: Vec3,
    /// Per-draw instance slot (chunk origin / transform index).
    pub instance: u32,
}

/// Records grouped for drawing: per page and stream, in key order.
#[derive(Clone, Debug, Default)]
pub struct DrawList {
    records: std::collections::BTreeMap<DrawKey, DrawRecord>,
}

impl DrawList {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn insert(&mut self, record: DrawRecord) -> Option<DrawRecord> {
        self.records.insert(record.key, record)
    }
    pub fn remove(&mut self, key: &DrawKey) -> Option<DrawRecord> {
        self.records.remove(key)
    }
    pub fn get(&self, key: &DrawKey) -> Option<&DrawRecord> {
        self.records.get(key)
    }
    pub fn len(&self) -> usize {
        self.records.len()
    }
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
    pub fn records(&self) -> impl Iterator<Item = &DrawRecord> {
        self.records.values()
    }
    /// Removes every record of a chunk (all layers, streams and levels).
    pub fn remove_chunk(&mut self, grid: GridId, chunk: IVec3) -> Vec<DrawRecord> {
        let keys: Vec<DrawKey> = self
            .records
            .keys()
            .filter(|k| k.grid == grid && k.chunk == chunk)
            .copied()
            .collect();
        keys.into_iter()
            .filter_map(|k| self.records.remove(&k))
            .collect()
    }
    /// Builds indirect commands for records accepted by `visible`, grouped by
    /// `(layer, stream, page)` in key order. `vertices_per_element` expands
    /// element counts (6 for quad-pulling streams, 1 for indexed/vertex lists).
    pub fn build(
        &self,
        mut visible: impl FnMut(&DrawRecord) -> bool,
        mut vertices_per_element: impl FnMut(u8) -> u32,
    ) -> Vec<DrawBatch> {
        let mut batches: Vec<DrawBatch> = Vec::new();
        let mut sorted: Vec<&DrawRecord> = self.records.values().filter(|r| visible(r)).collect();
        sorted.sort_by_key(|r| (r.key.layer, r.key.stream, r.page, r.key));
        for record in sorted {
            let group = (record.key.layer, record.key.stream, record.page);
            if batches
                .last()
                .is_none_or(|b| (b.layer, b.stream, b.page) != group)
            {
                batches.push(DrawBatch {
                    layer: record.key.layer,
                    stream: record.key.stream,
                    page: record.page,
                    draws: Vec::new(),
                    indexed: Vec::new(),
                    keys: Vec::new(),
                });
            }
            let batch = batches.last_mut().unwrap();
            let per = vertices_per_element(record.key.stream);
            batch.draws.push(DrawIndirect {
                vertex_count: record.count * per,
                instance_count: 1,
                first_vertex: record.first * per,
                first_instance: record.instance,
            });
            batch.indexed.push(DrawIndexedIndirect {
                index_count: record.count,
                instance_count: 1,
                first_index: record.first,
                base_vertex: record.base_vertex,
                first_instance: record.instance,
            });
            batch.keys.push(record.key);
        }
        batches
    }
}

/// Indirect commands for one `(layer, stream, page)` group.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawBatch {
    pub layer: Layer,
    pub stream: u8,
    pub page: u32,
    pub draws: Vec<DrawIndirect>,
    pub indexed: Vec<DrawIndexedIndirect>,
    pub keys: Vec<DrawKey>,
}

impl DrawBatch {
    /// Non-indexed commands as bytes for an indirect buffer.
    pub fn draw_bytes(&self) -> Vec<u8> {
        self.draws
            .iter()
            .flat_map(|d| {
                [
                    d.vertex_count,
                    d.instance_count,
                    d.first_vertex,
                    d.first_instance,
                ]
            })
            .flat_map(u32::to_le_bytes)
            .collect()
    }
    /// Indexed commands as bytes for an indirect buffer.
    pub fn indexed_bytes(&self) -> Vec<u8> {
        self.indexed
            .iter()
            .flat_map(|d| {
                [
                    d.index_count,
                    d.instance_count,
                    d.first_index,
                    d.base_vertex as u32,
                    d.first_instance,
                ]
            })
            .flat_map(u32::to_le_bytes)
            .collect()
    }
}
