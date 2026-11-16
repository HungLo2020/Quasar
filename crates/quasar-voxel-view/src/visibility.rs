//! Chunk connectivity and visibility traversal ("cave culling").
//!
//! Each chunk gets a 15-bit mask of which of its six faces connect to each
//! other through non-opaque cells. A breadth-first traversal from the camera
//! chunk only crosses a chunk when the face it entered through connects to
//! the face it leaves through, and never steps back towards the camera, so
//! sealed caves and terrain behind solid ground are skipped. Every choice that
//! decides the visit set or order is a [`TraversalPolicy`].
use quasar_math::{IVec3, Vec3};
use quasar_voxel::{ChannelId, ChunkLayout, Face, FxHashMap, Value, Volume};
use std::collections::VecDeque;

/// Face-to-face connectivity of one chunk (bit per unordered face pair).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Connectivity(pub u16);

impl Connectivity {
    pub const ALL: Self = Self(0x7fff);
    pub const NONE: Self = Self(0);
    fn bit(a: Face, b: Face) -> u16 {
        let (i, j) = if a.index() < b.index() {
            (a.index(), b.index())
        } else {
            (b.index(), a.index())
        };
        // Pairs (i < j) enumerated row by row: 15 bits.
        let index = i * 6 - i * (i + 1) / 2 + (j - i - 1);
        1 << index
    }
    pub fn connects(self, a: Face, b: Face) -> bool {
        a == b || self.0 & Self::bit(a, b) != 0
    }
    pub fn set(&mut self, a: Face, b: Face) {
        if a != b {
            self.0 |= Self::bit(a, b);
        }
    }
}

/// Decides which cells block visibility.
pub trait ConnectivityPolicy<T: Value> {
    fn opaque(&self, value: T) -> bool;
}

impl<T: Value, F: Fn(T) -> bool> ConnectivityPolicy<T> for F {
    fn opaque(&self, value: T) -> bool {
        self(value)
    }
}

/// Computes a chunk's connectivity by flood-filling its non-opaque cells.
/// Absent and uniform chunks take a fast path.
pub fn chunk_connectivity<L: ChunkLayout, T: Value>(
    volume: &Volume<L>,
    channel: ChannelId<T>,
    coord: IVec3,
    policy: &impl ConnectivityPolicy<T>,
    scratch: &mut Vec<u32>,
) -> Connectivity {
    let uniform = match volume.chunk(coord) {
        None => Some(T::from_raw(<T::Raw as quasar_voxel::Raw>::from_u32(
            volume.default_raw(channel.index()),
        ))),
        Some(chunk) => chunk.storage(channel).uniform().map(T::from_raw),
    };
    if let Some(value) = uniform {
        return if policy.opaque(value) {
            Connectivity::NONE
        } else {
            Connectivity::ALL
        };
    }
    let chunk = volume.chunk(coord).expect("non-uniform implies present");
    let n = L::EDGE as i32;
    // 0 = opaque, 1 = open and unvisited, 2 = visited.
    let mut state = vec![0u8; L::VOLUME];
    for (i, s) in state.iter_mut().enumerate() {
        if !policy.opaque(chunk.get_index(channel, i)) {
            *s = 1;
        }
    }
    let mut result = Connectivity::NONE;
    let faces_of = |p: IVec3| -> u8 {
        let mut mask = 0u8;
        for face in Face::ALL {
            let c = p[face.axis()];
            if (face.is_positive() && c == n - 1) || (!face.is_positive() && c == 0) {
                mask |= face.bit();
            }
        }
        mask
    };
    scratch.clear();
    for start in 0..L::VOLUME {
        if state[start] != 1 {
            continue;
        }
        let mut touched = 0u8;
        state[start] = 2;
        scratch.push(start as u32);
        while let Some(index) = scratch.pop() {
            let p = L::local(index as usize).as_ivec3();
            touched |= faces_of(p);
            for face in Face::ALL {
                let q = p + face.normal();
                if q.min_element() < 0 || q.max_element() >= n {
                    continue;
                }
                let qi = L::index(q.x as u32, q.y as u32, q.z as u32);
                if state[qi] == 1 {
                    state[qi] = 2;
                    scratch.push(qi as u32);
                }
            }
        }
        for a in Face::ALL {
            for b in Face::ALL {
                if touched & a.bit() != 0 && touched & b.bit() != 0 {
                    result.set(a, b);
                }
            }
        }
        if result == Connectivity::ALL {
            break;
        }
    }
    result
}

/// Decisions that fix a traversal's visit set and order.
pub trait TraversalPolicy {
    /// Order in which a chunk's neighbours are queued.
    fn face_order(&self) -> [Face; 6] {
        Face::ALL
    }
    /// Whether `chunk` may be visited at all (frustum, distance, loaded).
    fn admit(&self, chunk: IVec3) -> bool;
    /// Use connectivity masks (false = plain frustum BFS).
    fn use_connectivity(&self) -> bool {
        true
    }
}

/// Connectivity masks for loaded chunks plus a reusable traversal.
#[derive(Debug, Default)]
pub struct VisibilityGraph {
    masks: FxHashMap<IVec3, Connectivity>,
    queue: VecDeque<(IVec3, u8, Option<Face>)>,
    visited: FxHashMap<IVec3, ()>,
}

impl VisibilityGraph {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&mut self, chunk: IVec3, connectivity: Connectivity) {
        self.masks.insert(chunk, connectivity);
    }
    pub fn remove(&mut self, chunk: IVec3) {
        self.masks.remove(&chunk);
    }
    pub fn get(&self, chunk: IVec3) -> Option<Connectivity> {
        self.masks.get(&chunk).copied()
    }
    pub fn len(&self) -> usize {
        self.masks.len()
    }
    pub fn is_empty(&self) -> bool {
        self.masks.is_empty()
    }
    /// Visible chunks in visit order. Chunks without a mask (not loaded) are
    /// not entered. `start` is the camera's chunk.
    pub fn traverse(&mut self, start: IVec3, policy: &impl TraversalPolicy) -> Vec<IVec3> {
        let mut out = Vec::new();
        self.queue.clear();
        self.visited.clear();
        if !self.masks.contains_key(&start) || !policy.admit(start) {
            return out;
        }
        // Direction set (bit per face travelled) forbids stepping back.
        self.queue.push_back((start, 0, None));
        self.visited.insert(start, ());
        let order = policy.face_order();
        while let Some((chunk, travelled, entered)) = self.queue.pop_front() {
            out.push(chunk);
            let mask = self.masks[&chunk];
            for face in order {
                if travelled & face.opposite().bit() != 0 {
                    continue;
                }
                if policy.use_connectivity()
                    && let Some(entry) = entered
                    && !mask.connects(entry, face)
                {
                    continue;
                }
                let next = chunk + face.normal();
                if self.visited.contains_key(&next)
                    || !self.masks.contains_key(&next)
                    || !policy.admit(next)
                {
                    continue;
                }
                self.visited.insert(next, ());
                self.queue
                    .push_back((next, travelled | face.bit(), Some(face.opposite())));
            }
        }
        out
    }
}

/// View frustum and range test for chunks of a layout, in grid voxel space
/// relative to `camera_origin` (camera-relative matrices keep precision).
#[derive(Clone, Debug)]
pub struct ChunkFrustum {
    frustum: Option<quasar_spatial::Frustum<3>>,
    pub edge: f64,
    pub camera: Vec3,
    pub max_distance: f32,
}

impl ChunkFrustum {
    /// `view_projection` maps grid voxel space to clip space (`[0, 1]` depth).
    pub fn new(
        view_projection: quasar_math::Mat4,
        edge: usize,
        camera: Vec3,
        max_distance: f32,
    ) -> Self {
        let m = view_projection.as_dmat4().to_cols_array_2d();
        Self {
            frustum: quasar_spatial::Frustum::from_view_projection(
                m,
                quasar_spatial::ClipDepth::ZeroToOne,
            )
            .ok(),
            edge: edge as f64,
            camera,
            max_distance,
        }
    }
    pub fn contains(&self, chunk: IVec3) -> bool {
        let min = chunk.as_dvec3() * self.edge;
        let max = min + quasar_math::DVec3::splat(self.edge);
        let centre = ((min + max) * 0.5).as_vec3();
        let radius = (self.edge * 0.87) as f32;
        if (centre - self.camera).length() - radius > self.max_distance {
            return false;
        }
        match (
            &self.frustum,
            quasar_spatial::Aabb3::new(min.to_array(), max.to_array()),
        ) {
            (Some(f), Ok(aabb)) => f.intersects_aabb(&aabb),
            _ => true,
        }
    }
}

/// Default traversal policy: frustum + range, connectivity on.
pub struct FrustumPolicy<'a> {
    pub frustum: &'a ChunkFrustum,
    pub connectivity: bool,
}

impl TraversalPolicy for FrustumPolicy<'_> {
    fn admit(&self, chunk: IVec3) -> bool {
        self.frustum.contains(chunk)
    }
    fn use_connectivity(&self) -> bool {
        self.connectivity
    }
}
