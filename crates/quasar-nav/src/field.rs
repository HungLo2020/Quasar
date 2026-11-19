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
    /// Reused search queue (empty between calls).
    heap: BinaryHeap<Open>,
    /// Reused bucket queue for full rebuilds.
    buckets: Vec<Vec<Open>>,
    /// Cells whose distance changed during an incremental goal change.
    touched: Vec<u32>,
    record_touched: bool,
    /// Boundary masks for `dimensions` (see `side_masks`).
    sides: std::sync::Arc<[u64]>,
    /// Legal-move masks for the grid version they were built from.
    legal: Option<std::sync::Arc<LegalMasks>>,
}

/// Per cell, which of the (at most 32) moves are legal: `into` bit `m` is
/// set when `cell + offset[m]` may step into `cell`; `out` bit `m` when
/// `cell` may step to `cell + offset[m]`. Cached per grid version so
/// repeated goal changes on unchanged terrain skip every bounds, blocking
/// and corner test.
#[derive(Debug)]
struct LegalMasks {
    version: u64,
    len: usize,
    into: Vec<u32>,
    out: Vec<u32>,
}

/// Grids up to this many cells cache legal-move masks (8 bytes per cell).
const LEGAL_MASK_CELLS: usize = 1 << 21;

#[derive(Clone, Copy, Debug, PartialEq)]
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

/// One legal move direction with its precomputed linear index delta.
#[derive(Clone, Copy)]
struct Move<const N: usize> {
    /// `index(cell + offset) - index(cell)`.
    linear: isize,
    /// 1 or √2.
    multiplier: f32,
    /// Linear deltas from the move's source to the cells whose corner it
    /// cuts (only when corner cutting is off).
    corners: [isize; N],
    corner_count: u8,
    /// Boundary bits a cell must not have for this move to stay in bounds
    /// (see [`side_masks`]).
    sides: u64,
}

fn strides<const N: usize>(dimensions: [u32; N]) -> [isize; N] {
    let mut strides = [0isize; N];
    let mut stride = 1isize;
    for axis in 0..N {
        strides[axis] = stride;
        stride *= dimensions[axis] as isize;
    }
    strides
}

/// Neighbour moves in `offsets` order, without directions that can never be
/// legal on `grid` (diagonals when disabled, steps along a size-1 axis).
fn moves<const N: usize>(grid: &Grid<N>) -> Vec<Move<N>> {
    assert!(N <= 32, "distance fields support at most 32 dimensions");
    let dimensions = grid.dimensions();
    let strides = strides(dimensions);
    offsets::<N>()
        .into_iter()
        .filter(|(offset, nonzero)| {
            (*nonzero == 1 || grid.diagonal_enabled())
                && (0..N).all(|axis| offset[axis] == 0 || dimensions[axis] > 1)
        })
        .map(|(offset, nonzero)| {
            let linear = (0..N)
                .map(|axis| offset[axis] as isize * strides[axis])
                .sum();
            let mut corners = [0isize; N];
            let mut corner_count = 0u8;
            if nonzero > 1 && !grid.corner_cutting_enabled() {
                for axis in 0..N {
                    if offset[axis] != 0 {
                        corners[corner_count as usize] = offset[axis] as isize * strides[axis];
                        corner_count += 1;
                    }
                }
            }
            let mut sides = 0u64;
            for (axis, value) in offset.iter().enumerate() {
                match value {
                    -1 => sides |= 1 << (2 * axis),
                    1 => sides |= 1 << (2 * axis + 1),
                    _ => {}
                }
            }
            Move {
                linear,
                sides,
                multiplier: if nonzero > 1 {
                    std::f32::consts::SQRT_2
                } else {
                    1.0
                },
                corners,
                corner_count,
            }
        })
        .collect()
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
            heap: BinaryHeap::new(),
            buckets: Vec::new(),
            touched: Vec::new(),
            record_touched: false,
            sides: std::sync::Arc::from(Vec::new()),
            legal: None,
        };
        field.rebuild(self);
        Ok(field)
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

    /// Replaces the goal set, repairing the field in place.
    ///
    /// Added goals only lower distances, so they propagate outwards from the
    /// new goals; removed goals invalidate just the cells whose route ended
    /// at them, which are then re-settled from the surrounding cells (a full
    /// in-place search when most of the field is affected, for example when
    /// a single goal moves). Buffers are reused either way. Distances equal
    /// a fresh [`Grid::distance_field`]; where two routes tie,
    /// [`Self::next_cell`] may pick a different (equally short) neighbour.
    /// A stale field (the grid changed since it was built) is rebuilt.
    pub fn set_goals(&mut self, grid: &Grid<N>, goals: &[GridCell<N>]) -> NavResult<()> {
        if grid.len() > NO_PARENT as usize {
            return Err(NavError::InvalidDimensions);
        }
        if self.is_stale(grid) || self.distances.len() != grid.len() {
            self.goals = goals.to_vec();
            self.rebuild(grid);
            return Ok(());
        }
        let valid = |cell: &GridCell<N>| grid.index(*cell).ok().filter(|i| !grid.blocked_at(*i));
        let old: Vec<usize> = self.goals.iter().filter_map(valid).collect();
        let new: Vec<usize> = goals.iter().filter_map(valid).collect();
        self.goals = goals.to_vec();
        let removed: Vec<usize> = old.iter().copied().filter(|i| !new.contains(i)).collect();
        let mut heap = std::mem::take(&mut self.heap);
        heap.clear();
        self.touched.clear();
        if old.is_empty() {
            // Nothing was reachable: a plain rebuild is cheapest.
            self.heap = heap;
            self.rebuild(grid);
            return Ok(());
        }
        if removed.is_empty() {
            // Only additions: distances can only fall.
            self.record_touched = true;
            self.seed_goals(grid, None, &mut heap);
            self.propagate(grid, heap);
            self.record_touched = false;
            self.settle_touched(grid);
            return Ok(());
        }
        if removed.len() == old.len() {
            // Every route ended at a removed goal: nothing survives.
            self.heap = heap;
            self.rebuild(grid);
            return Ok(());
        }
        // Invalidate the route trees rooted at removed goals.
        let len = self.distances.len();
        let moves = moves(grid);
        let sides = self.side_table(grid);
        let mut invalid = vec![false; len];
        let mut stack: Vec<usize> = Vec::new();
        for &goal in &removed {
            if !invalid[goal] {
                invalid[goal] = true;
                stack.push(goal);
            }
        }
        let mut count = stack.len();
        while let Some(index) = stack.pop() {
            for m in &moves {
                let Some(neighbor) = step(&sides, index, m) else {
                    continue;
                };
                if self.parents[neighbor] == index as u32 && !invalid[neighbor] {
                    invalid[neighbor] = true;
                    count += 1;
                    stack.push(neighbor);
                }
            }
        }
        if count * 2 > len {
            self.heap = heap;
            self.rebuild(grid);
            return Ok(());
        }
        self.record_touched = true;
        for (index, bad) in invalid.iter().enumerate() {
            if *bad {
                self.distances[index] = f32::INFINITY;
                self.parents[index] = NO_PARENT;
                self.touched.push(index as u32);
            }
        }
        self.seed_goals(grid, None, &mut heap);
        let mut offered = vec![false; len];
        for index in (0..len).filter(|index| invalid[*index]) {
            for m in &moves {
                let Some(neighbor) = step(&sides, index, m) else {
                    continue;
                };
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
        self.propagate(grid, heap);
        self.record_touched = false;
        self.settle_touched(grid);
        Ok(())
    }

    /// Re-picks parents canonically for touched cells and every cell that
    /// can step into one, so incremental changes match a rebuild exactly.
    fn settle_touched(&mut self, grid: &Grid<N>) {
        let moves = moves(grid);
        let sides = self.side_table(grid);
        let mut cells = std::mem::take(&mut self.touched);
        let direct = cells.len();
        for i in 0..direct {
            let index = cells[i] as usize;
            for m in &moves {
                if let Some(from) = step(&sides, index, m) {
                    cells.push(from as u32);
                }
            }
        }
        cells.sort_unstable();
        cells.dedup();
        self.canonical_parents(grid, cells.iter().map(|&c| c as usize));
        cells.clear();
        self.touched = cells;
    }

    /// Recomputes every cell in place, reusing the buffers.
    fn rebuild(&mut self, grid: &Grid<N>) {
        self.dimensions = grid.dimensions();
        self.distances.clear();
        self.distances.resize(grid.len(), f32::INFINITY);
        self.parents.clear();
        self.parents.resize(grid.len(), NO_PARENT);
        self.grid_version = grid.version();
        let mut heap = std::mem::take(&mut self.heap);
        heap.clear();
        self.seed_goals(grid, None, &mut heap);
        match bucket_plan(grid) {
            Some((width, slots)) => {
                // Dial's bucket queue: O(1) queue operations instead of a
                // binary heap. Processing order inside a bucket differs from
                // the heap's, so parents are then chosen canonically, which
                // reproduces the heap search exactly.
                let seeds: Vec<Open> = heap.drain().collect();
                self.heap = heap;
                if let Some(legal) = self.legal_masks(grid) {
                    self.propagate_masked(grid, &legal, &seeds, width, slots);
                    self.canonical_parents_masked(grid, &legal);
                } else {
                    self.propagate_buckets(grid, &seeds, width, slots);
                    let len = self.distances.len();
                    self.canonical_parents(grid, 0..len);
                }
            }
            None => {
                self.propagate(grid, heap);
            }
        }
    }

    /// Label-correcting search over a circular bucket queue of `slots`
    /// buckets `width` wide (width <= the cheapest move, so a bucket's cells
    /// cannot improve each other). Sets distances only.
    fn propagate_buckets(&mut self, grid: &Grid<N>, seeds: &[Open], width: f32, slots: usize) {
        let moves = moves(grid);
        let sides = self.side_table(grid);
        let max_cost = self.options.max_cost;
        let inverse = 1.0 / width;
        let mut buckets = std::mem::take(&mut self.buckets);
        if buckets.len() < slots {
            buckets.resize_with(slots, Vec::new);
        }
        for bucket in &mut buckets {
            bucket.clear();
        }
        let mut queued = 0usize;
        for seed in seeds {
            buckets[0].push(*seed);
            queued += 1;
        }
        let mut slot = 0usize;
        let mut scratch: Vec<Open> = Vec::new();
        while queued > 0 {
            if buckets[slot].is_empty() {
                slot = (slot + 1) & (slots - 1);
                continue;
            }
            std::mem::swap(&mut scratch, &mut buckets[slot]);
            while let Some(Open { cost, index }) = scratch.pop() {
                queued -= 1;
                let index = index as usize;
                if cost > self.distances[index] {
                    continue;
                }
                let enter = grid.cost_at(index);
                for m in &moves {
                    let Some(from) = step(&sides, index, m) else {
                        continue;
                    };
                    if grid.blocked_at(from) {
                        continue;
                    }
                    let candidate = cost + enter * m.multiplier;
                    if candidate > max_cost || candidate >= self.distances[from] {
                        continue;
                    }
                    if m.corner_count > 0
                        && m.corners[..m.corner_count as usize]
                            .iter()
                            .any(|delta| grid.blocked_at((from as isize - delta) as usize))
                    {
                        continue;
                    }
                    self.distances[from] = candidate;
                    let target = (candidate * inverse) as usize & (slots - 1);
                    let entry = Open {
                        cost: candidate,
                        index: from as u32,
                    };
                    if target == slot {
                        scratch.push(entry);
                    } else {
                        buckets[target].push(entry);
                    }
                    queued += 1;
                }
            }
            std::mem::swap(&mut scratch, &mut buckets[slot]);
        }
        self.buckets = buckets;
    }

    /// Picks each cell's parent as the binary-heap search would: the legal
    /// neighbour minimising `(distance + step, neighbour distance, index)`.
    fn canonical_parents(&mut self, grid: &Grid<N>, cells: impl IntoIterator<Item = usize>) {
        let moves = moves(grid);
        let sides = self.side_table(grid);
        for cell in cells {
            let distance = self.distances[cell];
            if !distance.is_finite() || grid.blocked_at(cell) {
                self.parents[cell] = NO_PARENT;
                continue;
            }
            if distance == 0.0 && self.is_goal(grid, cell) {
                self.parents[cell] = NO_PARENT;
                continue;
            }
            let mut best = (f32::INFINITY, f32::INFINITY, NO_PARENT);
            for m in &moves {
                // Forward move cell -> cell + offset.
                let Some(next) = step(&sides, cell, m) else {
                    continue;
                };
                if grid.blocked_at(next) {
                    continue;
                }
                let base = self.distances[next];
                if !base.is_finite() {
                    continue;
                }
                if m.corner_count > 0
                    && m.corners[..m.corner_count as usize]
                        .iter()
                        .any(|delta| grid.blocked_at((cell as isize + delta) as usize))
                {
                    continue;
                }
                let candidate = base + grid.cost_at(next) * m.multiplier;
                let key = (candidate, base, next as u32);
                if key.0 < best.0
                    || (key.0 == best.0 && (key.1 < best.1 || (key.1 == best.1 && key.2 < best.2)))
                {
                    best = key;
                }
            }
            self.parents[cell] = best.2;
        }
    }

    /// Boundary masks for `grid`, rebuilt only when its size changes.
    fn side_table(&mut self, grid: &Grid<N>) -> std::sync::Arc<[u64]> {
        if self.sides.len() != grid.len() || self.dimensions != grid.dimensions() {
            let mut masks = Vec::new();
            side_masks(grid.dimensions(), &mut masks);
            self.sides = std::sync::Arc::from(masks);
        }
        self.sides.clone()
    }

    /// Legal-move masks for `grid`, rebuilt when the grid changed; `None`
    /// for grids too large to cache or with more than 32 moves.
    fn legal_masks(&mut self, grid: &Grid<N>) -> Option<std::sync::Arc<LegalMasks>> {
        let len = grid.len();
        if len > LEGAL_MASK_CELLS {
            return None;
        }
        if let Some(legal) = &self.legal
            && legal.version == grid.version()
            && legal.len == len
        {
            return Some(legal.clone());
        }
        let moves = moves(grid);
        if moves.len() > 32 {
            return None;
        }
        let sides = self.side_table(grid);
        let mut out = vec![0u32; len];
        for (cell, slot) in out.iter_mut().enumerate() {
            if grid.blocked_at(cell) {
                continue;
            }
            let mut bits = 0u32;
            for (bit, m) in moves.iter().enumerate() {
                let Some(next) = step(&sides, cell, m) else {
                    continue;
                };
                if grid.blocked_at(next) {
                    continue;
                }
                if m.corner_count > 0
                    && m.corners[..m.corner_count as usize]
                        .iter()
                        .any(|delta| grid.blocked_at((cell as isize + delta) as usize))
                {
                    continue;
                }
                bits |= 1 << bit;
            }
            *slot = bits;
        }
        // Moves are symmetric, so move `m` into a cell is the opposite move
        // (`count - 1 - m`) out of its neighbour.
        let count = moves.len();
        let mut into = vec![0u32; len];
        for (cell, &mask) in out.iter().enumerate() {
            let mut bits = mask;
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let next = (cell as isize + moves[bit].linear) as usize;
                into[next] |= 1 << (count - 1 - bit);
            }
        }
        let legal = std::sync::Arc::new(LegalMasks {
            version: grid.version(),
            len,
            into,
            out,
        });
        self.legal = Some(legal.clone());
        Some(legal)
    }

    /// [`Self::propagate_buckets`] driven by cached legal-move masks.
    fn propagate_masked(
        &mut self,
        grid: &Grid<N>,
        legal: &LegalMasks,
        seeds: &[Open],
        width: f32,
        slots: usize,
    ) {
        let moves = moves(grid);
        let max_cost = self.options.max_cost;
        let inverse = 1.0 / width;
        let mut buckets = std::mem::take(&mut self.buckets);
        if buckets.len() < slots {
            buckets.resize_with(slots, Vec::new);
        }
        for bucket in &mut buckets {
            bucket.clear();
        }
        let mut queued = seeds.len();
        buckets[0].extend_from_slice(seeds);
        let mut slot = 0usize;
        let mut scratch: Vec<Open> = Vec::new();
        let distances = &mut self.distances;
        while queued > 0 {
            if buckets[slot].is_empty() {
                slot = (slot + 1) & (slots - 1);
                continue;
            }
            std::mem::swap(&mut scratch, &mut buckets[slot]);
            while let Some(Open { cost, index }) = scratch.pop() {
                queued -= 1;
                let index = index as usize;
                if cost > distances[index] {
                    continue;
                }
                let enter = grid.cost_at(index);
                let mut bits = legal.into[index];
                while bits != 0 {
                    let m = &moves[bits.trailing_zeros() as usize];
                    bits &= bits - 1;
                    let from = (index as isize + m.linear) as usize;
                    let candidate = cost + enter * m.multiplier;
                    if candidate > max_cost || candidate >= distances[from] {
                        continue;
                    }
                    distances[from] = candidate;
                    let target = (candidate * inverse) as usize & (slots - 1);
                    let entry = Open {
                        cost: candidate,
                        index: from as u32,
                    };
                    if target == slot {
                        scratch.push(entry);
                    } else {
                        buckets[target].push(entry);
                    }
                    queued += 1;
                }
            }
            std::mem::swap(&mut scratch, &mut buckets[slot]);
        }
        self.buckets = buckets;
    }

    /// [`Self::canonical_parents`] for every cell, using legal-move masks.
    fn canonical_parents_masked(&mut self, grid: &Grid<N>, legal: &LegalMasks) {
        let moves = moves(grid);
        let goals: Vec<usize> = self
            .goals
            .iter()
            .filter_map(|goal| grid.index(*goal).ok())
            .collect();
        for cell in 0..self.distances.len() {
            let distance = self.distances[cell];
            if !distance.is_finite() || (distance == 0.0 && goals.contains(&cell)) {
                self.parents[cell] = NO_PARENT;
                continue;
            }
            let mut best = (f32::INFINITY, f32::INFINITY, NO_PARENT);
            let mut bits = legal.out[cell];
            while bits != 0 {
                let m = &moves[bits.trailing_zeros() as usize];
                bits &= bits - 1;
                let next = (cell as isize + m.linear) as usize;
                let base = self.distances[next];
                let candidate = base + grid.cost_at(next) * m.multiplier;
                if candidate < best.0
                    || (candidate == best.0
                        && (base < best.1 || (base == best.1 && (next as u32) < best.2)))
                {
                    best = (candidate, base, next as u32);
                }
            }
            self.parents[cell] = best.2;
        }
    }

    fn is_goal(&self, grid: &Grid<N>, cell: usize) -> bool {
        self.goals
            .iter()
            .any(|goal| grid.index(*goal).ok() == Some(cell))
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
                if self.record_touched {
                    self.touched.push(index as u32);
                }
                heap.push(Open {
                    cost: 0.0,
                    index: index as u32,
                });
            }
        }
    }

    /// Reverse Dijkstra: a popped cell `n` offers `dist[n] + cost(c -> n)` to
    /// every cell `c` that may step into it. Moves are precomputed as linear
    /// index deltas; the queue is handed back to `self.heap` for reuse.
    fn propagate(&mut self, grid: &Grid<N>, mut heap: BinaryHeap<Open>) -> usize {
        let moves = moves(grid);
        let sides = self.side_table(grid);
        let mut settled = 0;
        let max_cost = self.options.max_cost;
        while let Some(Open { cost, index }) = heap.pop() {
            let index = index as usize;
            if cost > self.distances[index] {
                continue;
            }
            settled += 1;
            // Entering `index` costs its cell cost (times √2 diagonally),
            // whichever neighbour the move starts from.
            let enter = grid.cost_at(index);
            for m in &moves {
                // `from = index + offset` steps back by `-offset` into
                // `index`; test the move from `from` because corner rules
                // are not symmetric in 3D.
                let Some(from) = step(&sides, index, m) else {
                    continue;
                };
                if grid.blocked_at(from) {
                    continue;
                }
                let candidate = cost + enter * m.multiplier;
                if candidate > max_cost || candidate >= self.distances[from] {
                    continue;
                }
                if m.corner_count > 0
                    && m.corners[..m.corner_count as usize]
                        .iter()
                        .any(|delta| grid.blocked_at((from as isize - delta) as usize))
                {
                    continue;
                }
                self.distances[from] = candidate;
                self.parents[from] = index as u32;
                if self.record_touched {
                    self.touched.push(from as u32);
                }
                heap.push(Open {
                    cost: candidate,
                    index: from as u32,
                });
            }
        }
        self.heap = heap;
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

/// Per cell, bit `2a` is set on the low face of axis `a` and bit `2a + 1` on
/// its high face, so a move stays in bounds when `sides[cell] & move.sides`
/// is zero. Built without divisions by walking the coordinates.
fn side_masks<const N: usize>(dimensions: [u32; N], out: &mut Vec<u64>) {
    let len: usize = dimensions.iter().map(|&d| d as usize).product();
    out.clear();
    out.reserve(len);
    let mut coords = [0u32; N];
    for _ in 0..len {
        let mut mask = 0u64;
        for axis in 0..N {
            if coords[axis] == 0 {
                mask |= 1 << (2 * axis);
            }
            if coords[axis] + 1 >= dimensions[axis] {
                mask |= 1 << (2 * axis + 1);
            }
        }
        out.push(mask);
        for axis in 0..N {
            coords[axis] += 1;
            if coords[axis] < dimensions[axis] {
                break;
            }
            coords[axis] = 0;
        }
    }
}

/// `index + move.offset` when it stays inside the grid.
#[inline(always)]
fn step<const N: usize>(sides: &[u64], index: usize, m: &Move<N>) -> Option<usize> {
    if sides[index] & m.sides != 0 {
        None
    } else {
        Some((index as isize + m.linear) as usize)
    }
}

/// `(width, slots)` for a bucket-queue rebuild, or `None` when a binary heap
/// is needed (zero-cost cells, or costs spread too widely for few buckets).
fn bucket_plan<const N: usize>(grid: &Grid<N>) -> Option<(f32, usize)> {
    let (mut low, mut high) = (f32::INFINITY, 0.0f32);
    for index in 0..grid.len() {
        if !grid.blocked_at(index) {
            let cost = grid.cost_at(index);
            low = low.min(cost);
            high = high.max(cost);
        }
    }
    if !(low > 0.0 && high.is_finite()) {
        return None;
    }
    let widest = high
        * if grid.diagonal_enabled() {
            3f32.sqrt()
        } else {
            1.0
        };
    // Slightly narrower than the cheapest move so rounding never lets a
    // bucket's cells improve each other more than the label-correcting
    // search tolerates.
    let width = low * 0.999;
    let slots = ((widest / width).ceil() as usize + 2).next_power_of_two();
    (slots <= 4096).then_some((width, slots))
}
