//! Many-agents-to-goal distance (flow) fields over [`Grid`].
//!
//! A [`DistanceField`] stores the cost to the nearest goal for every cell,
//! computed by one multi-source Dijkstra search. Every agent then steers by
//! looking up [`DistanceField::next_cell`] or [`DistanceField::gradient`] for
//! its own cell, which is the standard horde-AI primitive: one search serves
//! any number of agents. When terrain changes, [`DistanceField::update_region`]
//! repairs only the cells whose shortest route crossed the edited cells.
//!
//! Costs follow [`Grid::find_path`]: stepping into a cell costs that cell's
//! cost, multiplied by √2 for diagonal moves, and blocked cells are never
//! entered. The field owns no agents or steering policy.
use crate::error::{NavError, NavResult};
use crate::grid::{Grid, GridCell};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};

const NO_PARENT: u32 = u32::MAX;

/// Limits for [`Grid::distance_field`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldOptions {
    /// Cells costlier than this stay unreachable. Bounds work for local
    /// fields such as "flee within 20 tiles".
    pub max_cost: f32,
}

impl Default for FieldOptions {
    fn default() -> Self {
        Self {
            max_cost: f32::INFINITY,
        }
    }
}

impl FieldOptions {
    pub const fn with_max_cost(max_cost: f32) -> Self {
        Self { max_cost }
    }
}

/// Cost-to-goal for every grid cell. See the module docs.
#[derive(Clone, Debug)]
pub struct DistanceField<const N: usize> {
    dimensions: [u32; N],
    goals: Vec<GridCell<N>>,
    options: FieldOptions,
    distances: Vec<f32>,
    /// Index of the neighbour each cell steps to, or `NO_PARENT`.
    parents: Vec<u32>,
    grid_version: u64,
}

#[derive(Clone, Copy, PartialEq)]
struct Open {
    cost: f32,
    index: u32,
}
impl Eq for Open {}
impl Ord for Open {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap by cost, ties by index for determinism.
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| other.index.cmp(&self.index))
    }
}
impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Neighbour offsets in the same fixed order as `Grid::neighbors_into`.
fn offsets<const N: usize>() -> Vec<([i32; N], usize)> {
    let count = 3usize.pow(N as u32);
    let mut out = Vec::with_capacity(count - 1);
    for linear in 0..count {
        let mut digits = linear;
        let mut offset = [0i32; N];
        let mut nonzero = 0;
        for value in &mut offset {
            let digit = digits % 3;
            digits /= 3;
            *value = digit as i32 - 1;
            nonzero += usize::from(digit != 1);
        }
        if nonzero > 0 {
            out.push((offset, nonzero));
        }
    }
    out
}

impl<const N: usize> Grid<N> {
    /// Multi-source Dijkstra from `goals`. Blocked or out-of-bounds goals are
    /// ignored; with no usable goal every cell is unreachable.
    pub fn distance_field(
        &self,
        goals: &[GridCell<N>],
        options: &FieldOptions,
    ) -> NavResult<DistanceField<N>> {
        if options.max_cost.is_nan() || options.max_cost < 0.0 {
            return Err(NavError::InvalidCost);
        }
        if self.len() > NO_PARENT as usize {
            return Err(NavError::InvalidDimensions);
        }
        let mut field = DistanceField {
            dimensions: self.dimensions(),
            goals: goals.to_vec(),
            options: *options,
            distances: vec![f32::INFINITY; self.len()],
            parents: vec![NO_PARENT; self.len()],
            grid_version: self.version(),
        };
        let mut heap = BinaryHeap::new();
        field.seed_goals(self, None, &mut heap);
        field.propagate(self, heap);
        Ok(field)
    }

    /// Cost of the move from `from` by `offset`, mirroring
    /// `neighbors_into`: `None` when the target is out of bounds or blocked,
    /// the move is diagonal and diagonals are off, or it cuts a blocked
    /// corner.
    fn step_cost(
        &self,
        from: GridCell<N>,
        offset: &[i32; N],
        nonzero: usize,
    ) -> Option<(usize, f32)> {
        if nonzero > 1 && !self.diagonal_enabled() {
            return None;
        }
        let mut target = from.coords;
        for axis in 0..N {
            target[axis] += offset[axis];
        }
        let target_index = self.index(GridCell::new(target)).ok()?;
        if self.blocked_at(target_index) {
            return None;
        }
        if nonzero > 1 && !self.corner_cutting_enabled() {
            let cut = (0..N).any(|axis| {
                if offset[axis] == 0 {
                    return false;
                }
                let mut intermediate = from.coords;
                intermediate[axis] += offset[axis];
                self.is_blocked(GridCell::new(intermediate))
            });
            if cut {
                return None;
            }
        }
        let step = if nonzero > 1 {
            std::f32::consts::SQRT_2
        } else {
            1.0
        };
        Some((target_index, self.cost_at(target_index) * step))
    }
}

impl<const N: usize> DistanceField<N> {
    pub fn dimensions(&self) -> [u32; N] {
        self.dimensions
    }
    pub fn goals(&self) -> &[GridCell<N>] {
        &self.goals
    }
    pub fn options(&self) -> FieldOptions {
        self.options
    }
    /// The grid version the field was last built or repaired against.
    pub fn grid_version(&self) -> u64 {
        self.grid_version
    }
    /// Whether `grid` changed since the field was built or last repaired.
    pub fn is_stale(&self, grid: &Grid<N>) -> bool {
        grid.version() != self.grid_version || grid.dimensions() != self.dimensions
    }
    /// Raw per-cell costs in grid index order; unreachable cells are infinite.
    pub fn distances(&self) -> &[f32] {
        &self.distances
    }

    fn index(&self, cell: GridCell<N>) -> Option<usize> {
        let mut index = 0usize;
        let mut stride = 1usize;
        for axis in 0..N {
            let coordinate = cell.coords[axis];
            if coordinate < 0 || coordinate as u32 >= self.dimensions[axis] {
                return None;
            }
            index += coordinate as usize * stride;
            stride *= self.dimensions[axis] as usize;
        }
        Some(index)
    }

    /// Cost from `cell` to its nearest goal, or `None` when unreachable or
    /// out of bounds.
    pub fn distance(&self, cell: GridCell<N>) -> Option<f32> {
        self.index(cell)
            .map(|index| self.distances[index])
            .filter(|distance| distance.is_finite())
    }

    /// The neighbour an agent at `cell` should step to, or `None` at a goal,
    /// at an unreachable cell or out of bounds.
    pub fn next_cell(&self, grid: &Grid<N>, cell: GridCell<N>) -> Option<GridCell<N>> {
        let index = self.index(cell)?;
        let parent = self.parents[index];
        if parent == NO_PARENT {
            return None;
        }
        grid.cell(parent as usize)
    }

    /// Unit direction from `cell` towards its next cell, or `None` where
    /// [`Self::next_cell`] is `None`.
    pub fn gradient(&self, grid: &Grid<N>, cell: GridCell<N>) -> Option<[f32; N]> {
        let next = self.next_cell(grid, cell)?;
        let mut direction: [f32; N] =
            std::array::from_fn(|axis| (next.coords[axis] - cell.coords[axis]) as f32);
        let length = direction.iter().map(|v| v * v).sum::<f32>().sqrt();
        if length == 0.0 {
            return None;
        }
        for value in &mut direction {
            *value /= length;
        }
        Some(direction)
    }

    /// Follows the field from `start` to a goal (inclusive). Empty when
    /// `start` is unreachable.
    pub fn path_from(&self, grid: &Grid<N>, start: GridCell<N>) -> Vec<GridCell<N>> {
        if self.distance(start).is_none() {
            return Vec::new();
        }
        let mut path = vec![start];
        let mut current = start;
        while let Some(next) = self.next_cell(grid, current) {
            path.push(next);
            current = next;
            if path.len() > self.distances.len() {
                break;
            }
        }
        path
    }

    /// Replaces the goal set and rebuilds. Use [`Self::update_region`] for
    /// terrain edits that keep the goals.
    pub fn set_goals(&mut self, grid: &Grid<N>, goals: &[GridCell<N>]) -> NavResult<()> {
        *self = grid.distance_field(goals, &self.options)?;
        Ok(())
    }

    fn seed_goals(&mut self, grid: &Grid<N>, only: Option<&[bool]>, heap: &mut BinaryHeap<Open>) {
        for goal in &self.goals {
            let Ok(index) = grid.index(*goal) else {
                continue;
            };
            if grid.blocked_at(index) || only.is_some_and(|mask| !mask[index]) {
                continue;
            }
            if self.distances[index] > 0.0 {
                self.distances[index] = 0.0;
                self.parents[index] = NO_PARENT;
                heap.push(Open {
                    cost: 0.0,
                    index: index as u32,
                });
            }
        }
    }

    /// Reverse Dijkstra: a popped cell `n` offers `dist[n] + cost(c -> n)` to
    /// every cell `c` that may step into it.
    fn propagate(&mut self, grid: &Grid<N>, mut heap: BinaryHeap<Open>) -> usize {
        let offsets = offsets::<N>();
        let mut settled = 0;
        let max_cost = self.options.max_cost;
        while let Some(Open { cost, index }) = heap.pop() {
            let index = index as usize;
            if cost > self.distances[index] {
                continue;
            }
            settled += 1;
            // Every cell `c` with a legal move into `index` is a geometric
            // neighbour; test the move from `c` itself because corner rules
            // are not symmetric in 3D.
            let Some(cell) = grid.cell(index) else {
                continue;
            };
            for (offset, nonzero) in &offsets {
                let mut coords = cell.coords;
                for axis in 0..N {
                    coords[axis] += offset[axis];
                }
                let from = GridCell::new(coords);
                let Ok(neighbor) = grid.index(from) else {
                    continue;
                };
                if grid.blocked_at(neighbor) {
                    continue;
                }
                let back = offset.map(|value| -value);
                let Some((_, step)) = grid.step_cost(from, &back, *nonzero) else {
                    continue;
                };
                let candidate = cost + step;
                if candidate > max_cost || candidate >= self.distances[neighbor] {
                    continue;
                }
                self.distances[neighbor] = candidate;
                self.parents[neighbor] = index as u32;
                heap.push(Open {
                    cost: candidate,
                    index: neighbor as u32,
                });
            }
        }
        settled
    }

    /// Repairs the field after `dirty` cells changed blocking or cost in
    /// `grid`, returning how many cells were re-settled.
    ///
    /// Only cells whose stored route passed through or diagonally around a
    /// dirty cell are invalidated; the rest keep their distances, and
    /// improvements (unblocked or cheaper cells) propagate outwards from the
    /// edit. The result equals a full rebuild. Dimension changes are rejected
    /// with [`NavError::CellCountMismatch`].
    pub fn update_region(&mut self, grid: &Grid<N>, dirty: &[GridCell<N>]) -> NavResult<usize> {
        if grid.dimensions() != self.dimensions {
            return Err(NavError::CellCountMismatch);
        }
        let offsets = offsets::<N>();
        let len = self.distances.len();
        let mut invalid = vec![false; len];
        let mut queue = VecDeque::new();
        let mark = |index: usize, invalid: &mut Vec<bool>, queue: &mut VecDeque<usize>| {
            if !invalid[index] {
                invalid[index] = true;
                queue.push_back(index);
            }
        };
        let neighborhood = |index: usize| -> Vec<usize> {
            let Some(cell) = grid.cell(index) else {
                return Vec::new();
            };
            offsets
                .iter()
                .filter_map(|(offset, _)| {
                    let mut target = cell.coords;
                    for axis in 0..N {
                        target[axis] += offset[axis];
                    }
                    grid.index(GridCell::new(target)).ok()
                })
                .collect()
        };
        for cell in dirty {
            let index = grid.index(*cell)?;
            mark(index, &mut invalid, &mut queue);
            // A newly blocked cell can forbid diagonal moves that cut its
            // corner: invalidate neighbours whose step stays in this box.
            let around = neighborhood(index);
            for &neighbor in &around {
                let parent = self.parents[neighbor];
                if parent != NO_PARENT && around.contains(&(parent as usize)) {
                    mark(neighbor, &mut invalid, &mut queue);
                }
            }
        }
        // Invalidate every descendant in the shortest-route tree.
        while let Some(index) = queue.pop_front() {
            for neighbor in neighborhood(index) {
                if self.parents[neighbor] == index as u32 {
                    mark(neighbor, &mut invalid, &mut queue);
                }
            }
        }
        for (index, invalid) in invalid.iter().enumerate() {
            if *invalid {
                self.distances[index] = f32::INFINITY;
                self.parents[index] = NO_PARENT;
            }
        }
        let mut heap = BinaryHeap::new();
        self.seed_goals(grid, Some(&invalid), &mut heap);
        // Valid cells bordering the invalid region (and the edited cells'
        // neighbours, which may now improve) re-offer their distances.
        let mut offered = vec![false; len];
        for index in (0..len).filter(|index| invalid[*index]) {
            for neighbor in neighborhood(index) {
                if !invalid[neighbor] && !offered[neighbor] && self.distances[neighbor].is_finite()
                {
                    offered[neighbor] = true;
                    heap.push(Open {
                        cost: self.distances[neighbor],
                        index: neighbor as u32,
                    });
                }
            }
        }
        let settled = self.propagate(grid, heap);
        self.grid_version = grid.version();
        Ok(settled)
    }
}
