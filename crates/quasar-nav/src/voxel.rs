//! Navigation grids from voxel occupancy (feature `voxel`).
//!
//! [`walkable_grid`] marks a cell walkable when it is open, the cell below is
//! solid and `clearance` cells above are open (an agent can stand there).
//! [`sync_changes`] updates that grid from edit change events and returns the
//! cells to pass to [`crate::DistanceField::update_region`], so mining or
//! building repairs flow fields incrementally.
use crate::{Grid, GridCell, NavResult};
use quasar_math::IVec3;
use quasar_voxel::{Change, ChannelId, ChunkLayout, Value, Volume};

/// Maps voxel space to a nav grid covering `min..=max` (inclusive voxels).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoxelNavBounds {
    pub min: IVec3,
    pub max: IVec3,
}

impl VoxelNavBounds {
    pub fn dimensions(&self) -> [u32; 3] {
        let size = self.max - self.min + IVec3::ONE;
        [size.x as u32, size.y as u32, size.z as u32]
    }
    pub fn cell(&self, voxel: IVec3) -> Option<GridCell<3>> {
        let local = voxel - self.min;
        let dims = self.dimensions();
        (local.min_element() >= 0
            && (local.x as u32) < dims[0]
            && (local.y as u32) < dims[1]
            && (local.z as u32) < dims[2])
            .then(|| GridCell::new3(local.x, local.y, local.z))
    }
    pub fn voxel(&self, cell: GridCell<3>) -> IVec3 {
        self.min + IVec3::from_array(cell.coords)
    }
}

fn walkable<L: ChunkLayout, T: Value>(
    volume: &Volume<L>,
    channel: ChannelId<T>,
    voxel: IVec3,
    clearance: i32,
    solid: &mut impl FnMut(T) -> bool,
) -> bool {
    solid(volume.get(channel, voxel - IVec3::Y))
        && (0..clearance.max(1)).all(|dy| !solid(volume.get(channel, voxel + IVec3::Y * dy)))
}

/// A nav grid whose open cells are the standable voxels.
pub fn walkable_grid<L: ChunkLayout, T: Value>(
    volume: &Volume<L>,
    channel: ChannelId<T>,
    bounds: VoxelNavBounds,
    clearance: i32,
    mut solid: impl FnMut(T) -> bool,
) -> NavResult<Grid<3>> {
    let dims = bounds.dimensions();
    let mut blocked = Vec::with_capacity((dims[0] * dims[1] * dims[2]) as usize);
    let mut grid = Grid::<3>::new(dims)?;
    for index in 0..grid.len() {
        let cell = grid.cell(index).expect("index in range");
        blocked.push(!walkable(
            volume,
            channel,
            bounds.voxel(cell),
            clearance,
            &mut solid,
        ));
    }
    grid = Grid::from_blocked(dims, blocked)?;
    Ok(grid)
}

/// A nav grid that blocks exactly the solid voxels (flying/swimming agents).
pub fn occupancy_grid<L: ChunkLayout, T: Value>(
    volume: &Volume<L>,
    channel: ChannelId<T>,
    bounds: VoxelNavBounds,
    mut solid: impl FnMut(T) -> bool,
) -> NavResult<Grid<3>> {
    let dims = bounds.dimensions();
    let probe = Grid::<3>::new(dims)?;
    let blocked = (0..probe.len())
        .map(|i| solid(volume.get(channel, bounds.voxel(probe.cell(i).unwrap()))))
        .collect();
    Grid::from_blocked(dims, blocked)
}

/// Applies `changes` of `channel` to a [`walkable_grid`] (re-evaluating each
/// changed voxel and the cells that stand on or under it). Returns the cells
/// whose blocking changed, sorted, for `DistanceField::update_region`.
pub fn sync_changes<L: ChunkLayout, T: Value>(
    grid: &mut Grid<3>,
    volume: &Volume<L>,
    channel: ChannelId<T>,
    bounds: VoxelNavBounds,
    clearance: i32,
    changes: &[Change],
    mut solid: impl FnMut(T) -> bool,
) -> NavResult<Vec<GridCell<3>>> {
    let mut touched: Vec<IVec3> = Vec::new();
    for change in changes
        .iter()
        .filter(|c| usize::from(c.channel) == channel.index())
    {
        for dy in -clearance.max(1)..=1 {
            touched.push(change.voxel + IVec3::Y * dy);
        }
    }
    touched.sort_by_key(|v| (v.x, v.y, v.z));
    touched.dedup();
    let mut dirty = Vec::new();
    for voxel in touched {
        let Some(cell) = bounds.cell(voxel) else {
            continue;
        };
        let blocked = !walkable(volume, channel, voxel, clearance, &mut solid);
        if grid.set_blocked(cell, blocked)? {
            dirty.push(cell);
        }
    }
    Ok(dirty)
}
