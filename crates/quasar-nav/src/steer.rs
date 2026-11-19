//! Line-of-sight steering over a [`DistanceField`].
//!
//! The usual horde rule: an agent that can see its goal walks straight at
//! it; otherwise it follows the field, cutting corners by heading for the
//! furthest cell a few steps ahead that it can see. The game supplies the
//! visibility tests (a ray against its own collision, or
//! [`Grid::line_of_sight`]), so the same helper serves enemies, escorts and
//! turrets alike.
use crate::field::DistanceField;
use crate::grid::{Grid, GridCell};

/// What an agent should head for this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Steer<const N: usize> {
    /// The goal is visible: move straight at it.
    Goal,
    /// Head for this cell (its centre) along the field.
    Toward(GridCell<N>),
    /// The agent stands on a goal cell.
    Arrived,
    /// No route: the cell is unreachable or outside the grid.
    Stuck,
}

/// Steering settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Steering {
    /// How many cells ahead along the field to test for a visible shortcut
    /// (0 or 1 always takes the next cell).
    pub lookahead: usize,
}

impl Default for Steering {
    fn default() -> Self {
        Self { lookahead: 4 }
    }
}

impl Steering {
    pub const fn new(lookahead: usize) -> Self {
        Self { lookahead }
    }

    /// Steering for an agent in `cell`. `sees_goal` is asked first (and
    /// only once); `sees_cell(c)` is asked for cells ahead along the field,
    /// furthest first, until one is visible.
    pub fn steer<const N: usize>(
        &self,
        field: &DistanceField<N>,
        grid: &Grid<N>,
        cell: GridCell<N>,
        sees_goal: impl FnOnce() -> bool,
        mut sees_cell: impl FnMut(GridCell<N>) -> bool,
    ) -> Steer<N> {
        if sees_goal() {
            return Steer::Goal;
        }
        match field.distance(cell) {
            None => return Steer::Stuck,
            Some(distance) if distance == 0.0 && field.next_cell(grid, cell).is_none() => {
                return Steer::Arrived;
            }
            Some(_) => {}
        }
        let Some(first) = field.next_cell(grid, cell) else {
            return Steer::Arrived;
        };
        let mut ahead = [first; 32];
        let mut count = 1;
        let limit = self.lookahead.clamp(1, ahead.len());
        while count < limit {
            match field.next_cell(grid, ahead[count - 1]) {
                Some(next) => {
                    ahead[count] = next;
                    count += 1;
                }
                None => break,
            }
        }
        for &candidate in ahead[1..count].iter().rev() {
            if sees_cell(candidate) {
                return Steer::Toward(candidate);
            }
        }
        Steer::Toward(first)
    }

    /// [`Self::steer`] using the grid's own [`Grid::line_of_sight`] for
    /// both the goal and the cells ahead.
    pub fn steer_on_grid<const N: usize>(
        &self,
        field: &DistanceField<N>,
        grid: &Grid<N>,
        cell: GridCell<N>,
        goal: GridCell<N>,
    ) -> Steer<N> {
        self.steer(
            field,
            grid,
            cell,
            || cell != goal && grid.line_of_sight(cell, goal),
            |ahead| grid.line_of_sight(cell, ahead),
        )
    }
}
