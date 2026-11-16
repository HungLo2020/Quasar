//! The documented conventions, checked numerically.
use quasar_math::*;
use std::f32::consts::{FRAC_PI_2, PI};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}
fn close3(a: Vec3, b: Vec3) -> bool {
    (a - b).length() < 1e-4
}

#[test]
fn view_matrices_are_right_handed_and_look_down_negative_z() {
    let view = look_at(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, UP).unwrap();
    // The target sits 5 units in front of the camera, i.e. at view-space -Z.
    assert!(close3(
        view.transform_point3(Vec3::ZERO),
        Vec3::new(0.0, 0.0, -5.0)
    ));
    // +X stays to the right.
    assert!(view.transform_point3(Vec3::X).x > 0.0);
    assert!(look_at(Vec3::ONE, Vec3::ONE, UP).is_none());
    assert!(
        look_at(Vec3::ZERO, Vec3::Y, UP).is_none(),
        "up parallel to view"
    );
    assert!(look_at(Vec3::splat(f32::NAN), Vec3::ZERO, UP).is_none());
    assert!(look_to(Vec3::ZERO, FORWARD, UP).is_some());
}

#[test]
fn projections_use_zero_to_one_depth() {
    let p = perspective(1.0, 1.5, 0.1, 100.0).unwrap();
    for (z, expected) in [(-0.1, 0.0), (-100.0, 1.0)] {
        let clip = p * Vec4::new(0.0, 0.0, z, 1.0);
        assert!(close(clip.z / clip.w, expected), "{z}");
    }
    let o = orthographic(-2.0, 6.0, -3.0, 1.0, 0.1, 20.0).unwrap();
    let near = o * Vec4::new(-2.0, -3.0, -0.1, 1.0);
    let far = o * Vec4::new(6.0, 1.0, -20.0, 1.0);
    assert!(close3(near.truncate(), Vec3::new(-1.0, -1.0, 0.0)));
    assert!(close3(far.truncate(), Vec3::new(1.0, 1.0, 1.0)));
    let reverse = perspective_infinite_reverse(1.0, 1.0, 0.1).unwrap();
    let clip = reverse * Vec4::new(0.0, 0.0, -0.1, 1.0);
    assert!(close(clip.z / clip.w, 1.0));
    for bad in [
        perspective(0.0, 1.0, 0.1, 10.0),
        perspective(PI, 1.0, 0.1, 10.0),
        perspective(1.0, 0.0, 0.1, 10.0),
        perspective(1.0, 1.0, 0.0, 10.0),
        perspective(1.0, 1.0, 1.0, 0.5),
        orthographic(1.0, -1.0, -1.0, 1.0, 0.1, 1.0),
        orthographic(-1.0, 1.0, -1.0, 1.0, -1.0, 1.0),
        perspective_infinite_reverse(1.0, f32::NAN, 0.1),
    ] {
        assert!(bad.is_none());
    }
}

#[test]
fn project_unproject_and_screen_rays_round_trip() {
    let viewport = Viewport::new(800.0, 600.0).with_origin(10.0, 20.0);
    let view = look_at(Vec3::new(3.0, 4.0, 10.0), Vec3::new(0.0, 1.0, 0.0), UP).unwrap();
    for projection in [
        perspective(1.1, viewport.aspect(), 0.1, 200.0).unwrap(),
        orthographic(-8.0, 8.0, -6.0, 6.0, 0.1, 50.0).unwrap(),
    ] {
        let view_projection = projection * view;
        let center = project(view_projection, Vec3::new(0.0, 1.0, 0.0), viewport).unwrap();
        assert!(close(center.x, 410.0) && close(center.y, 320.0), "{center}");
        for world in [Vec3::new(1.0, 2.0, -1.0), Vec3::new(-2.0, 0.5, 3.0)] {
            let screen = project(view_projection, world, viewport).unwrap();
            assert!((0.0..=1.0).contains(&screen.z));
            let back = unproject(view_projection, screen, viewport).unwrap();
            assert!((back - world).length() < 1e-3, "{back} vs {world}");
            let (origin, direction) =
                screen_ray(view_projection, screen.truncate(), viewport).unwrap();
            assert!(close(direction.length(), 1.0));
            // The ray passes through the projected world point.
            let to_point = world - origin;
            let along = to_point.dot(direction);
            assert!((origin + direction * along - world).length() < 1e-2);
        }
        // Screen +Y is down: a point above the target projects higher.
        let above = project(view_projection, Vec3::new(0.0, 3.0, 0.0), viewport).unwrap();
        assert!(above.y < center.y);
    }
    let perspective_vp = perspective(1.0, 1.0, 0.1, 100.0).unwrap();
    assert!(
        project(perspective_vp, Vec3::new(0.0, 0.0, 5.0), viewport).is_none(),
        "behind camera"
    );
    assert!(project(perspective_vp, Vec3::ZERO, Viewport::new(0.0, 10.0)).is_none());
    assert!(viewport.contains(Vec2::new(10.0, 20.0)));
    assert!(!viewport.contains(Vec2::new(9.0, 20.0)));
    assert_eq!(
        viewport.screen_to_ndc(viewport.ndc_to_screen(Vec2::new(0.25, -0.5))),
        Vec2::new(0.25, -0.5)
    );
}

#[test]
fn angles_wrap_turn_and_follow_the_yaw_convention() {
    assert!(close(wrap_angle(3.0 * PI), PI));
    assert!(close(wrap_angle(-PI), PI));
    assert!(close(wrap_angle(0.5), 0.5));
    assert!(close(angle_delta(PI - 0.1, -PI + 0.1), 0.2));
    assert!(close(lerp_angle(PI - 0.1, -PI + 0.1, 0.5), PI));
    assert!(close(turn_towards(0.0, 1.0, 0.25), 0.25));
    assert!(close(turn_towards(0.0, -1.0, 0.25), -0.25));
    assert!(close(turn_towards(0.0, 0.1, 0.25), 0.1));
    assert!(close3(forward_from_yaw(0.0), FORWARD));
    assert!(
        close3(forward_from_yaw(FRAC_PI_2), Vec3::NEG_X),
        "positive yaw turns left"
    );
    assert!(close3(
        yaw_rotation(FRAC_PI_2) * FORWARD,
        forward_from_yaw(FRAC_PI_2)
    ));
    for yaw in [-2.0f32, -0.3, 0.0, 1.0, 3.0] {
        assert!(close(yaw_of(forward_from_yaw(yaw)).unwrap(), yaw));
    }
    assert!(yaw_of(Vec3::Y).is_none());
    assert!(close3(forward_from_yaw_pitch(0.0, FRAC_PI_2), UP));
    assert!(close(forward_from_yaw_pitch(0.7, 0.3).length(), 1.0));
}

#[test]
fn helpers_compose_like_the_docs_say() {
    let model = trs(
        Vec3::new(1.0, 2.0, 3.0),
        yaw_rotation(FRAC_PI_2),
        Vec3::splat(2.0),
    );
    // Scale, then rotate, then translate: -Z scaled to -2Z, rotated to -2X.
    assert!(close3(
        model.transform_point3(FORWARD),
        Vec3::new(-1.0, 2.0, 3.0)
    ));
    // `a * b` applies `b` first.
    let ab = Mat4::from_translation(Vec3::X) * Mat4::from_scale(Vec3::splat(3.0));
    assert!(close3(
        ab.transform_point3(Vec3::X),
        Vec3::new(4.0, 0.0, 0.0)
    ));
    assert!(close(damp_factor(10.0, 0.0), 0.0));
    assert!(damp_factor(10.0, 1.0 / 60.0) > 0.0 && damp_factor(10.0, 1.0 / 60.0) < 1.0);
    let half = damp_factor(1.0, 0.5);
    assert!(close(
        1.0 - (1.0 - half) * (1.0 - half),
        damp_factor(1.0, 1.0)
    ));
    assert_eq!(damp_factor(f32::INFINITY, 0.1), 1.0);
}

#[cfg(feature = "serde")]
#[test]
fn types_serialise_when_enabled() {
    let json = serde_json::to_string(&Vec3::new(1.0, 2.0, 3.0)).unwrap();
    assert_eq!(
        serde_json::from_str::<Vec3>(&json).unwrap(),
        Vec3::new(1.0, 2.0, 3.0)
    );
}
