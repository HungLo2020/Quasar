//! Optional `quasar-jobs` offload for synchronous grid searches.
use crate::graph::{SearchOptions, SearchOutcome};
use crate::grid::{Grid, GridCell};
use quasar_jobs::{JobError, JobHandle, JobPool};

/// Submits one 2D grid search to a caller-owned job pool.
///
/// The job is deterministic and uses the same `SearchOptions` cancellation flag
/// as the synchronous API. Pool-level cancellation is handled by
/// `quasar-jobs`; the returned handle owns the result.
pub fn spawn_grid_path(
    pool: &JobPool,
    grid: Grid<2>,
    start: GridCell<2>,
    goal: GridCell<2>,
    options: SearchOptions,
) -> Result<JobHandle<SearchOutcome<GridCell<2>>>, JobError> {
    pool.spawn(move |_| Ok::<_, std::convert::Infallible>(grid.find_path(start, goal, options)))
}
