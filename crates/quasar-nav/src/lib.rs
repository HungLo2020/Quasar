//! Reusable A*/Dijkstra pathfinding, grid navigation and optional adapters.
//!
//! `quasar-nav` owns no agents, steering, ECS, renderer or global world. Games
//! own graphs/grids, decision-making and movement; this crate returns paths.
//!
//! ```
//! use quasar_nav::{Grid, GridCell, SearchOptions};
//!
//! let mut grid = Grid::<2>::new([8, 8])?;
//! grid.set_blocked(GridCell::new2(3, 4), true)?;
//! let path = grid
//!     .find_path(GridCell::new2(0, 0), GridCell::new2(7, 7), SearchOptions::default())
//!     .path()
//!     .unwrap();
//! assert_eq!(path.nodes.first(), Some(&GridCell::new2(0, 0)));
//! assert_eq!(path.nodes.last(), Some(&GridCell::new2(7, 7)));
//! # Ok::<(), quasar_nav::NavError>(())
//! ```
//!
//! # Determinism
//!
//! Open-set ties are broken by node order and insertion sequence; grid neighbor
//! order is fixed. Paths are therefore deterministic for an unchanged graph and
//! option set. No wall clock or global state is used. Random tie-breaking is not
//! provided intentionally.
//!
//! # Large searches
//!
//! `SearchOptions` bounds visited nodes and total cost and accepts an atomic
//! cancellation flag. The synchronous search is the default; the optional
//! `jobs` feature submits grid searches to `quasar-jobs`.
mod error;
mod graph;
mod grid;
#[cfg(feature = "jobs")]
mod jobs;
#[cfg(feature = "navmesh")]
pub mod navmesh_support;
#[cfg(feature = "physics3d")]
pub mod physics3d;
#[cfg(feature = "tilemap")]
mod tilemap;

pub use error::{NavError, NavResult};
pub use graph::{
    Graph, Neighbor, Path, SearchOptions, SearchOutcome, astar, dijkstra, smooth_path,
};
pub use grid::{Grid, GridCell};
#[cfg(feature = "jobs")]
pub use jobs::spawn_grid_path;
#[cfg(feature = "tilemap")]
pub use tilemap::grid_from_tilemap;

/// Convenience alias for a two-dimensional grid.
pub type Grid2 = Grid<2>;
/// Convenience alias for a three-dimensional/voxel grid.
pub type Grid3 = Grid<3>;
/// Convenience alias for a two-dimensional cell.
pub type Cell2 = GridCell<2>;
/// Convenience alias for a three-dimensional cell.
pub type Cell3 = GridCell<3>;
