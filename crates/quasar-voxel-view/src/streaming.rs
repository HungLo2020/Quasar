//! What to load, keep and unload around viewers.
use quasar_math::{IVec3, Vec3};
use std::collections::BTreeSet;

/// One viewer of a grid: position in grid voxel space plus ranges in chunks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewer {
    pub position: Vec3,
    /// Optional view direction; chunks ahead load first.
    pub forward: Option<Vec3>,
    /// Horizontal load radius in chunks.
    pub radius: i32,
    /// Inclusive vertical chunk range (absolute), e.g. a world's height.
    pub vertical: Option<(i32, i32)>,
    /// Extra chunks kept before unloading (hysteresis).
    pub keep_margin: i32,
}

impl Viewer {
    pub fn new(position: Vec3, radius: i32) -> Self {
        Self {
            position,
            forward: None,
            radius,
            vertical: None,
            keep_margin: 1,
        }
    }
}

/// Per-update work limits.
#[derive(Clone, Copy, Debug)]
pub struct StreamBudget {
    pub max_loads: usize,
    pub max_unloads: usize,
}

impl Default for StreamBudget {
    fn default() -> Self {
        Self {
            max_loads: 64,
            max_unloads: 256,
        }
    }
}

/// Ordered streaming work.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamPlan {
    /// Chunks to load/generate, highest priority first.
    pub load: Vec<IVec3>,
    /// Chunks no viewer needs (beyond the keep margin), sorted.
    pub unload: Vec<IVec3>,
    /// Desired chunks still missing after this plan's loads.
    pub remaining: usize,
}

/// Plans chunk residency for a set of viewers. Deterministic: priorities
/// are distance (scaled down for chunks ahead of a viewer), then coordinate.
#[derive(Clone, Debug, Default)]
pub struct StreamingPlanner {
    pub budget: StreamBudget,
}

impl StreamingPlanner {
    pub fn new(budget: StreamBudget) -> Self {
        Self { budget }
    }
    /// `edge` is the chunk edge in voxels; `loaded` reports residency.
    pub fn plan(&self, edge: usize, viewers: &[Viewer], loaded: &[IVec3]) -> StreamPlan {
        let edge = edge as f32;
        let loaded_set: BTreeSet<(i32, i32, i32)> =
            loaded.iter().map(|c| (c.x, c.y, c.z)).collect();
        let mut wanted: Vec<(i64, (i32, i32, i32))> = Vec::new();
        let mut desired = BTreeSet::new();
        for viewer in viewers {
            let centre = (viewer.position / edge).floor().as_ivec3();
            let (ylo, yhi) = viewer
                .vertical
                .unwrap_or((centre.y - viewer.radius, centre.y + viewer.radius));
            for y in ylo..=yhi {
                for z in centre.z - viewer.radius..=centre.z + viewer.radius {
                    for x in centre.x - viewer.radius..=centre.x + viewer.radius {
                        let chunk = IVec3::new(x, y, z);
                        let mid = (chunk.as_vec3() + 0.5) * edge;
                        let offset = mid - viewer.position;
                        let horizontal = Vec3::new(offset.x, 0.0, offset.z).length() / edge;
                        if horizontal > viewer.radius as f32 + 0.5 {
                            continue;
                        }
                        let key = (x, y, z);
                        if !desired.insert(key) {
                            continue;
                        }
                        if loaded_set.contains(&key) {
                            continue;
                        }
                        let mut distance = offset.length();
                        if let Some(forward) = viewer.forward.and_then(|f| f.try_normalize())
                            && offset.length() > 1e-3
                        {
                            let ahead = forward.dot(offset.normalize());
                            distance *= 1.25 - 0.5 * ahead.max(0.0);
                        }
                        wanted.push(((distance * 16.0) as i64, key));
                    }
                }
            }
        }
        wanted.sort();
        let remaining = wanted.len().saturating_sub(self.budget.max_loads);
        let load = wanted
            .into_iter()
            .take(self.budget.max_loads)
            .map(|(_, (x, y, z))| IVec3::new(x, y, z))
            .collect();
        let keep = |c: &IVec3| {
            viewers.iter().any(|viewer| {
                let centre = (viewer.position / edge).floor().as_ivec3();
                let r = viewer.radius + viewer.keep_margin;
                let (ylo, yhi) = viewer
                    .vertical
                    .map_or((centre.y - r, centre.y + r), |(a, b)| {
                        (a - viewer.keep_margin, b + viewer.keep_margin)
                    });
                (c.x - centre.x).abs() <= r
                    && (c.z - centre.z).abs() <= r
                    && c.y >= ylo
                    && c.y <= yhi
            })
        };
        let mut unload: Vec<IVec3> = loaded.iter().copied().filter(|c| !keep(c)).collect();
        unload.sort_by_key(|c| (c.x, c.y, c.z));
        unload.truncate(self.budget.max_unloads);
        StreamPlan {
            load,
            unload,
            remaining,
        }
    }
}
