#![cfg(feature = "gltf")]
//! Opt-in skin, node-hierarchy and animation-channel import.
use quasar_assets::{model::*, *};
use std::sync::Arc;

const BAR: &[u8] = include_bytes!("fixtures/skinned_bar.gltf");

fn server(loader: GltfLoader) -> AssetServer {
    let mut source = MemorySource::new("fixture");
    source.insert("bar.gltf", BAR.to_vec()).unwrap();
    let mut server = AssetServer::new();
    server.mount("", Arc::new(source)).unwrap();
    server.register(loader);
    server
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
}

fn transform(m: [[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r])
}

#[test]
fn skins_stay_rejected_unless_requested() {
    let mut static_server = server(GltfLoader::default());
    let error = static_server.load::<GltfLoader>("bar.gltf").unwrap_err();
    assert!(error.message().contains("import_animation"), "{error}");
    let mut pose_only = server(GltfLoader {
        allow_animation_default_pose: true,
        ..GltfLoader::default()
    });
    assert!(
        pose_only.load::<GltfLoader>("bar.gltf").is_err(),
        "skins still need opt-in"
    );
}

#[test]
fn hierarchy_skins_and_weights_import() {
    let mut server = server(GltfLoader::animated());
    let model = server.load::<GltfLoader>("bar.gltf").unwrap();
    assert_eq!(model.nodes.len(), 3);
    let root = model.node_index("root").unwrap();
    let tip = model.node_index("tip").unwrap();
    assert_eq!(model.nodes[tip].parent, Some(root));
    assert_eq!(model.nodes[root].children, vec![tip]);
    assert_eq!(model.nodes[tip].world[3], [0., 1., 0., 1.]);
    assert_eq!(model.nodes[2].primitives, vec![0]);
    assert_eq!(model.nodes[2].skin, Some(0));
    assert_eq!(model.instances[0].skin, Some(0));

    let skin = &model.skins[0];
    assert_eq!(skin.name, "bar_rig");
    assert_eq!(skin.joints, vec![root, tip]);
    assert_eq!(skin.skeleton, Some(root));
    assert_eq!(skin.inverse_bind_matrices[1][3], [0., -1., 0., 1.]);

    // No normals: vertices are expanded per face, and weights with them.
    let primitive = &model.primitives[0];
    let weights = primitive.skin.as_ref().unwrap();
    assert_eq!(primitive.vertices.len(), 12);
    assert_eq!(weights.joints.len(), 12);
    assert_eq!(weights.weights.len(), 12);
    for w in &weights.weights {
        assert!(
            (w.iter().sum::<f32>() - 1.).abs() < 1e-5,
            "normalised: {w:?}"
        );
    }
    // In the bind pose the palette is identity, so skinning is a no-op.
    let palette = skin.joint_matrices(&model.world_transforms(|_| None));
    for matrix in palette {
        for (column, expected) in matrix.iter().zip([
            [1., 0., 0., 0.],
            [0., 1., 0., 0.],
            [0., 0., 1., 0.],
            [0., 0., 0., 1.],
        ]) {
            assert!(
                column
                    .iter()
                    .zip(expected)
                    .all(|(a, b)| (a - b).abs() < 1e-6)
            );
        }
    }
}

#[test]
fn animated_world_transforms_drive_rigid_and_skinned_parts() {
    let mut server = server(GltfLoader::animated());
    let model = server.load::<GltfLoader>("bar.gltf").unwrap();
    let tip = model.node_index("tip").unwrap();
    let (s, c) = std::f32::consts::FRAC_PI_4.sin_cos();
    // Bend the tip 90 degrees about +Z at its pivot.
    let bent = model.world_transforms(|node| {
        (node == tip).then(|| trs_matrix([0., 1., 0.], [0., 0., s, c], [1.; 3]))
    });
    assert!(close(transform(bent[tip], [0., 1., 0.]), [-1., 1., 0.]));
    let palette = model.skins[0].joint_matrices(&bent);
    // A vertex fully weighted to the tip at (0.5, 2, 0) swings to (-1, 1.5, 0).
    assert!(close(transform(palette[1], [0.5, 2., 0.]), [-1., 1.5, 0.]));
    // A root-weighted vertex stays put.
    assert!(close(transform(palette[0], [0.5, 0., 0.]), [0.5, 0., 0.]));
}

#[test]
fn animation_channels_keep_their_interpolation_and_keys() {
    let mut server = server(GltfLoader::animated());
    let model = server.load::<GltfLoader>("bar.gltf").unwrap();
    assert_eq!(model.animations.len(), 1);
    let bend = &model.animations[0];
    assert_eq!(bend.name, "bend");
    assert_eq!(bend.duration, 1.);
    assert_eq!(bend.channels.len(), 3);
    let rotation = &bend.channels[0];
    assert_eq!(rotation.target, ModelChannelTarget::Rotation);
    assert_eq!(rotation.interpolation, ModelInterpolation::Linear);
    assert_eq!(rotation.times, vec![0., 1.]);
    assert!((rotation.values[1][2] - std::f32::consts::FRAC_PI_4.sin()).abs() < 1e-6);
    assert_eq!(bend.channels[1].interpolation, ModelInterpolation::Step);
    assert_eq!(bend.channels[1].target, ModelChannelTarget::Translation);
    let scale = &bend.channels[2];
    assert_eq!(scale.interpolation, ModelInterpolation::CubicSpline);
    assert_eq!(
        scale.values.len(),
        6,
        "in-tangent, value, out-tangent per key"
    );
    assert!(model.memory_size() > 0);
    assert!(
        !model.warnings.iter().any(|w| w.contains("not evaluated")),
        "imported clips are not reported as skipped"
    );
}

#[test]
fn joint_and_keyframe_budgets_apply() {
    for limits in [
        ModelLimits {
            max_joints: 1,
            ..ModelLimits::default()
        },
        ModelLimits {
            max_keyframes: 3,
            ..ModelLimits::default()
        },
    ] {
        let mut server = server(GltfLoader {
            limits,
            ..GltfLoader::animated()
        });
        assert_eq!(
            server.load::<GltfLoader>("bar.gltf").unwrap_err().kind(),
            AssetErrorKind::Budget
        );
    }
}

#[test]
fn static_models_still_get_a_node_table() {
    // Existing static assets gain the hierarchy without opting in.
    let json = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AACAvwAAgL8AAAAAAACAPwAAgL8AAAAAAAAAAAAAgD8AAAAA"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[-1,-1,0],"max":[1,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],"nodes":[{"name":"pivot","children":[1],"translation":[0,2,0]},{"mesh":0}],"scenes":[{"nodes":[0]}]}"#;
    let mut source = MemorySource::new("fixture");
    source.insert("tri.gltf", json.to_vec()).unwrap();
    let mut server = AssetServer::new();
    server.mount("", Arc::new(source)).unwrap();
    server.register(GltfLoader::default());
    let model = server.load::<GltfLoader>("tri.gltf").unwrap();
    assert_eq!(model.nodes[1].name, "node1");
    assert_eq!(model.nodes[1].parent, Some(0));
    assert_eq!(model.nodes[1].world[3], [0., 2., 0., 1.]);
    assert!(model.skins.is_empty() && model.animations.is_empty());
    assert_eq!(model.primitives[0].skin, None);
}

#[test]
fn named_loader_configurations_coexist_and_cache_separately() {
    let mut server = server(GltfLoader::default());
    server.register_named("animated", GltfLoader::animated());
    // The default slot still rejects skins; the named slot imports them.
    assert!(server.load::<GltfLoader>("bar.gltf").is_err());
    let animated = server
        .load_named::<GltfLoader>("animated", "bar.gltf")
        .unwrap();
    assert!(!animated.skins.is_empty());
    let again = server
        .load_named::<GltfLoader>("animated", "bar.gltf")
        .unwrap();
    assert_eq!(animated.id(), again.id(), "named loads hit their own cache");
    let missing = server
        .load_named::<GltfLoader>("static", "bar.gltf")
        .unwrap_err();
    assert_eq!(missing.kind(), AssetErrorKind::Configuration);
    assert!(missing.message().contains("\"static\""), "{missing}");
    // Unloading a path drops every configuration's entry.
    drop((animated, again));
    assert!(server.unload("bar.gltf").unwrap());
    assert!(!server.is_cached("bar.gltf"));
}
