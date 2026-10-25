//! Tilemap metadata adapter. It consumes `quasar-tilemap` without enabling its
//! renderer feature.
use crate::error::{NavError, NavResult};
use crate::grid::Grid;

/// Builds a 2D navigation grid from one tile layer. `blocked` receives the
/// tile GID (`0` for empty). The grid dimensions match `Runtime::map()`.
pub fn grid_from_tilemap(
    runtime: &quasar_tilemap::Runtime,
    layer: u32,
    mut blocked: impl FnMut(u32) -> bool,
) -> NavResult<Grid<2>> {
    let map = runtime.map();
    let width = map.width;
    let height = map.height;
    let mut flags = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height {
        for x in 0..width {
            let gid = runtime
                .tile(layer, x as i32, y as i32)
                .map_or(0, |cell| cell.gid);
            flags.push(blocked(gid));
        }
    }
    Grid::from_blocked([width, height], flags).map_err(|error| NavError::Tilemap(error.to_string()))
}
