//! Collision-only capsule sweeps and Rapier's kinematic sliding controller.
use crate::{
    dim3::{CollisionWorld, Shape3, ShapeError3},
    rapier3d::{
        control::{CharacterLength, KinematicCharacterController},
        geometry::SharedShape,
        math::{Pose, Vector},
        parry::query::ShapeCastOptions,
        pipeline::QueryFilter,
    },
};
#[derive(Debug, Clone, Copy)]
pub struct CharacterMove3 {
    pub feet: [f32; 3],
    pub translation: [f32; 3],
    pub grounded: bool,
    pub collisions: usize,
}
pub struct CapsuleController3 {
    shape: SharedShape,
    height: f32,
    radius: f32,
    skin: f32,
    controller: KinematicCharacterController,
}
impl CapsuleController3 {
    pub fn new(height: f32, radius: f32, skin: f32) -> Result<Self, ShapeError3> {
        if !height.is_finite()
            || !radius.is_finite()
            || !skin.is_finite()
            || radius <= 0.
            || height < radius * 2.
            || skin <= 0.
            || skin > radius * 0.25
        {
            return Err(ShapeError3("invalid capsule height/radius/skin"));
        }
        let shape = Shape3::CapsuleY {
            half_segment: height * 0.5 - radius,
            radius,
        }
        .shared()?;
        let controller = KinematicCharacterController {
            offset: CharacterLength::Absolute(skin),
            autostep: None,
            snap_to_ground: None,
            ..Default::default()
        };
        Ok(Self {
            shape,
            height,
            radius,
            skin,
            controller,
        })
    }
    pub fn height(&self) -> f32 {
        self.height
    }
    pub fn radius(&self) -> f32 {
        self.radius
    }
    pub fn skin(&self) -> f32 {
        self.skin
    }
    pub fn shape(&self) -> &SharedShape {
        &self.shape
    }
    pub fn pose(&self, feet: [f32; 3]) -> Pose {
        Pose::from_translation(Vector::new(feet[0], feet[1] + self.height * 0.5, feet[2]))
    }
    /// Refresh the collision world's broad phase before querying after changes.
    pub fn overlaps(
        &self,
        world: &CollisionWorld,
        feet: [f32; 3],
        filter: QueryFilter<'_>,
    ) -> bool {
        if feet.iter().any(|v| !v.is_finite()) {
            return true;
        }
        let pose = self.pose(feet);
        world
            .intersect_shape(pose, self.shape.as_ref(), filter)
            .any(|(_, collider)| {
                // GJK's conservative boolean test can report near-separated shapes
                // as intersecting. Ask Parry for signed contact distance before blocking.
                match crate::rapier3d::parry::query::contact(
                    &pose,
                    self.shape.as_ref(),
                    collider.position(),
                    collider.shape(),
                    0.,
                ) {
                    Ok(Some(contact)) => contact.dist <= 0.,
                    Ok(None) => false,
                    Err(_) => true,
                }
            })
    }
    /// Exact swept-capsule clearance for navigation edges/LOS; no box inflation.
    pub fn clear_segment(
        &self,
        world: &CollisionWorld,
        from: [f32; 3],
        to: [f32; 3],
        filter: QueryFilter<'_>,
    ) -> bool {
        if from.iter().chain(&to).any(|v| !v.is_finite()) {
            return false;
        }
        let delta = Vector::from_array(to) - Vector::from_array(from);
        world
            .cast_shape(
                &self.pose(from),
                delta,
                self.shape.as_ref(),
                ShapeCastOptions {
                    max_time_of_impact: 1.,
                    target_distance: self.skin,
                    stop_at_penetration: true,
                    ..Default::default()
                },
                filter,
            )
            .is_none()
    }
    pub fn move_feet(
        &self,
        world: &CollisionWorld,
        feet: [f32; 3],
        delta: [f32; 3],
        dt: f32,
        filter: QueryFilter<'_>,
    ) -> Result<CharacterMove3, ShapeError3> {
        if feet.iter().chain(&delta).any(|v| !v.is_finite())
            || !dt.is_finite()
            || dt <= 0.
            || dt > crate::MAX_FIXED_DT
        {
            return Err(ShapeError3("invalid character movement"));
        }
        let queries = world.query_pipeline_with_filter(filter);
        let pose = self.pose(feet);
        let mut collisions = 0;
        // Rapier's zero-motion path resolves initial penetration (e.g. teleports).
        let correction = self.controller.move_shape(
            dt,
            &queries,
            self.shape.as_ref(),
            &pose,
            Vector::ZERO,
            |_| collisions += 1,
        );
        let corrected = Pose::from_translation(correction.translation) * pose;
        let movement = self.controller.move_shape(
            dt,
            &queries,
            self.shape.as_ref(),
            &corrected,
            Vector::from_array(delta),
            |_| collisions += 1,
        );
        let translation = correction.translation + movement.translation;
        Ok(CharacterMove3 {
            feet: (Vector::from_array(feet) + translation).to_array(),
            translation: translation.to_array(),
            grounded: movement.grounded,
            collisions,
        })
    }
    pub fn ray_distance(
        &self,
        feet: [f32; 3],
        origin: [f32; 3],
        direction: [f32; 3],
        limit: f32,
    ) -> Option<f32> {
        let direction = crate::common::normalize_direction(direction)?;
        if feet.iter().chain(&origin).any(|v| !v.is_finite()) || !limit.is_finite() || limit < 0. {
            return None;
        }
        self.shape.cast_ray(
            &self.pose(feet),
            &crate::rapier3d::geometry::Ray::new(
                Vector::from_array(origin),
                Vector::from_array(direction),
            ),
            limit,
            true,
        )
    }
}
