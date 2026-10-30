//! A prepared, immutable prop set and its collision/navigation caches.
use crate::props::*;
use quasar_nav::physics3d::CollisionGrid3;
use quasar_physics::{
    dim3::{CapsuleController3, CollisionWorld, ShapeError3},
    rapier3d::pipeline::QueryFilter,
};
pub(crate) struct Scene {
    pub props: Vec<Prop>,
    pub world: CollisionWorld,
    pub nav: CollisionGrid3,
    pub player: CapsuleController3,
    pub zombie: CapsuleController3,
    pub fingerprint: u64,
}
impl Scene {
    pub fn new(props: Vec<Prop>) -> Result<Self, ShapeError3> {
        if props.len() > 256 {
            return Err(ShapeError3("Nightwatch prop budget exceeded"));
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut world = CollisionWorld::new();
        for prop in &props {
            if !ids.insert(prop.id) || prop.id == 0 {
                return Err(ShapeError3("duplicate/zero prop identity"));
            }
            prop.pose.rapier()?;
            let half = match prop.visual {
                PropVisual::Model { half, .. } | PropVisual::Cuboid { half, .. } => half,
            };
            if half.iter().any(|v| !v.is_finite() || *v <= 0.) {
                return Err(ShapeError3("invalid visual bounds"));
            }
            if let Some(shape) = prop.placed_collider() {
                let mut collider = shape.collider()?;
                collider.user_data = prop.id as u128;
                world.add_collider(collider);
            }
        }
        world.update();
        let player = CapsuleController3::new(PLAYER_HEIGHT, PLAYER_RADIUS, SKIN)?;
        let zombie = CapsuleController3::new(ZOMBIE_HEIGHT, ZOMBIE_RADIUS, SKIN)?;
        // Slight extra clearance avoids baking paths at the movement skin's exact boundary.
        let navigator = CapsuleController3::new(ZOMBIE_HEIGHT, ZOMBIE_RADIUS + 0.015, SKIN)?;
        let nav = CollisionGrid3::bake(
            &world,
            &navigator,
            [60, 60],
            [-14.75, -14.75],
            0.5,
            SKIN * 2.,
            QueryFilter::default(),
        )
        .map_err(|_| ShapeError3("invalid navigation bake"))?;
        let fingerprint = format!("{props:?}")
            .bytes()
            .fold(0xcbf29ce484222325u64, |hash, b| {
                (hash ^ b as u64).wrapping_mul(0x100000001b3)
            });
        Ok(Self {
            props,
            world,
            nav,
            player,
            zombie,
            fingerprint,
        })
    }
}
