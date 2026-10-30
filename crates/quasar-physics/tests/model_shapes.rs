#![cfg(feature = "model-assets")]
use quasar_assets::model::*;
use quasar_physics::dim3::*;
fn model() -> Model3D {
    let vertices = (0..8)
        .map(|bits| ModelVertex {
            position: [
                if bits & 1 == 0 { -1. } else { 1. },
                if bits & 2 == 0 { 0. } else { 2. },
                if bits & 4 == 0 { -1. } else { 1. },
            ],
            normal: [0., 1., 0.],
            uv: [0., 0.],
        })
        .collect();
    Model3D {
        primitives: vec![ModelPrimitive {
            name: "body".into(),
            vertices,
            indices: vec![],
            material: 0,
        }],
        instances: vec![
            ModelInstance {
                node: 0,
                name: "body".into(),
                primitive: 0,
                transform: RigidPose3::translated([4., 0., 3.]).matrix().unwrap(),
            },
            ModelInstance {
                node: 1,
                name: "decoration".into(),
                primitive: 0,
                transform: RigidPose3::translated([20., 4., 0.]).matrix().unwrap(),
            },
        ],
        ..Default::default()
    }
}
#[test]
fn default_pose_and_named_selection_fit_only_explicit_geometry() {
    let model = model();
    let selected = model_bounds_selected(&model, |i| i.name == "body").unwrap();
    assert_eq!(selected.min, [3., 0., 2.]);
    assert_eq!(selected.max, [5., 2., 4.]);
    assert_eq!(model_bounds(&model).unwrap().max, [21., 6., 4.]);
    assert!(model_bounds_selected(&model, |_| false).is_err());
    let round = model_cylinder_y(&model, |i| i.name == "body").unwrap();
    assert_eq!(round.pose.translation, [4., 1., 3.]);
    let Shape3::CylinderY {
        radius,
        half_height,
    } = round.shape
    else {
        unreachable!()
    };
    assert!((radius - 2.0f32.sqrt()).abs() < 1e-6);
    assert_eq!(half_height, 1.);
    assert!(selected.collider(PrimitiveFit3::Cuboid).is_ok());
}
#[test]
fn invalid_geometry_and_projective_transforms_fail_without_gpu_or_decoder() {
    let mut m = model();
    m.instances[0].primitive = 99;
    assert!(model_bounds(&m).is_err());
    let mut m = model();
    m.instances[0].transform[3][3] = 2.;
    assert!(model_bounds(&m).is_err());
    let mut m = model();
    m.primitives[0].vertices[0].position[0] = f32::NAN;
    assert!(model_bounds(&m).is_err());
    assert!(model_bounds(&Model3D::default()).is_err());
}
