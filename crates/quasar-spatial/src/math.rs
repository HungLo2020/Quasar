//! Optional conversions to and from `quasar-math` (glam) types.
//!
//! `quasar-spatial` stays dependency-free by default and stores `f64`
//! arrays; enable the `math` feature to move between those and the shared
//! `f32`/`f64` glam vectors games and renderers use.
use crate::{Aabb2, Aabb3, Ray2, Ray3, SpatialResult};
use quasar_math::{DVec2, DVec3, Vec2, Vec3};

impl Aabb3 {
    /// Box from `f32` glam corners.
    pub fn from_vec3(min: Vec3, max: Vec3) -> SpatialResult<Self> {
        Self::new(min.as_dvec3().to_array(), max.as_dvec3().to_array())
    }
    pub fn min_vec3(&self) -> Vec3 {
        DVec3::from(self.min).as_vec3()
    }
    pub fn max_vec3(&self) -> Vec3 {
        DVec3::from(self.max).as_vec3()
    }
}

impl Aabb2 {
    pub fn from_vec2(min: Vec2, max: Vec2) -> SpatialResult<Self> {
        Self::new(min.as_dvec2().to_array(), max.as_dvec2().to_array())
    }
    pub fn min_vec2(&self) -> Vec2 {
        DVec2::from(self.min).as_vec2()
    }
    pub fn max_vec2(&self) -> Vec2 {
        DVec2::from(self.max).as_vec2()
    }
}

impl Ray3 {
    /// Ray from an `f32` origin and (not necessarily normalised) direction,
    /// e.g. the result of `quasar_math::screen_ray`.
    pub fn from_vec3(origin: Vec3, direction: Vec3) -> SpatialResult<Self> {
        Self::new(
            origin.as_dvec3().to_array(),
            direction.as_dvec3().to_array(),
        )
    }
    pub fn origin_vec3(&self) -> Vec3 {
        DVec3::from(self.origin).as_vec3()
    }
    pub fn direction_vec3(&self) -> Vec3 {
        DVec3::from(self.direction).as_vec3()
    }
}

impl Ray2 {
    pub fn from_vec2(origin: Vec2, direction: Vec2) -> SpatialResult<Self> {
        Self::new(
            origin.as_dvec2().to_array(),
            direction.as_dvec2().to_array(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_round_trip_and_validate() {
        let aabb = Aabb3::from_vec3(Vec3::new(-1.0, 0.0, 2.0), Vec3::new(1.0, 3.0, 4.0)).unwrap();
        assert_eq!(aabb.min_vec3(), Vec3::new(-1.0, 0.0, 2.0));
        assert_eq!(aabb.max_vec3(), Vec3::new(1.0, 3.0, 4.0));
        assert!(Aabb3::from_vec3(Vec3::ONE, Vec3::ZERO).is_err());
        let ray = Ray3::from_vec3(Vec3::ZERO, Vec3::new(0.0, 0.0, -4.0)).unwrap();
        assert_eq!(ray.direction_vec3(), Vec3::NEG_Z);
        assert!(Ray3::from_vec3(Vec3::ZERO, Vec3::ZERO).is_err());
        let hit = ray.intersect_aabb(
            &Aabb3::from_vec3(Vec3::new(-1.0, -1.0, -6.0), Vec3::new(1.0, 1.0, -5.0)).unwrap(),
            100.0,
        );
        assert!(hit.is_some());
        let flat = Aabb2::from_vec2(Vec2::ZERO, Vec2::ONE).unwrap();
        assert_eq!(flat.max_vec2(), Vec2::ONE);
        assert!(Ray2::from_vec2(Vec2::ZERO, Vec2::X).is_ok());
    }
}
