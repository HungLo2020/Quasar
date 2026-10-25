use crate::grid::SpatialError;

fn all_finite<const N: usize>(values: &[f64; N]) -> bool {
    values.iter().all(|value| value.is_finite())
}

/// Axis-aligned bounding box in `N` dimensions.
///
/// Bounds are inclusive. A degenerate box (`min == max`) is a valid point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb<const N: usize> {
    pub min: [f64; N],
    pub max: [f64; N],
}

impl<const N: usize> Aabb<N> {
    /// Creates a box from inclusive minimum and maximum corners.
    pub fn new(min: [f64; N], max: [f64; N]) -> crate::SpatialResult<Self> {
        if !all_finite(&min) || !all_finite(&max) {
            return Err(SpatialError::NonFinite);
        }
        if (0..N).any(|axis| min[axis] > max[axis]) {
            return Err(SpatialError::InvalidBounds);
        }
        Ok(Self { min, max })
    }

    /// Creates a degenerate box containing exactly one point.
    pub fn point(point: [f64; N]) -> crate::SpatialResult<Self> {
        Self::new(point, point)
    }

    /// Creates a box from a center and non-negative half extents.
    pub fn from_center_half_extents(
        center: [f64; N],
        half_extents: [f64; N],
    ) -> crate::SpatialResult<Self> {
        if !all_finite(&center) || !all_finite(&half_extents) {
            return Err(SpatialError::NonFinite);
        }
        if half_extents.iter().any(|extent| *extent < 0.0) {
            return Err(SpatialError::InvalidBounds);
        }
        let mut min = [0.0; N];
        let mut max = [0.0; N];
        for axis in 0..N {
            min[axis] = center[axis] - half_extents[axis];
            max[axis] = center[axis] + half_extents[axis];
        }
        Self::new(min, max)
    }

    /// Creates a box from a minimum corner and a non-negative size.
    pub fn from_min_size(min: [f64; N], size: [f64; N]) -> crate::SpatialResult<Self> {
        if !all_finite(&min) || !all_finite(&size) {
            return Err(SpatialError::NonFinite);
        }
        if size.iter().any(|extent| *extent < 0.0) {
            return Err(SpatialError::InvalidBounds);
        }
        let mut max = [0.0; N];
        for axis in 0..N {
            max[axis] = min[axis] + size[axis];
        }
        Self::new(min, max)
    }

    pub fn center(&self) -> [f64; N] {
        let mut center = [0.0; N];
        for (axis, value) in center.iter_mut().enumerate() {
            *value = self.min[axis] * 0.5 + self.max[axis] * 0.5;
        }
        center
    }

    pub fn extents(&self) -> [f64; N] {
        let mut extents = [0.0; N];
        for (axis, value) in extents.iter_mut().enumerate() {
            *value = self.max[axis] - self.min[axis];
        }
        extents
    }

    pub fn half_extents(&self) -> [f64; N] {
        let extents = self.extents();
        let mut half = [0.0; N];
        for (axis, value) in half.iter_mut().enumerate() {
            *value = extents[axis] * 0.5;
        }
        half
    }

    pub fn volume(&self) -> f64 {
        self.extents().into_iter().product()
    }

    pub fn is_point(&self) -> bool {
        (0..N).all(|axis| self.min[axis] == self.max[axis])
    }

    /// Inclusive overlap test. Boxes that only touch at a face/edge/corner
    /// intersect.
    pub fn intersects(&self, other: &Self) -> bool {
        (0..N).all(|axis| self.min[axis] <= other.max[axis] && self.max[axis] >= other.min[axis])
    }

    pub fn contains_point(&self, point: &[f64; N]) -> bool {
        (0..N).all(|axis| point[axis] >= self.min[axis] && point[axis] <= self.max[axis])
    }

    pub fn contains_aabb(&self, other: &Self) -> bool {
        (0..N).all(|axis| self.min[axis] <= other.min[axis] && self.max[axis] >= other.max[axis])
    }

    /// Inclusive intersection, or `None` when the boxes are separated.
    pub fn intersection(&self, other: &Self) -> Option<Self> {
        if !self.intersects(other) {
            return None;
        }
        let mut min = [0.0; N];
        let mut max = [0.0; N];
        for axis in 0..N {
            min[axis] = self.min[axis].max(other.min[axis]);
            max[axis] = self.max[axis].min(other.max[axis]);
        }
        Self::new(min, max).ok()
    }

    pub fn union(&self, other: &Self) -> Self {
        let mut min = [0.0; N];
        let mut max = [0.0; N];
        for axis in 0..N {
            min[axis] = self.min[axis].min(other.min[axis]);
            max[axis] = self.max[axis].max(other.max[axis]);
        }
        // Component-wise extrema preserve `min <= max` for valid inputs.
        Self { min, max }
    }

    pub fn expanded(&self, margin: f64) -> crate::SpatialResult<Self> {
        if !margin.is_finite() {
            return Err(SpatialError::NonFinite);
        }
        if margin < 0.0 {
            return Err(SpatialError::InvalidBounds);
        }
        let mut min = self.min;
        let mut max = self.max;
        for axis in 0..N {
            min[axis] -= margin;
            max[axis] += margin;
        }
        Self::new(min, max)
    }

    /// Squared distance from `point` to the nearest point inside the box.
    /// Points inside or on the boundary return zero.
    pub fn distance_squared_to_point(&self, point: &[f64; N]) -> f64 {
        point
            .iter()
            .zip(&self.min)
            .zip(&self.max)
            .map(|((value, min), max)| {
                let gap = if value < min {
                    min - value
                } else if value > max {
                    value - max
                } else {
                    0.0
                };
                gap * gap
            })
            .sum()
    }

    /// Squared distance between the two boxes; zero when they intersect.
    pub fn distance_squared_to_aabb(&self, other: &Self) -> f64 {
        let mut distance = 0.0;
        for axis in 0..N {
            let gap = if self.max[axis] < other.min[axis] {
                other.min[axis] - self.max[axis]
            } else if other.max[axis] < self.min[axis] {
                self.min[axis] - other.max[axis]
            } else {
                0.0
            };
            distance += gap * gap;
        }
        distance
    }
}

/// Ray with a normalized direction. `t` values are world-space distances.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray<const N: usize> {
    pub origin: [f64; N],
    pub direction: [f64; N],
}

impl<const N: usize> Ray<N> {
    /// Creates a ray and normalizes its direction.
    pub fn new(origin: [f64; N], direction: [f64; N]) -> crate::SpatialResult<Self> {
        if !all_finite(&origin) || !all_finite(&direction) {
            return Err(SpatialError::NonFinite);
        }
        let norm_squared: f64 = direction.iter().map(|value| value * value).sum();
        let norm = norm_squared.sqrt();
        if !norm.is_finite() || norm <= 0.0 {
            return Err(SpatialError::InvalidDirection);
        }
        let mut normalized = [0.0; N];
        for axis in 0..N {
            normalized[axis] = direction[axis] / norm;
        }
        Ok(Self {
            origin,
            direction: normalized,
        })
    }

    pub fn at(&self, t: f64) -> [f64; N] {
        let mut point = [0.0; N];
        for (axis, value) in point.iter_mut().enumerate() {
            *value = self.origin[axis] + self.direction[axis] * t;
        }
        point
    }

    /// Slab intersection with an AABB. Returns the entry distance and outward
    /// normal. A ray starting inside returns distance zero and a zero normal.
    pub fn intersect_aabb(&self, aabb: &Aabb<N>, max_distance: f64) -> Option<(f64, [f64; N])> {
        if !max_distance.is_finite() || max_distance < 0.0 {
            return None;
        }
        let mut near = 0.0;
        let mut far = max_distance;
        let mut normal = [0.0; N];
        let mut hit_axis = false;
        for axis in 0..N {
            let direction = self.direction[axis];
            if direction == 0.0 {
                if self.origin[axis] < aabb.min[axis] || self.origin[axis] > aabb.max[axis] {
                    return None;
                }
                continue;
            }
            let mut first = (aabb.min[axis] - self.origin[axis]) / direction;
            let mut second = (aabb.max[axis] - self.origin[axis]) / direction;
            let mut sign = -1.0;
            if first > second {
                std::mem::swap(&mut first, &mut second);
                sign = 1.0;
            }
            if first > near {
                near = first;
                normal = [0.0; N];
                normal[axis] = sign;
                hit_axis = true;
            }
            far = far.min(second);
            if near > far {
                return None;
            }
        }
        if !hit_axis {
            normal = [0.0; N];
        }
        Some((near, normal))
    }
}

/// Half-space with a unit normal. Points with non-negative signed distance are
/// inside.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane<const N: usize> {
    normal: [f64; N],
    offset: f64,
}

impl<const N: usize> Plane<N> {
    /// Creates a plane from a non-zero normal and offset. Both are normalized
    /// so [`Self::signed_distance`] is in world units.
    pub fn new(normal: [f64; N], offset: f64) -> crate::SpatialResult<Self> {
        if !all_finite(&normal) || !offset.is_finite() {
            return Err(SpatialError::NonFinite);
        }
        let norm_squared: f64 = normal.iter().map(|value| value * value).sum();
        let norm = norm_squared.sqrt();
        if !norm.is_finite() || norm <= 0.0 {
            return Err(SpatialError::InvalidPlane);
        }
        let mut unit = [0.0; N];
        for axis in 0..N {
            unit[axis] = normal[axis] / norm;
        }
        Ok(Self {
            normal: unit,
            offset: offset / norm,
        })
    }

    /// Plane through `point` whose positive side faces `normal`.
    pub fn from_point_normal(point: [f64; N], normal: [f64; N]) -> crate::SpatialResult<Self> {
        if !all_finite(&point) {
            return Err(SpatialError::NonFinite);
        }
        let offset: f64 = (0..N).map(|axis| normal[axis] * point[axis]).sum();
        Self::new(normal, -offset)
    }

    pub fn normal(&self) -> [f64; N] {
        self.normal
    }

    pub fn offset(&self) -> f64 {
        self.offset
    }

    pub fn signed_distance(&self, point: &[f64; N]) -> f64 {
        (0..N)
            .map(|axis| self.normal[axis] * point[axis])
            .sum::<f64>()
            + self.offset
    }

    /// Conservative AABB test: false means the box is certainly outside.
    pub fn intersects_aabb(&self, aabb: &Aabb<N>) -> bool {
        let mut support = 0.0;
        for axis in 0..N {
            support += self.normal[axis]
                * if self.normal[axis] >= 0.0 {
                    aabb.max[axis]
                } else {
                    aabb.min[axis]
                };
        }
        support + self.offset >= 0.0
    }
}

/// Convex region defined by the intersection of half-spaces.
///
/// The planes are not required to be normalized or ordered. An empty plane list
/// is rejected because it would silently contain the entire world.
#[derive(Clone, Debug, PartialEq)]
pub struct Frustum<const N: usize> {
    planes: Vec<Plane<N>>,
}

impl<const N: usize> Frustum<N> {
    pub fn new(planes: Vec<Plane<N>>) -> crate::SpatialResult<Self> {
        if planes.is_empty() {
            return Err(SpatialError::EmptyFrustum);
        }
        Ok(Self { planes })
    }

    pub fn planes(&self) -> &[Plane<N>] {
        &self.planes
    }

    pub fn contains_point(&self, point: &[f64; N]) -> bool {
        self.planes
            .iter()
            .all(|plane| plane.signed_distance(point) >= 0.0)
    }

    /// Conservative AABB test. The result may be true for a box outside the
    /// exact frustum, but never false for a box that intersects it.
    pub fn intersects_aabb(&self, aabb: &Aabb<N>) -> bool {
        self.planes.iter().all(|plane| plane.intersects_aabb(aabb))
    }
}
