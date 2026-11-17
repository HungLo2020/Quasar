//! Renderer-neutral voxel volumes for block worlds, smooth deformable
//! terrain and block grids.
//!
//! `quasar-voxel` is the core of Quasar's voxel stack (see
//! `docs/voxel-spec.md`). It owns data, queries and edits; extraction,
//! visibility, streaming and rendering live in `quasar-voxel-mesh`,
//! `quasar-voxel-view` and `quasar-render-voxel`.
//!
//! * **Layouts:** cubic chunks with a power-of-two edge fixed at compile
//!   time ([`Cube16`], [`Cube32`], ...), monomorphised index orders.
//! * **Channels:** a [`Schema`] of typed per-voxel attributes, each stored
//!   uniform, paletted (with a [`PalettePolicy`] for exact external formats),
//!   dense, nibble-packed or sparse.
//! * **Volumes and grids:** deterministic chunk maps with procedural
//!   [`VoxelSource`]s and edit deltas; [`Grid`]s place volumes in the world
//!   with `f64` transforms; a [`Scene`] holds many grids.
//! * **Block catalogs:** multi-cell, 24-orientation blocks with per-face
//!   shapes and mount points.
//! * **Edits:** ordered [`EditBatch`]es with density brushes, change events
//!   and dirty-chunk tracking; deterministic [`codec`] encodings.
//! * **Queries:** DDA raycasts with tie-break policy, density hits,
//!   AABB overlap and sweep.
//! * **Propagation** and **connectivity/splitting** with pluggable rules.
//!
//! ```
//! use quasar_voxel::{Cube16, EditBatch, EditOptions, Schema, StorageKind, TieBreak, Volume};
//! use quasar_math::{IVec3, Vec3};
//!
//! let mut schema = Schema::new();
//! let block = schema.channel("block", StorageKind::Paletted, 0u16);
//! let mut world = Volume::<Cube16>::new(schema);
//! let mut batch = EditBatch::new();
//! batch.set(block, IVec3::new(4, 0, 0), 1);
//! let result = world.apply(&batch, EditOptions::default());
//! assert!(result.dirty.contains(&IVec3::ZERO));
//! let hit = world
//!     .raycast(block, Vec3::new(0.5, 0.5, 0.5), Vec3::X, 16.0, TieBreak::default(), |_, v| v != 0)
//!     .unwrap();
//! assert_eq!(hit.voxel, IVec3::new(4, 0, 0));
//! ```
mod catalog;
mod chunk;
pub mod codec;
pub mod conformance;
mod connect;
mod edit;
mod grid;
mod layout;
mod orient;
mod propagate;
mod query;
mod storage;
mod volume;

pub use catalog::{
    AIR, BlockCatalog, BlockChannels, BlockDef, FaceShape, Layer, PlaceError, UNIT_BOUNDS,
    box_face_shape,
};
pub use chunk::{ChannelData, ChannelDef, ChannelId, Chunk, RawKind, Schema};
pub use connect::{Components, components, split_check};
pub use edit::{
    Brush, BrushOp, BrushShape, Change, DENSITY_SCALE, EditBatch, EditOp, EditOptions, EditResult,
    quantize_density,
};
pub use grid::{
    DensityGrid, Grid, GridId, GridKind, GridTransform, Scene, SceneGrid, SceneHit, SolidGrid,
};
pub use layout::{
    ChunkLayout, Cube, Cube8, Cube16, Cube32, Cube64, Face, IndexOrder, Xyz, Xzy, Yzx,
};
pub use orient::Orientation;
pub use propagate::{PropagationRules, PropagationStats, Propagator, QueueDiscipline, sky_seeds};
pub use query::{CellHit, CellVisit, RayHit, TieBreak, traverse};
pub use storage::{
    BitPacking, FastPalette, PackedWords, PaletteLayout, PalettePolicy, Paletted, Raw, Sparse,
    Storage, StorageKind, ThresholdPalette, Value,
};
pub use volume::{Cursor, FxHashMap, FxHasher, Snapshot, Volume, VoxelSource};
