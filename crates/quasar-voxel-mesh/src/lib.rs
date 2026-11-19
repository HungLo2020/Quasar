//! Renderer-neutral voxel extraction.
//!
//! Extractors read padded [`quasar_voxel::Snapshot`]s and write into a
//! [`MeshSink`] in deterministic order:
//!
//! * [`CulledModelExtractor`]: per-block model quads with injectable
//!   [`ModelSource`], [`FacePolicy`], [`ShadeSource`], [`LayerRouter`] and
//!   [`IterationOrder`]. The path for games that must reproduce a reference
//!   mesher exactly.
//! * [`GreedyExtractor`]: binary greedy meshing of full cubes.
//! * [`SurfaceNetsExtractor`]: smooth density surfaces with material blending
//!   and LOD seam skirts.
//!
//! Output encodes to compact GPU formats ([`PackedQuad`], [`ModelVertex`],
//! [`PackedSmoothVertex`]) with matching WGSL decoders in [`WGSL_FORMATS`];
//! translucent quads sort with a [`TranslucentSorter`].
mod bevel;
mod culled;
mod format;
mod greedy;
mod quad;
#[cfg(feature = "jobs")]
mod queue;
mod smooth;
mod sort;

pub use bevel::{BevelExtractor, BevelInput, BevelOptions};
pub use culled::{
    CatalogFaces, CatalogLayers, CatalogModels, CellInfo, CulledInput, CulledModelExtractor,
    ExtractStats, FacePolicy, IterationOrder, LayerRouter, ModelLibrary, ModelQuad, ModelSource,
    ShadeInput, ShadeSource, VertexShade, box_quads, cube_face, cube_quad, opaque_table,
};
pub use format::{
    GpuLayer, ModelVertex, POSITION_BIAS, POSITION_SCALE, PackError, PackedQuad,
    PackedSmoothVertex, UV_SCALE, WGSL_FORMATS, canonical_corners, canonical_swaps, oct_decode,
    oct_encode, packed_uv,
};
pub use greedy::{
    CatalogGreedy, GREEDY_MAX_MATERIAL, GreedyExtractor, GreedyMaterials, GreedyOptions,
};
pub use quad::{
    CountingSink, LayerBuffers, MeshBuffers, MeshSink, Quad, SmoothVertex, record_mesh,
};
#[cfg(feature = "jobs")]
pub use queue::RemeshQueue;
pub use smooth::{SmoothOptions, SmoothStats, SurfaceNetsExtractor};
pub use sort::{CentroidSorter, PlaneSorter, TranslucentSorter};
