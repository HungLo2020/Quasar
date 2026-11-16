#![cfg(feature = "model")]
//! glTF clips and skins through the renderer-neutral animation layer.
use quasar_animation::model::*;
use quasar_animation::{LoopMode, Marker, Player, Value};
use quasar_assets::model::Model3D;
use quasar_assets::{AssetServer, GltfLoader, MemorySource};
use std::sync::Arc;

fn bar() -> quasar_assets::Handle<Model3D> {
    let mut source = MemorySource::new("fixture");
    source
        .insert(
            "bar.gltf",
            include_bytes!("../../quasar-assets/tests/fixtures/skinned_bar.gltf").to_vec(),
        )
        .unwrap();
    let mut server = AssetServer::new();
    server.mount("", Arc::new(source)).unwrap();
    server.register(GltfLoader::animated());
    server.load::<GltfLoader>("bar.gltf").unwrap()
}

fn transform(m: [[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r])
}

fn close(a: [f32; 3], b: [f32; 3], tolerance: f32) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < tolerance)
}

#[test]
fn clips_use_skeleton_property_names_and_interpolations() {
    let model = bar();
    let clip = clip_from_model(&model, 0).unwrap();
    assert_eq!(clip.name(), "bend");
    assert_eq!(clip.duration(), 1.0);
    let names: Vec<_> = clip.tracks().iter().map(|t| t.property()).collect();
    assert!(names.contains(&"bone.tip.rotation"));
    assert!(names.contains(&"bone.root.translation"));
    assert!(names.contains(&"bone.root.scale"));
    let half = clip.sample(0.5);
    // Linear quaternion track: halfway between identity and 90 degrees about Z.
    let q = half.quat("bone.tip.rotation").unwrap();
    let angle = 2.0 * q[3].acos();
    assert!((angle - std::f32::consts::FRAC_PI_4).abs() < 1e-3, "{q:?}");
    // Step translation holds the first key until t = 1.
    assert_eq!(half.vec3("bone.root.translation"), Some([0., 0., 0.]));
    assert_eq!(
        clip.sample(1.0).vec3("bone.root.translation"),
        Some([1., 0., 0.])
    );
    // Cubic spline with zero tangents eases: 1.5 at the midpoint, ends exact.
    let scale = half.vec3("bone.root.scale").unwrap();
    assert!(close(scale, [1.5; 3], 1e-3), "{scale:?}");
    let quarter = clip.sample(0.25).vec3("bone.root.scale").unwrap();
    assert!(
        quarter[0] < 1.25,
        "ease-in is slower than linear: {quarter:?}"
    );
    assert!(close(
        clip.sample(1.0).vec3("bone.root.scale").unwrap(),
        [2.; 3],
        1e-5
    ));
    assert!(clip_from_model(&model, 7).is_err());
    assert_eq!(clips_from_model(&model).unwrap().len(), 1);
}

#[test]
fn markers_attach_to_imported_clips_and_fire_during_playback() {
    let model = bar();
    let clip = clip_from_model_with_markers(&model, 0, vec![Marker::new(0.4, "footstep")]).unwrap();
    let mut player = Player::new(Arc::new(clip), LoopMode::Once);
    player.play();
    let events = player.advance(0.5).unwrap();
    assert!(
        events.events.iter().any(|event| event.marker == "footstep"),
        "{events:?}"
    );
}

#[test]
fn skeletons_order_parents_first_and_produce_joint_palettes() {
    let model = bar();
    let (skeleton, binding) = skeleton_from_model(&model, 0).unwrap();
    let names: Vec<_> = skeleton.bones().iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["root", "tip"]);
    assert_eq!(skeleton.bones()[1].parent, Some(0));
    assert_eq!(binding.bone_for_joint, vec![0, 1]);

    let clip = clip_from_model(&model, 0).unwrap();
    // Rest pose: identity palette, so bind-pose vertices are unchanged.
    let rest = binding.joint_matrices(&skeleton.sample(&quasar_animation::Pose::new()));
    assert!(close(
        transform(rest[1], [0.5, 2., 0.]),
        [0.5, 2., 0.],
        1e-5
    ));
    // Before the step at t = 1 the root has not moved; the tip bends 90
    // degrees by the end. At t = 0.999 rotation is ~90 and scale ~2.
    let pose = skeleton.sample(&clip.sample(1.0));
    let palette = binding.joint_matrices(&pose);
    // Root scales by 2 and moves +1 X; the tip (child) bends 90 degrees.
    let tip_vertex = transform(palette[1], [0.5, 2., 0.]);
    // Bent tip vertex in root space is (-1, 1.5, 0); scaled by 2, then +1 X.
    assert!(close(tip_vertex, [-1.0, 3.0, 0.0], 1e-3), "{tip_vertex:?}");
    let base_vertex = transform(palette[0], [0.5, 0., 0.]);
    assert!(close(base_vertex, [2.0, 0.0, 0.0], 1e-4), "{base_vertex:?}");
    assert!(skeleton_from_model(&model, 3).is_err());
}

#[test]
fn rigid_rigs_animate_through_node_local_matrices() {
    let model = bar();
    let clip = clip_from_model(&model, 0).unwrap();
    let locals = node_local_matrices(&model, &clip.sample(1.0));
    let world = model.world_transforms(|node| Some(locals[node]));
    let tip = model.node_index("tip").unwrap();
    // The tip pivot sits 1 unit above the scaled, translated root.
    assert!(close(
        transform(world[tip], [0., 0., 0.]),
        [1., 2., 0.],
        1e-4
    ));
    // Unanimated nodes keep their authored transform.
    let unposed = node_local_matrices(&model, &quasar_animation::Pose::new());
    assert_eq!(unposed[tip], model.nodes[tip].local_matrix());
    assert_eq!(node_bone_name(&model, tip), "tip");
    let _ = Value::Float(0.0);
}
