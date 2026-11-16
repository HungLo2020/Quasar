//! Renderer-neutral planning for voxel worlds: what is visible, what to
//! stream, which LOD to show, and where geometry lives on the GPU.
//!
//! * [`visibility`]: per-chunk face connectivity and a policy-driven
//!   traversal (cave culling) with frustum and range tests.
//! * [`streaming`]: ordered load/unload plans around viewers with budgets.
//! * [`lod`]: octree/clipmap node selection with hysteresis and seam flags,
//!   density sampling and majority downsampling for coarse levels.
//! * [`residency`]: best-fit range allocation with deferred frees and pages.
//! * [`draw`]: per-chunk draw records and wgpu-layout indirect commands.
//!
//! Nothing here creates GPU objects; `quasar-render-voxel` (or a game's own
//! renderer) does the uploads and draws.
pub mod draw;
pub mod lod;
pub mod residency;
pub mod streaming;
pub mod visibility;

pub use draw::{DrawBatch, DrawIndexedIndirect, DrawIndirect, DrawKey, DrawList, DrawRecord};
pub use lod::{LodConfig, LodNode, LodSelector, SelectedNode, downsample_majority, sample_lod};
pub use residency::{Allocation, PageAllocation, PagedArena, RangeAllocator};
pub use streaming::{StreamBudget, StreamPlan, StreamingPlanner, Viewer};
pub use visibility::{
    ChunkFrustum, Connectivity, ConnectivityPolicy, FrustumPolicy, TraversalPolicy,
    VisibilityGraph, chunk_connectivity,
};
