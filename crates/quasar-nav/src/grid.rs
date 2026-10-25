use crate::error::{NavError, NavResult};
use crate::graph::{
    Graph, Neighbor, Path, SearchOptions, SearchOutcome, astar, dijkstra, smooth_path,
};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GridCell<const N: usize> {
    pub coords: [i32; N],
}

impl<const N: usize> GridCell<N> {
    pub const fn new(coords: [i32; N]) -> Self {
        Self { coords }
    }
}

impl GridCell<2> {
    pub const fn new2(x: i32, y: i32) -> Self {
        Self { coords: [x, y] }
    }
}

impl GridCell<3> {
    pub const fn new3(x: i32, y: i32, z: i32) -> Self {
        Self { coords: [x, y, z] }
    }
}

impl<const N: usize> std::fmt::Display for GridCell<N> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?}", self.coords)
    }
}

/// Caller-owned N-dimensional grid graph with per-cell costs and invalidation.
/// A version counter increments whenever blocking or costs change.
#[derive(Clone, Debug)]
pub struct Grid<const N: usize> {
    dimensions: [u32; N],
    blocked: Vec<bool>,
    costs: Vec<f32>,
    diagonal: bool,
    corner_cutting: bool,
    version: u64,
}

impl<const N: usize> Grid<N> {
    pub fn new(dimensions: [u32; N]) -> NavResult<Self> {
        Self::from_blocked(dimensions, vec![false; cell_count(dimensions)?])
    }

    pub fn from_blocked(dimensions: [u32; N], blocked: Vec<bool>) -> NavResult<Self> {
        let count = cell_count(dimensions)?;
        if blocked.len() != count {
            return Err(NavError::CellCountMismatch);
        }
        Ok(Self {
            dimensions,
            costs: vec![1.0; count],
            blocked,
            diagonal: true,
            corner_cutting: false,
            version: 0,
        })
    }

    pub fn dimensions(&self) -> [u32; N] {
        self.dimensions
    }

    pub fn len(&self) -> usize {
        self.blocked.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocked.is_empty()
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn set_diagonal(&mut self, diagonal: bool) {
        self.diagonal = diagonal;
        self.version = self.version.wrapping_add(1);
    }

    pub fn set_corner_cutting(&mut self, corner_cutting: bool) {
        self.corner_cutting = corner_cutting;
        self.version = self.version.wrapping_add(1);
    }

    pub fn index(&self, cell: GridCell<N>) -> NavResult<usize> {
        let mut index = 0usize;
        let mut stride = 1usize;
        for axis in 0..N {
            let coordinate = cell.coords[axis];
            if coordinate < 0 || coordinate as u32 >= self.dimensions[axis] {
                return Err(NavError::OutOfBounds);
            }
            index += coordinate as usize * stride;
            stride *= self.dimensions[axis] as usize;
        }
        Ok(index)
    }

    pub fn cell(&self, index: usize) -> Option<GridCell<N>> {
        if index >= self.blocked.len() {
            return None;
        }
        let mut remainder = index;
        let mut coords = [0i32; N];
        for (axis, value) in coords.iter_mut().enumerate() {
            let dimension = self.dimensions[axis] as usize;
            *value = (remainder % dimension) as i32;
            remainder /= dimension;
        }
        Some(GridCell::new(coords))
    }

    pub fn is_blocked(&self, cell: GridCell<N>) -> bool {
        self.index(cell)
            .ok()
            .is_none_or(|index| self.blocked[index])
    }

    pub fn cost(&self, cell: GridCell<N>) -> Option<f32> {
        self.index(cell).ok().map(|index| self.costs[index])
    }

    /// Returns whether the state changed. Changing any cell increments `version`.
    pub fn set_blocked(&mut self, cell: GridCell<N>, blocked: bool) -> NavResult<bool> {
        let index = self.index(cell)?;
        if self.blocked[index] == blocked {
            return Ok(false);
        }
        self.blocked[index] = blocked;
        self.version = self.version.wrapping_add(1);
        Ok(true)
    }

    pub fn set_cost(&mut self, cell: GridCell<N>, cost: f32) -> NavResult<()> {
        if !cost.is_finite() || cost < 0.0 {
            return Err(NavError::InvalidCost);
        }
        let index = self.index(cell)?;
        if self.costs[index] != cost {
            self.costs[index] = cost;
            self.version = self.version.wrapping_add(1);
        }
        Ok(())
    }

    pub fn set_blocked_many(
        &mut self,
        cells: impl IntoIterator<Item = GridCell<N>>,
        blocked: bool,
    ) -> NavResult<usize> {
        let mut changed = 0;
        for cell in cells {
            changed += usize::from(self.set_blocked(cell, blocked)?);
        }
        Ok(changed)
    }

    pub fn neighbors_into(&self, cell: GridCell<N>, output: &mut Vec<Neighbor<GridCell<N>>>) {
        output.clear();
        let offsets = 3usize.pow(N as u32);
        for linear in 1..offsets {
            let mut digits = linear;
            let mut offset = [0i32; N];
            let mut nonzero = 0usize;
            for value in &mut offset {
                let digit = digits % 3;
                digits /= 3;
                *value = digit as i32 - 1;
                nonzero += usize::from(digit != 1);
            }
            if nonzero == 0 {
                continue;
            }
            if nonzero > 1 && !self.diagonal {
                continue;
            }
            let mut target = cell.coords;
            for axis in 0..N {
                target[axis] += offset[axis];
            }
            let target = GridCell::new(target);
            let Ok(index) = self.index(target) else {
                continue;
            };
            if self.blocked[index] {
                continue;
            }
            if nonzero > 1 && !self.corner_cutting {
                let mut blocked_corner = false;
                for axis in 0..N {
                    if offset[axis] == 0 {
                        continue;
                    }
                    let mut intermediate = cell.coords;
                    intermediate[axis] += offset[axis];
                    if self.is_blocked(GridCell::new(intermediate)) {
                        blocked_corner = true;
                        break;
                    }
                }
                if blocked_corner {
                    continue;
                }
            }
            let step = if nonzero > 1 {
                std::f32::consts::SQRT_2
            } else {
                1.0
            };
            output.push(Neighbor::new(target, self.costs[index] * step));
        }
    }

    pub fn find_path(
        &self,
        start: GridCell<N>,
        goal: GridCell<N>,
        options: SearchOptions,
    ) -> SearchOutcome<GridCell<N>> {
        if self.is_blocked(start) || self.is_blocked(goal) {
            return SearchOutcome::NotFound { visited: 0 };
        }
        astar(
            self,
            start,
            goal,
            |node| self.heuristic(node, goal),
            options,
        )
    }

    pub fn find_path_dijkstra(
        &self,
        start: GridCell<N>,
        goal: GridCell<N>,
        options: SearchOptions,
    ) -> SearchOutcome<GridCell<N>> {
        if self.is_blocked(start) || self.is_blocked(goal) {
            return SearchOutcome::NotFound { visited: 0 };
        }
        dijkstra(self, start, goal, options)
    }

    pub fn find_smoothed_path(
        &self,
        start: GridCell<N>,
        goal: GridCell<N>,
        options: SearchOptions,
    ) -> SearchOutcome<GridCell<N>> {
        match self.find_path(start, goal, options) {
            SearchOutcome::Found(path) => {
                let nodes = smooth_path(&path.nodes, |left, right| self.line_of_sight(left, right));
                let cost = path_cost(self, &nodes);
                SearchOutcome::Found(Path {
                    visited: path.visited,
                    nodes,
                    cost,
                })
            }
            other => other,
        }
    }

    pub fn heuristic(&self, start: GridCell<N>, goal: GridCell<N>) -> f32 {
        if N == 2 {
            let dx = (goal.coords[0] - start.coords[0]).abs() as f32;
            let dy = (goal.coords[1] - start.coords[1]).abs() as f32;
            let min = dx.min(dy);
            let max = dx.max(dy);
            return max + (std::f32::consts::SQRT_2 - 1.0) * min;
        }
        let mut maximum = 0i32;
        for axis in 0..N {
            maximum = maximum.max((goal.coords[axis] - start.coords[axis]).abs());
        }
        maximum as f32
    }

    /// Conservative cell-sampled line-of-sight used by path smoothing.
    pub fn line_of_sight(&self, start: GridCell<N>, goal: GridCell<N>) -> bool {
        if self.is_blocked(start) || self.is_blocked(goal) {
            return false;
        }
        let mut steps = 1usize;
        let mut difference = [0i32; N];
        for (axis, value) in difference.iter_mut().enumerate() {
            *value = goal.coords[axis] - start.coords[axis];
            steps = steps.max(value.unsigned_abs() as usize * 4);
        }
        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            let mut coordinate = [0i32; N];
            for axis in 0..N {
                coordinate[axis] =
                    start.coords[axis] + (difference[axis] as f32 * t).round() as i32;
            }
            if self.is_blocked(GridCell::new(coordinate)) {
                return false;
            }
        }
        true
    }
}

impl<const N: usize> Graph for Grid<N> {
    type Node = GridCell<N>;
    fn neighbors(&self, node: Self::Node, output: &mut Vec<Neighbor<Self::Node>>) {
        self.neighbors_into(node, output);
    }
}

fn cell_count<const N: usize>(dimensions: [u32; N]) -> NavResult<usize> {
    let mut count = 1usize;
    for dimension in dimensions {
        if dimension == 0 {
            return Err(NavError::InvalidDimensions);
        }
        count = count
            .checked_mul(dimension as usize)
            .ok_or(NavError::InvalidDimensions)?;
    }
    if count == 0 || count > 16_777_216 {
        return Err(NavError::InvalidDimensions);
    }
    Ok(count)
}

fn path_cost<const N: usize>(grid: &Grid<N>, nodes: &[GridCell<N>]) -> f64 {
    let mut cost = 0.0f64;
    for pair in nodes.windows(2) {
        let mut nonzero = 0usize;
        for axis in 0..N {
            nonzero += usize::from(pair[1].coords[axis] != pair[0].coords[axis]);
        }
        let step = if nonzero > 1 {
            f64::from(std::f32::consts::SQRT_2)
        } else {
            1.0
        };
        cost += f64::from(grid.cost(pair[1]).unwrap_or(1.0)) * step;
    }
    cost
}

#[cfg(feature = "spatial")]
impl Grid<2> {
    /// Marks every cell whose center is covered by an AABB. Returns the number
    /// of cells whose state changed.
    pub fn set_aabb_blocked(
        &mut self,
        aabb: quasar_spatial::Aabb<2>,
        blocked: bool,
    ) -> NavResult<usize> {
        let min_x = aabb.min[0].floor() as i64;
        let min_y = aabb.min[1].floor() as i64;
        let max_x = aabb.max[0].ceil() as i64;
        let max_y = aabb.max[1].ceil() as i64;
        let width = (max_x - min_x).max(0);
        let height = (max_y - min_y).max(0);
        let cells =
            u64::try_from(width.saturating_mul(height)).map_err(|_| NavError::InvalidDimensions)?;
        if cells > 1_000_000 {
            return Err(NavError::InvalidDimensions);
        }
        let mut changed = 0;
        for y in min_y..max_y {
            for x in min_x..max_x {
                if x < 0
                    || y < 0
                    || x >= self.dimensions[0] as i64
                    || y >= self.dimensions[1] as i64
                {
                    continue;
                }
                changed +=
                    usize::from(self.set_blocked(GridCell::new2(x as i32, y as i32), blocked)?);
            }
        }
        Ok(changed)
    }
}
