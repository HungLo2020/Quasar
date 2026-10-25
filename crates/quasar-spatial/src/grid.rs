use crate::{Aabb, Frustum, Ray};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt,
};

/// Failure modes for construction and index updates. Queries never panic on
/// ordinary input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpatialError {
    NonFinite,
    InvalidBounds,
    InvalidCellSize,
    InvalidDirection,
    InvalidPlane,
    EmptyFrustum,
    UnknownId,
}

impl SpatialError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::NonFinite => "spatial.non_finite",
            Self::InvalidBounds => "spatial.invalid_bounds",
            Self::InvalidCellSize => "spatial.invalid_cell_size",
            Self::InvalidDirection => "spatial.invalid_direction",
            Self::InvalidPlane => "spatial.invalid_plane",
            Self::EmptyFrustum => "spatial.empty_frustum",
            Self::UnknownId => "spatial.unknown_id",
        }
    }
}

impl fmt::Display for SpatialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::NonFinite => "coordinates, extents and margins must be finite",
            Self::InvalidBounds => "AABB minimum must not exceed its maximum",
            Self::InvalidCellSize => "cell size must be finite and greater than zero",
            Self::InvalidDirection => "ray direction must be finite and non-zero",
            Self::InvalidPlane => "plane normal must be finite and non-zero",
            Self::EmptyFrustum => "a frustum needs at least one plane",
            Self::UnknownId => "the spatial index does not contain this id",
        };
        formatter.write_str(message)
    }
}
impl std::error::Error for SpatialError {}

pub type SpatialResult<T> = Result<T, SpatialError>;

/// Cell size and bounded-work limits for a [`SpatialGrid`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridConfig {
    /// World-space edge length of one cell. Must be finite and greater than
    /// zero.
    pub cell_size: f64,
    /// AABB covering more cells than this is stored in the large-item list and
    /// tested linearly. Zero forces every item into that list.
    pub max_cells_per_item: usize,
    /// A query region covering more cells than this falls back to a linear scan
    /// of all entries instead of walking the cell range. Zero always uses the
    /// linear fallback.
    pub max_query_cells: usize,
}

impl GridConfig {
    pub fn new(
        cell_size: f64,
        max_cells_per_item: usize,
        max_query_cells: usize,
    ) -> SpatialResult<Self> {
        if !cell_size.is_finite() {
            return Err(SpatialError::NonFinite);
        }
        if cell_size <= 0.0 {
            return Err(SpatialError::InvalidCellSize);
        }
        Ok(Self {
            cell_size,
            max_cells_per_item,
            max_query_cells,
        })
    }

    pub fn validate(self) -> SpatialResult<Self> {
        Self::new(
            self.cell_size,
            self.max_cells_per_item,
            self.max_query_cells,
        )
    }
}

impl Default for GridConfig {
    fn default() -> Self {
        Self {
            cell_size: 32.0,
            max_cells_per_item: 4096,
            max_query_cells: 1_000_000,
        }
    }
}

/// Approximate retained-memory accounting. Allocator and map bookkeeping
/// overhead is not included.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GridMemory {
    pub entries: usize,
    pub occupied_cells: usize,
    pub cell_references: usize,
    pub large_items: usize,
    pub approximate_bytes: usize,
}

/// One closest result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Neighbor<T> {
    pub id: T,
    pub distance: f64,
}

/// One ray/AABB intersection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit<const N: usize, T> {
    pub id: T,
    pub distance: f64,
    pub normal: [f64; N],
    pub bounds: Aabb<N>,
}

#[derive(Clone, Copy, Debug)]
struct Entry<const N: usize> {
    bounds: Aabb<N>,
    large: bool,
}

/// Deterministic spatial hash for arbitrary ordered caller keys.
///
/// Items are indexed by every cell their AABB covers. Insert, update and remove
/// are `O(cells covered)`. Point/AABB/radius/frustum queries visit only
/// overlapping cells plus the bounded large-item list. Ray queries walk cells
/// with a generalized DDA.
///
/// Results are sorted by key. A caller that needs stable "first" behavior can
/// sort by its own priority when populating the index, or use
/// [`SpatialGrid::nearest`]/[`SpatialGrid::first_ray`], which break ties by
/// distance and then key.
#[derive(Clone, Debug)]
pub struct SpatialGrid<const N: usize, T: Copy + Ord> {
    config: GridConfig,
    entries: BTreeMap<T, Entry<N>>,
    cells: HashMap<[i64; N], Vec<T>>,
    large: BTreeSet<T>,
}

impl<const N: usize, T: Copy + Ord> SpatialGrid<N, T> {
    pub fn new(config: GridConfig) -> SpatialResult<Self> {
        Ok(Self {
            config: config.validate()?,
            entries: BTreeMap::new(),
            cells: HashMap::new(),
            large: BTreeSet::new(),
        })
    }

    pub fn config(&self) -> GridConfig {
        self.config
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains(&self, id: T) -> bool {
        self.entries.contains_key(&id)
    }

    pub fn get(&self, id: T) -> Option<Aabb<N>> {
        self.entries.get(&id).map(|entry| entry.bounds)
    }

    /// Inserts or replaces an item. Returns the previous bounds if the key
    /// already existed.
    pub fn insert(&mut self, id: T, bounds: Aabb<N>) -> Option<Aabb<N>> {
        let previous = self.remove(id);
        let large = self
            .cover_range(&bounds)
            .filter(|(_, _, count)| *count <= self.config.max_cells_per_item);
        if let Some((min, max, count)) = large {
            for_each_cell(min, max, count, |cell| {
                self.cells.entry(cell).or_default().push(id);
            });
        } else {
            self.large.insert(id);
        }
        self.entries.insert(
            id,
            Entry {
                bounds,
                large: large.is_none(),
            },
        );
        previous
    }

    /// Updates an existing item. Returns [`SpatialError::UnknownId`] when the
    /// key was never inserted, leaving the grid unchanged.
    pub fn update(&mut self, id: T, bounds: Aabb<N>) -> SpatialResult<Aabb<N>> {
        let previous = self.get(id).ok_or(SpatialError::UnknownId)?;
        self.insert(id, bounds);
        Ok(previous)
    }

    pub fn remove(&mut self, id: T) -> Option<Aabb<N>> {
        let entry = self.entries.remove(&id)?;
        if entry.large {
            self.large.remove(&id);
        } else if let Some((min, max, count)) = self.cover_range(&entry.bounds) {
            for_each_cell(min, max, count, |cell| {
                if let Some(bucket) = self.cells.get_mut(&cell) {
                    bucket.retain(|existing| *existing != id);
                    if bucket.is_empty() {
                        self.cells.remove(&cell);
                    }
                }
            });
        }
        Some(entry.bounds)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.cells.clear();
        self.large.clear();
    }

    /// Rebuilds every item with a new configuration. Items that no longer fit
    /// the large-item policy move automatically.
    pub fn rebuild(&mut self, config: GridConfig) -> SpatialResult<()> {
        let config = config.validate()?;
        let entries: Vec<(T, Aabb<N>)> = self.iter().collect();
        self.clear();
        self.config = config;
        for (id, bounds) in entries {
            self.insert(id, bounds);
        }
        Ok(())
    }

    /// Ordered iteration by key.
    pub fn iter(&self) -> impl Iterator<Item = (T, Aabb<N>)> + '_ {
        self.entries.iter().map(|(id, entry)| (*id, entry.bounds))
    }

    /// Union of every item, or `None` for an empty index.
    pub fn bounds(&self) -> Option<Aabb<N>> {
        let mut iter = self.entries.values();
        let first = iter.next()?;
        Some(iter.fold(first.bounds, |bounds, entry| bounds.union(&entry.bounds)))
    }

    pub fn occupied_cells(&self) -> usize {
        self.cells.len()
    }

    pub fn cell_references(&self) -> usize {
        self.cells.values().map(Vec::len).sum()
    }

    pub fn large_items(&self) -> usize {
        self.large.len()
    }

    pub fn memory(&self) -> GridMemory {
        let entries = self.entries.len();
        let occupied_cells = self.cells.len();
        let cell_references = self.cell_references();
        let large_items = self.large.len();
        let approximate_bytes = std::mem::size_of::<Self>()
            + entries * (std::mem::size_of::<T>() + std::mem::size_of::<Entry<N>>())
            + occupied_cells
                * (std::mem::size_of::<[i64; N]>()
                    + std::mem::size_of::<Vec<T>>()
                    + std::mem::size_of::<T>())
            + cell_references * std::mem::size_of::<T>()
            + large_items * (std::mem::size_of::<T>() * 2 + std::mem::size_of::<usize>());
        GridMemory {
            entries,
            occupied_cells,
            cell_references,
            large_items,
            approximate_bytes,
        }
    }

    /// All items whose bounds intersect `region`, sorted by key.
    pub fn query(&self, region: Aabb<N>) -> Vec<T> {
        let mut output = Vec::new();
        self.query_into(region, &mut output);
        output
    }

    /// Replaces `output` with all items intersecting `region`, sorted by key.
    pub fn query_into(&self, region: Aabb<N>, output: &mut Vec<T>) {
        output.clear();
        let mut found = BTreeSet::new();
        let scan_all = self
            .cover_range(&region)
            .is_none_or(|(_, _, count)| count > self.config.max_query_cells);
        if scan_all {
            for (id, entry) in &self.entries {
                if entry.bounds.intersects(&region) {
                    found.insert(*id);
                }
            }
        } else if let Some((min, max, count)) = self.cover_range(&region) {
            for_each_cell(min, max, count, |cell| {
                if let Some(bucket) = self.cells.get(&cell) {
                    for id in bucket {
                        if self
                            .entries
                            .get(id)
                            .is_some_and(|entry| entry.bounds.intersects(&region))
                        {
                            found.insert(*id);
                        }
                    }
                }
            });
            for id in &self.large {
                if self
                    .entries
                    .get(id)
                    .is_some_and(|entry| entry.bounds.intersects(&region))
                {
                    found.insert(*id);
                }
            }
        }
        output.extend(found);
    }

    pub fn query_point(&self, point: [f64; N]) -> Vec<T> {
        match Aabb::point(point) {
            Ok(bounds) => self.query(bounds),
            Err(_) => Vec::new(),
        }
    }

    /// Items whose AABB intersects `frustum` inside `region`, sorted by key.
    ///
    /// `region` bounds the candidate set (for example a camera's world/level
    /// bounds). The frustum test itself is conservative.
    pub fn query_frustum(&self, frustum: &Frustum<N>, region: Aabb<N>) -> Vec<T> {
        let mut output = Vec::new();
        self.query_frustum_into(frustum, region, &mut output);
        output
    }

    pub fn query_frustum_into(&self, frustum: &Frustum<N>, region: Aabb<N>, output: &mut Vec<T>) {
        output.clear();
        output.extend(self.query(region).into_iter().filter(|id| {
            self.entries
                .get(id)
                .is_some_and(|entry| frustum.intersects_aabb(&entry.bounds))
        }));
    }

    /// Sorted-by-key items within `radius` of `center` (AABB distance).
    pub fn query_radius(&self, center: [f64; N], radius: f64) -> Vec<T> {
        let mut output = Vec::new();
        self.query_radius_into(center, radius, &mut output);
        output
    }

    pub fn query_radius_into(&self, center: [f64; N], radius: f64, output: &mut Vec<T>) {
        output.clear();
        if !radius.is_finite() || radius < 0.0 || !center.iter().all(|value| value.is_finite()) {
            return;
        }
        let center_box = match Aabb::point(center) {
            Ok(bounds) => bounds,
            Err(_) => return,
        };
        let region = match center_box.expanded(radius) {
            Ok(bounds) => bounds,
            Err(_) => return,
        };
        let radius_squared = radius * radius;
        let mut found = BTreeMap::new();
        let mut candidates = Vec::new();
        self.query_into(region, &mut candidates);
        for id in candidates {
            if let Some(entry) = self.entries.get(&id) {
                let distance = entry.bounds.distance_squared_to_point(&center);
                if distance <= radius_squared {
                    found.insert(id, distance);
                }
            }
        }
        output.extend(found.into_keys());
    }

    /// Closest item and its distance, sorted by distance then key.
    pub fn nearest(&self, center: [f64; N], max_distance: f64) -> Option<Neighbor<T>> {
        if !max_distance.is_finite()
            || max_distance < 0.0
            || !center.iter().all(|value| value.is_finite())
        {
            return None;
        }
        let center_box = Aabb::point(center).ok()?;
        let region = center_box.expanded(max_distance).ok()?;
        let mut best: Option<(f64, T)> = None;
        let mut candidates = Vec::new();
        self.query_into(region, &mut candidates);
        for id in candidates {
            let Some(entry) = self.entries.get(&id) else {
                continue;
            };
            let distance = entry.bounds.distance_squared_to_point(&center);
            if distance > max_distance * max_distance {
                continue;
            }
            let candidate = (distance, id);
            if best.is_none_or(|current| {
                candidate.0.total_cmp(&current.0).is_lt()
                    || (candidate.0 == current.0 && candidate.1 < current.1)
            }) {
                best = Some(candidate);
            }
        }
        best.map(|(distance, id)| Neighbor {
            id,
            distance: distance.sqrt(),
        })
    }

    /// All hits along a finite ray, sorted by distance then key. The ray's
    /// direction is normalized by [`Ray::new`].
    pub fn cast_ray(&self, ray: &Ray<N>, max_distance: f64) -> Vec<RayHit<N, T>> {
        let mut output = Vec::new();
        self.cast_ray_into(ray, max_distance, &mut output);
        output
    }

    pub fn cast_ray_into(&self, ray: &Ray<N>, max_distance: f64, output: &mut Vec<RayHit<N, T>>) {
        output.clear();
        if !max_distance.is_finite() || max_distance < 0.0 {
            return;
        }
        let mut candidates = BTreeSet::new();
        let cells = self.ray_cells(ray, max_distance);
        if let Some(cells) = cells {
            for cell in cells {
                if let Some(bucket) = self.cells.get(&cell) {
                    candidates.extend(bucket.iter().copied());
                }
            }
            candidates.extend(self.large.iter().copied());
        } else {
            candidates.extend(self.entries.keys().copied());
        }
        for id in candidates {
            let Some(entry) = self.entries.get(&id) else {
                continue;
            };
            if let Some((distance, normal)) = ray.intersect_aabb(&entry.bounds, max_distance) {
                output.push(RayHit {
                    id,
                    distance,
                    normal,
                    bounds: entry.bounds,
                });
            }
        }
        output.sort_by(|left, right| {
            left.distance
                .total_cmp(&right.distance)
                .then_with(|| left.id.cmp(&right.id))
        });
    }

    pub fn first_ray(&self, ray: &Ray<N>, max_distance: f64) -> Option<RayHit<N, T>> {
        self.cast_ray(ray, max_distance).into_iter().next()
    }

    fn cover_range(&self, bounds: &Aabb<N>) -> Option<([i64; N], [i64; N], usize)> {
        let mut min = [0; N];
        let mut max = [0; N];
        let mut count = 1usize;
        for axis in 0..N {
            let low = cell_coordinate(bounds.min[axis], self.config.cell_size)?;
            let high = cell_coordinate(bounds.max[axis], self.config.cell_size)?;
            min[axis] = low;
            max[axis] = high;
            let span = i128::from(high) - i128::from(low) + 1;
            let span = usize::try_from(span).ok()?;
            count = count.checked_mul(span)?;
        }
        Some((min, max, count))
    }

    fn ray_cells(&self, ray: &Ray<N>, max_distance: f64) -> Option<Vec<[i64; N]>> {
        const MAX_RAY_CELLS: usize = 262_144;
        let mut cell = [0i64; N];
        for (axis, value) in cell.iter_mut().enumerate() {
            *value = cell_coordinate(ray.origin[axis], self.config.cell_size)?;
        }
        let mut t_max = [f64::INFINITY; N];
        let mut t_delta = [f64::INFINITY; N];
        let mut step = [0i64; N];
        for axis in 0..N {
            let direction = ray.direction[axis];
            if direction > 0.0 {
                step[axis] = 1;
                let boundary = (cell[axis] as f64 + 1.0) * self.config.cell_size;
                t_max[axis] = (boundary - ray.origin[axis]) / direction;
                t_delta[axis] = self.config.cell_size / direction;
            } else if direction < 0.0 {
                step[axis] = -1;
                let boundary = cell[axis] as f64 * self.config.cell_size;
                t_max[axis] = (boundary - ray.origin[axis]) / direction;
                t_delta[axis] = -self.config.cell_size / direction;
            }
        }
        let mut visited = Vec::new();
        visited.push(cell);
        while visited.len() < MAX_RAY_CELLS {
            let mut axis = 0;
            for candidate in 1..N {
                if t_max[candidate] < t_max[axis] {
                    axis = candidate;
                }
            }
            if !t_max[axis].is_finite() || t_max[axis] > max_distance {
                break;
            }
            cell[axis] = cell[axis].checked_add(step[axis])?;
            t_max[axis] += t_delta[axis];
            if visited.last() != Some(&cell) {
                visited.push(cell);
            }
        }
        Some(visited)
    }
}

fn cell_coordinate(value: f64, cell_size: f64) -> Option<i64> {
    let coordinate = (value / cell_size).floor();
    const LIMIT: f64 = 9_223_372_036_854_775_808.0;
    if !coordinate.is_finite() || !(-LIMIT..LIMIT).contains(&coordinate) {
        return None;
    }
    Some(coordinate as i64)
}

fn for_each_cell<const N: usize>(
    min: [i64; N],
    max: [i64; N],
    count: usize,
    mut visit: impl FnMut([i64; N]),
) {
    let mut cell = [0i64; N];
    for linear in 0..count {
        let mut remainder = linear;
        for axis in 0..N {
            let span = (i128::from(max[axis]) - i128::from(min[axis]) + 1) as usize;
            cell[axis] = min[axis] + (remainder % span) as i64;
            remainder /= span;
        }
        visit(cell);
    }
}
