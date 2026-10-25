//! Reusable, renderer-independent spatial indexing and geometric queries.
//!
//! The crate owns no world, entity IDs, ECS, renderer or scene graph. Callers
//! choose an ordered key type and a coordinate system. A [`SpatialGrid`] is a
//! deterministic spatial hash with `f64` bounds; [`Ray`], [`Plane`] and
//! [`Frustum`] query helpers are usable without an index.
//!
//! ```
//! use quasar_spatial::{Aabb, GridConfig, SpatialGrid};
//!
//! let mut grid = SpatialGrid::<2, u32>::new(GridConfig::default()).unwrap();
//! grid.insert(7, Aabb::new([0.0, 0.0], [2.0, 2.0]).unwrap());
//! assert_eq!(grid.query(Aabb::point([1.0, 1.0]).unwrap()), vec![7]);
//! assert_eq!(grid.nearest([3.0, 3.0], 5.0).unwrap().id, 7);
//! ```
//!
//! # Precision and world origin
//!
//! Coordinates are `f64` and the grid computes cell coordinates with `floor`.
//! The floating-point exponent gives a much larger usable range than an `f32`
//! world while retaining sub-millimetre precision near ordinary game scales.
//! Very large coordinates still lose absolute precision; games that need
//! camera-relative or origin-shifted worlds should apply that policy before
//! inserting. Cell coordinates are checked conversions to `i64`; an AABB whose
//! cells cannot be represented is kept in the bounded "large item" list rather
//! than overflowing. Queries test large items linearly, preserving correctness
//! with bounded memory.
//!
//! # Determinism
//!
//! All result lists are sorted by the caller's key (or by distance then key for
//! nearest/radius queries). Internal `HashMap` iteration order never leaks into
//! public results. [`SpatialGrid::iter`] is ordered by key.
mod geometry;
mod grid;

pub use geometry::{Aabb, Frustum, Plane, Ray};
pub use grid::{
    GridConfig, GridMemory, Neighbor, RayHit, SpatialError, SpatialGrid, SpatialResult,
};

/// Convenience alias for a two-dimensional AABB.
pub type Aabb2 = Aabb<2>;
/// Convenience alias for a three-dimensional AABB.
pub type Aabb3 = Aabb<3>;
/// Convenience alias for a two-dimensional ray.
pub type Ray2 = Ray<2>;
/// Convenience alias for a three-dimensional ray.
pub type Ray3 = Ray<3>;
/// Convenience alias for a two-dimensional frustum.
pub type Frustum2 = Frustum<2>;
/// Convenience alias for a three-dimensional frustum.
pub type Frustum3 = Frustum<3>;
/// Convenience alias for a two-dimensional grid.
pub type SpatialGrid2<T> = SpatialGrid<2, T>;
/// Convenience alias for a three-dimensional grid.
pub type SpatialGrid3<T> = SpatialGrid<3, T>;
