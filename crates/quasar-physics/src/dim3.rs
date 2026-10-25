//! Three-dimensional collision and rigid-body integration backed by `rapier3d`.
//!
//! Advanced Rapier APIs remain available through [`Deref`] on the wrappers,
//! through `PhysicsWorld::raw`/`PhysicsWorld::raw_mut` and through the
//! re-exported [`crate::rapier3d`] crate.
//!
//! ```
//! use quasar_physics::dim3::CollisionWorld;
//! use quasar_physics::rapier3d::prelude::*;
//!
//! let mut world = CollisionWorld::new();
//! world.add_collider(ColliderBuilder::cuboid(10.0, 0.5, 10.0));
//! world.update();
//! assert!(world.raycast([0.0, 5.0, 0.0], [0.0, -1.0, 0.0], 20.0, true).is_some());
//! ```

#[cfg(feature = "dynamics")]
pub use crate::StepReport;
use crate::common::normalize_direction;
#[cfg(feature = "dynamics")]
use crate::common::validate_delta;
pub use crate::{PhysicsError, PhysicsEvents, StepConfig, WorldConfig};
#[cfg(feature = "spatial")]
use rapier3d::geometry::ColliderSet;
#[cfg(feature = "events")]
use rapier3d::pipeline::ChannelEventCollector;
use rapier3d::{
    geometry::{Collider, ColliderHandle},
    math::Vector,
    pipeline::{PhysicsWorld as RapierWorld, QueryFilter},
};
use std::ops::{Deref, DerefMut};

/// Configuration alias for two dimensions.
pub type WorldConfig3 = WorldConfig<3>;
/// Three-dimensional step configuration.
pub type StepConfig3 = StepConfig;
pub use rapier3d::geometry::CollisionEvent;
pub use rapier3d::geometry::ContactForceEvent;
pub use rapier3d::geometry::Group;
pub use rapier3d::geometry::RayIntersection;
pub use rapier3d::geometry::{InteractionGroups, InteractionTestMode};
pub use rapier3d::math::Pose;
/// Rapier's event flags, re-exported for callers that inspect collision events.
pub use rapier3d::pipeline::ActiveEvents;
pub use rapier3d::pipeline::QueryFilterFlags;

/// A ray hit with array coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    pub collider: ColliderHandle,
    pub distance: f32,
    pub normal: [f32; 3],
    pub point: [f32; 3],
}

fn make_ray(origin: [f32; 3], direction: [f32; 3]) -> Option<rapier3d::geometry::Ray> {
    if !origin.iter().all(|value| value.is_finite()) {
        return None;
    }
    let direction = normalize_direction(direction)?;
    Some(rapier3d::geometry::Ray::new(
        Vector::new(origin[0], origin[1], origin[2]),
        Vector::new(direction[0], direction[1], direction[2]),
    ))
}

fn ray_hit(
    ray: &rapier3d::geometry::Ray,
    collider: ColliderHandle,
    intersection: RayIntersection,
) -> RayHit {
    let point = ray.point_at(intersection.time_of_impact);
    RayHit {
        collider,
        distance: intersection.time_of_impact,
        normal: [
            intersection.normal.x,
            intersection.normal.y,
            intersection.normal.z,
        ],
        point: [point.x, point.y, point.z],
    }
}

#[cfg(feature = "events")]
struct EventQueue {
    collector: ChannelEventCollector,
    collisions: std::sync::mpsc::Receiver<CollisionEvent>,
    contact_forces: std::sync::mpsc::Receiver<ContactForceEvent>,
}

#[cfg(feature = "events")]
impl EventQueue {
    fn new() -> Self {
        let (collision_sender, collisions) = std::sync::mpsc::channel();
        let (force_sender, contact_forces) = std::sync::mpsc::channel();
        let (soft_sender, _soft_receiver) = std::sync::mpsc::channel();
        Self {
            collector: ChannelEventCollector::new(collision_sender, force_sender, soft_sender),
            collisions,
            contact_forces,
        }
    }
    fn drain(&self) -> PhysicsEvents<CollisionEvent, ContactForceEvent> {
        let mut events = PhysicsEvents::default();
        events.collisions.extend(self.collisions.try_iter());
        events.contact_forces.extend(self.contact_forces.try_iter());
        events
    }
}

#[cfg(feature = "dynamics")]
/// Caller-owned 3D rigid-body world with explicit fixed stepping.
pub struct PhysicsWorld {
    inner: RapierWorld,
    config: WorldConfig<3>,
    accumulator: f64,
    fixed_steps: u64,
    #[cfg(feature = "events")]
    events: EventQueue,
}

#[cfg(feature = "dynamics")]
impl PhysicsWorld {
    pub fn new(config: WorldConfig<3>) -> Result<Self, PhysicsError> {
        let config = config.validate()?;
        let mut inner = RapierWorld {
            gravity: Vector::new(config.gravity[0], config.gravity[1], config.gravity[2]),
            ..RapierWorld::default()
        };
        inner.integration_parameters.dt = config.step.fixed_dt;
        Ok(Self {
            inner,
            config,
            accumulator: 0.0,
            fixed_steps: 0,
            #[cfg(feature = "events")]
            events: EventQueue::new(),
        })
    }

    pub fn config(&self) -> WorldConfig<3> {
        self.config
    }

    pub fn raw(&self) -> &RapierWorld {
        &self.inner
    }

    pub fn raw_mut(&mut self) -> &mut RapierWorld {
        &mut self.inner
    }

    pub fn set_gravity(&mut self, gravity: [f32; 3]) -> Result<(), PhysicsError> {
        WorldConfig::new(gravity, self.config.step).map(|config| {
            self.config = config;
            self.inner.gravity = Vector::new(gravity[0], gravity[1], gravity[2]);
        })
    }

    pub fn set_step_config(&mut self, step: StepConfig) -> Result<(), PhysicsError> {
        let config = WorldConfig::new(self.config.gravity, step)?;
        self.config = config;
        self.inner.integration_parameters.dt = step.fixed_dt;
        self.accumulator = 0.0;
        Ok(())
    }

    pub fn reset_accumulator(&mut self) {
        self.accumulator = 0.0;
    }

    /// Advances exactly one fixed timestep.
    pub fn step_fixed(&mut self) -> StepReport {
        self.inner.integration_parameters.dt = self.config.step.fixed_dt;
        #[cfg(feature = "events")]
        self.inner.step_with_events(&(), &self.events.collector);
        #[cfg(not(feature = "events"))]
        self.inner.step();
        self.fixed_steps += 1;
        StepReport {
            steps: 1,
            simulated_seconds: f64::from(self.config.step.fixed_dt),
            saturated: false,
        }
    }

    /// Accumulates a frame delta and executes bounded fixed steps.
    ///
    /// For deterministic replay, drive simulation ticks and call
    /// [`Self::step_fixed`] once per tick instead of passing wall-clock deltas.
    pub fn advance(&mut self, frame_delta: f32) -> Result<StepReport, PhysicsError> {
        let frame_delta =
            f64::from(validate_delta(frame_delta)?).min(f64::from(self.config.step.max_frame_dt));
        self.accumulator += frame_delta;
        let fixed_dt = f64::from(self.config.step.fixed_dt);
        let mut steps = 0u32;
        while self.accumulator + 1e-9 >= fixed_dt && steps < self.config.step.max_substeps {
            self.step_fixed();
            self.accumulator -= fixed_dt;
            steps += 1;
        }
        let saturated = self.accumulator + 1e-9 >= fixed_dt;
        if saturated {
            self.accumulator = 0.0;
        }
        Ok(StepReport {
            steps,
            simulated_seconds: f64::from(steps) * fixed_dt,
            saturated,
        })
    }

    pub fn fixed_steps(&self) -> u64 {
        self.fixed_steps
    }

    pub fn simulated_seconds(&self) -> f64 {
        f64::from(self.config.step.fixed_dt) * self.fixed_steps as f64
    }

    /// Drains collision and contact-force events since the previous drain.
    pub fn drain_events(&self) -> PhysicsEvents<CollisionEvent, ContactForceEvent> {
        #[cfg(feature = "events")]
        {
            self.events.drain()
        }
        #[cfg(not(feature = "events"))]
        {
            PhysicsEvents::default()
        }
    }

    /// Convenience raycast with array inputs. The direction is normalized.
    pub fn raycast(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        max_distance: f32,
        solid: bool,
    ) -> Option<RayHit> {
        self.raycast_filtered(
            origin,
            direction,
            max_distance,
            solid,
            QueryFilter::default(),
        )
    }

    pub fn raycast_filtered(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        max_distance: f32,
        solid: bool,
        filter: QueryFilter<'_>,
    ) -> Option<RayHit> {
        if !max_distance.is_finite() || max_distance < 0.0 {
            return None;
        }
        let ray = make_ray(origin, direction)?;
        let (collider, intersection) =
            self.inner
                .cast_ray_and_get_normal(&ray, max_distance, solid, filter)?;
        Some(ray_hit(&ray, collider, intersection))
    }

    #[cfg(feature = "spatial")]
    pub fn spatial_index(
        &self,
    ) -> quasar_spatial::SpatialResult<quasar_spatial::SpatialGrid<3, u64>> {
        spatial_index(&self.inner.colliders)
    }
}

#[cfg(feature = "dynamics")]
impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new(WorldConfig::default()).expect("default 3D physics configuration is valid")
    }
}

#[cfg(feature = "dynamics")]
impl Deref for PhysicsWorld {
    type Target = RapierWorld;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
#[cfg(feature = "dynamics")]
impl DerefMut for PhysicsWorld {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
#[cfg(feature = "dynamics")]
impl AsRef<RapierWorld> for PhysicsWorld {
    fn as_ref(&self) -> &RapierWorld {
        &self.inner
    }
}
#[cfg(feature = "dynamics")]
impl AsMut<RapierWorld> for PhysicsWorld {
    fn as_mut(&mut self) -> &mut RapierWorld {
        &mut self.inner
    }
}

/// Collider-only 3D world for overlap, ray and shape queries.
///
/// It never integrates rigid bodies unless the caller explicitly reaches the
/// raw Rapier world. Call [`CollisionWorld::update`] after adding, removing or
/// moving colliders so the broad phase is current.
pub struct CollisionWorld {
    inner: RapierWorld,
    updates: u64,
    #[cfg(feature = "events")]
    events: EventQueue,
}

impl CollisionWorld {
    pub fn new() -> Self {
        let inner = RapierWorld {
            gravity: Vector::new(0.0, 0.0, 0.0),
            ..RapierWorld::default()
        };
        Self {
            inner,
            updates: 0,
            #[cfg(feature = "events")]
            events: EventQueue::new(),
        }
    }

    /// Refreshes broad/narrow-phase collision state without integrating bodies.
    pub fn update(&mut self) {
        #[cfg(feature = "events")]
        self.inner.detect_collisions(&(), &self.events.collector);
        #[cfg(not(feature = "events"))]
        self.inner.detect_collisions(&(), &());
        self.updates += 1;
    }

    pub fn updates(&self) -> u64 {
        self.updates
    }

    pub fn raw(&self) -> &RapierWorld {
        &self.inner
    }

    pub fn raw_mut(&mut self) -> &mut RapierWorld {
        &mut self.inner
    }

    /// Inserts a standalone collider and returns its handle.
    pub fn add_collider(&mut self, collider: impl Into<Collider>) -> ColliderHandle {
        self.inner.insert_collider(collider, None)
    }

    /// Convenience raycast with array inputs. Call [`Self::update`] first after
    /// mutating colliders.
    pub fn raycast(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        max_distance: f32,
        solid: bool,
    ) -> Option<RayHit> {
        self.raycast_filtered(
            origin,
            direction,
            max_distance,
            solid,
            QueryFilter::default(),
        )
    }

    pub fn raycast_filtered(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        max_distance: f32,
        solid: bool,
        filter: QueryFilter<'_>,
    ) -> Option<RayHit> {
        if !max_distance.is_finite() || max_distance < 0.0 {
            return None;
        }
        let ray = make_ray(origin, direction)?;
        let (collider, intersection) =
            self.inner
                .cast_ray_and_get_normal(&ray, max_distance, solid, filter)?;
        Some(ray_hit(&ray, collider, intersection))
    }

    pub fn drain_events(&self) -> PhysicsEvents<CollisionEvent, ContactForceEvent> {
        #[cfg(feature = "events")]
        {
            self.events.drain()
        }
        #[cfg(not(feature = "events"))]
        {
            PhysicsEvents::default()
        }
    }

    #[cfg(feature = "spatial")]
    pub fn spatial_index(
        &self,
    ) -> quasar_spatial::SpatialResult<quasar_spatial::SpatialGrid<3, u64>> {
        spatial_index(&self.inner.colliders)
    }
}

impl Default for CollisionWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for CollisionWorld {
    type Target = RapierWorld;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl DerefMut for CollisionWorld {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

/// Stable compact key for a Rapier collider handle.
#[cfg(feature = "spatial")]
pub fn collider_key(handle: ColliderHandle) -> u64 {
    let (index, generation) = handle.into_raw_parts();
    (generation as u64) << 32 | index as u64
}

#[cfg(feature = "spatial")]
pub fn collider_from_key(key: u64) -> ColliderHandle {
    ColliderHandle::from_raw_parts(key as u32, (key >> 32) as u32)
}

/// Builds a `quasar-spatial` index of collider AABBs keyed by
/// [`collider_key`]. Rapier's broad phase remains authoritative for physics;
/// this index is for caller-defined filtering, interest management or culling.
#[cfg(feature = "spatial")]
pub fn spatial_index(
    colliders: &ColliderSet,
) -> quasar_spatial::SpatialResult<quasar_spatial::SpatialGrid<3, u64>> {
    spatial_index_with_config(colliders, quasar_spatial::GridConfig::default())
}

/// Builds a collider spatial index with an explicit cell configuration.
#[cfg(feature = "spatial")]
pub fn spatial_index_with_config(
    colliders: &ColliderSet,
    config: quasar_spatial::GridConfig,
) -> quasar_spatial::SpatialResult<quasar_spatial::SpatialGrid<3, u64>> {
    let mut grid = quasar_spatial::SpatialGrid::new(config)?;
    for (handle, collider) in colliders.iter() {
        grid.insert(collider_key(handle), collider_bounds(collider)?);
    }
    Ok(grid)
}

/// Converts a collider's current AABB to `quasar-spatial` coordinates.
#[cfg(feature = "spatial")]
pub fn collider_bounds(
    collider: &Collider,
) -> quasar_spatial::SpatialResult<quasar_spatial::Aabb<3>> {
    let aabb = collider.compute_aabb();
    quasar_spatial::Aabb::new(
        [
            f64::from(aabb.mins.x),
            f64::from(aabb.mins.y),
            f64::from(aabb.mins.z),
        ],
        [
            f64::from(aabb.maxs.x),
            f64::from(aabb.maxs.y),
            f64::from(aabb.maxs.z),
        ],
    )
}

#[cfg(feature = "spatial")]
pub fn aabb_from_rapier(
    aabb: rapier3d::geometry::Aabb,
) -> quasar_spatial::SpatialResult<quasar_spatial::Aabb<3>> {
    quasar_spatial::Aabb::new(
        [
            f64::from(aabb.mins.x),
            f64::from(aabb.mins.y),
            f64::from(aabb.mins.z),
        ],
        [
            f64::from(aabb.maxs.x),
            f64::from(aabb.maxs.y),
            f64::from(aabb.maxs.z),
        ],
    )
}

#[cfg(feature = "spatial")]
pub fn aabb_to_rapier(aabb: quasar_spatial::Aabb<3>) -> rapier3d::geometry::Aabb {
    rapier3d::geometry::Aabb {
        mins: Vector::new(aabb.min[0] as f32, aabb.min[1] as f32, aabb.min[2] as f32),
        maxs: Vector::new(aabb.max[0] as f32, aabb.max[1] as f32, aabb.max[2] as f32),
    }
}
