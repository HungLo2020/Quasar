//! Opt-in, collision-baked XZ navigation. Both nodes and edges use a Rapier capsule.
use crate::{
    Graph, Grid, GridCell, NavError, NavResult, Neighbor, SearchOptions, SearchOutcome, astar,
};
use quasar_physics::{
    dim3::{CapsuleController3, CollisionWorld},
    rapier3d::pipeline::QueryFilter,
};
#[derive(Clone, Debug)]
pub struct CollisionGrid3 {
    grid: Grid<2>,
    origin: [f32; 2],
    spacing: f32,
    feet_y: f32,
    edges: Vec<Vec<Neighbor<GridCell<2>>>>,
}
impl CollisionGrid3 {
    /// Origin is the center of cell (0,0), in world XZ. Re-bake after changes to
    /// static geometry. No physics dependency is enabled on ordinary nav grids.
    pub fn bake(
        world: &CollisionWorld,
        character: &CapsuleController3,
        dimensions: [u32; 2],
        origin: [f32; 2],
        spacing: f32,
        feet_y: f32,
        filter: QueryFilter<'_>,
    ) -> NavResult<Self> {
        if origin.iter().any(|v| !v.is_finite())
            || !spacing.is_finite()
            || spacing <= 0.
            || !feet_y.is_finite()
        {
            return Err(NavError::InvalidCost);
        }
        if u64::from(dimensions[0]) * u64::from(dimensions[1]) > 262144 {
            return Err(NavError::InvalidDimensions);
        }
        if (0..2)
            .any(|i| !(origin[i] + dimensions[i].saturating_sub(1) as f32 * spacing).is_finite())
        {
            return Err(NavError::InvalidDimensions);
        }
        let mut grid = Grid::new(dimensions)?;
        let point = |cell: GridCell<2>| {
            [
                origin[0] + cell.coords[0] as f32 * spacing,
                feet_y,
                origin[1] + cell.coords[1] as f32 * spacing,
            ]
        };
        for index in 0..grid.len() {
            let cell = grid.cell(index).unwrap();
            grid.set_blocked(cell, character.overlaps(world, point(cell), filter))?;
        }
        let mut edges = vec![Vec::new(); grid.len()];
        for (index, output) in edges.iter_mut().enumerate() {
            let cell = grid.cell(index).unwrap();
            if grid.is_blocked(cell) {
                continue;
            }
            grid.neighbors_into(cell, output);
            output.retain(|edge| {
                character.clear_segment(world, point(cell), point(edge.node), filter)
            });
        }
        Ok(Self {
            grid,
            origin,
            spacing,
            feet_y,
            edges,
        })
    }
    pub fn grid(&self) -> &Grid<2> {
        &self.grid
    }
    pub fn point(&self, cell: GridCell<2>) -> [f32; 3] {
        [
            self.origin[0] + cell.coords[0] as f32 * self.spacing,
            self.feet_y,
            self.origin[1] + cell.coords[1] as f32 * self.spacing,
        ]
    }
    pub fn cell(&self, xz: [f32; 2]) -> GridCell<2> {
        GridCell::new2(
            ((xz[0] - self.origin[0]) / self.spacing).round() as i32,
            ((xz[1] - self.origin[1]) / self.spacing).round() as i32,
        )
    }
    pub fn nearest_free(&self, cell: GridCell<2>) -> Option<GridCell<2>> {
        if !self.grid.is_blocked(cell) {
            return Some(cell);
        }
        (0..self.grid.len())
            .filter_map(|i| self.grid.cell(i))
            .filter(|&c| !self.grid.is_blocked(c))
            .min_by_key(|c| {
                let dx = i64::from(c.coords[0]) - i64::from(cell.coords[0]);
                let dy = i64::from(c.coords[1]) - i64::from(cell.coords[1]);
                (
                    dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy)),
                    *c,
                )
            })
    }
    /// Finds an unblocked node reachable from an actor's actual feet position.
    /// Useful when rounding a point near a curved obstacle selects a blocked cell.
    pub fn nearest_reachable(
        &self,
        world: &CollisionWorld,
        character: &CapsuleController3,
        feet: [f32; 3],
        filter: QueryFilter<'_>,
    ) -> Option<GridCell<2>> {
        if feet.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let cell = self.cell([feet[0], feet[2]]);
        if !self.grid.is_blocked(cell)
            && character.clear_segment(world, feet, self.point(cell), filter)
        {
            return Some(cell);
        }
        let mut candidates: Vec<_> = (0..self.grid.len())
            .filter_map(|i| self.grid.cell(i))
            .filter(|&c| !self.grid.is_blocked(c))
            .collect();
        candidates.sort_by(|&a, &b| {
            let distance = |c| {
                let p = self.point(c);
                (p[0] - feet[0]).powi(2) + (p[2] - feet[2]).powi(2)
            };
            distance(a).total_cmp(&distance(b)).then(a.cmp(&b))
        });
        candidates
            .into_iter()
            .find(|&c| character.clear_segment(world, feet, self.point(c), filter))
    }
    pub fn find_path(
        &self,
        start: GridCell<2>,
        goal: GridCell<2>,
        options: SearchOptions,
    ) -> SearchOutcome<GridCell<2>> {
        if self.grid.is_blocked(start) || self.grid.is_blocked(goal) {
            return SearchOutcome::NotFound { visited: 0 };
        }
        astar(
            self,
            start,
            goal,
            |node| self.grid.heuristic(node, goal),
            options,
        )
    }
}
impl Graph for CollisionGrid3 {
    type Node = GridCell<2>;
    fn neighbors(&self, node: Self::Node, output: &mut Vec<Neighbor<Self::Node>>) {
        output.clear();
        if let Ok(index) = self.grid.index(node) {
            output.extend_from_slice(&self.edges[index]);
        }
    }
}
