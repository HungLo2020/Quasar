#![cfg(feature = "gltf")]
use quasar_assets::{model::*, *};
use std::sync::Arc;
fn triangle(uri: Option<&str>) -> (serde_json::Value, Vec<u8>) {
    let mut bytes = Vec::new();
    for p in [[-1f32, -1., 0.], [1., -1., 0.], [0., 1., 0.]] {
        for v in p {
            bytes.extend(v.to_le_bytes());
        }
    }
    for i in [0u16, 1, 2] {
        bytes.extend(i.to_le_bytes());
    }
    let mut buffer = serde_json::json!({"byteLength":bytes.len()});
    if let Some(uri) = uri {
        buffer["uri"] = uri.into();
    }
    (
        serde_json::json!({"asset":{"version":"2.0"},"buffers":[buffer],"bufferViews":[{"buffer":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":6}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[-1,-1,0],"max":[1,1,0]},{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],"nodes":[{"mesh":0,"name":"triangle","translation":[2,0,0]}],"scenes":[{"nodes":[0]}],"scene":0}),
        bytes,
    )
}
fn server(document: &serde_json::Value, bytes: Vec<u8>) -> AssetServer {
    let mut source = MemorySource::new("fixture");
    source
        .insert(
            "models/triangle.gltf",
            serde_json::to_vec(document).unwrap(),
        )
        .unwrap();
    source.insert("models/mesh.bin", bytes).unwrap();
    let mut server = AssetServer::new();
    server.mount("", Arc::new(source)).unwrap();
    server.register(GltfLoader::default());
    server
}
#[test]
fn gltf_loads_through_mounts_generates_normals_and_tracks_dependencies() {
    let (document, bytes) = triangle(Some("mesh.bin"));
    let mut server = server(&document, bytes);
    let model = server.load::<GltfLoader>("models/triangle.gltf").unwrap();
    assert_eq!(model.primitives.len(), 1);
    assert_eq!(model.primitives[0].vertices[0].normal, [0., 0., 1.]);
    assert_eq!(model.primitives[0].indices, [0, 1, 2]);
    assert_eq!(model.instances[0].transform[3], [2., 0., 0., 1.]);
    assert_eq!(model.instances[0].name, "triangle");
    assert_eq!(
        server.dependencies("models/triangle.gltf"),
        vec![AssetPath::parse("models/mesh.bin").unwrap()]
    );
    assert_eq!(
        model.dependencies,
        server.dependencies("models/triangle.gltf")
    );
    let report = server.invalidate("models/mesh.bin", true).unwrap();
    assert!(
        report
            .paths
            .contains(&AssetPath::parse("models/triangle.gltf").unwrap())
    );
    let new = server.load::<GltfLoader>("models/triangle.gltf").unwrap();
    assert!(new.revision() > model.revision());
    assert_eq!(model.primitives[0].vertices.len(), 3);
}
#[test]
fn glb_and_base64_sources_share_the_same_cpu_geometry() {
    use base64::Engine;
    let (mut document, bytes) = triangle(None);
    let mut json = serde_json::to_vec(&document).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut bin = bytes.clone();
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let mut glb = Vec::new();
    for n in [
        0x46546c67u32,
        2,
        (12 + 8 + json.len() + 8 + bin.len()) as u32,
        json.len() as u32,
        0x4e4f534a,
    ] {
        glb.extend(n.to_le_bytes());
    }
    glb.extend(json);
    glb.extend((bin.len() as u32).to_le_bytes());
    glb.extend(0x004e4942u32.to_le_bytes());
    glb.extend(bin);
    let mut source = MemorySource::new("embedded");
    source.insert("triangle.glb", glb).unwrap();
    document["buffers"][0]["uri"] = format!(
        "data:application/octet-stream;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
    .into();
    source
        .insert("triangle.gltf", serde_json::to_vec(&document).unwrap())
        .unwrap();
    let mut server = AssetServer::new();
    server.mount("", Arc::new(source)).unwrap();
    server.register(GltfLoader::default());
    let glb = server.load::<GltfLoader>("triangle.glb").unwrap();
    let gltf = server.load::<GltfLoader>("triangle.gltf").unwrap();
    assert_eq!(glb.primitives[0].vertices, gltf.primitives[0].vertices);
    assert_eq!(glb.instances[0].transform, gltf.instances[0].transform);
}
#[test]
fn malformed_ranges_indices_paths_graphs_and_budgets_are_errors() {
    let (original, bytes) = triangle(Some("mesh.bin"));
    let mut cases = Vec::new();
    let mut d = original.clone();
    d["accessors"][1]["componentType"] = 5122.into();
    cases.push(d);
    let mut d = original.clone();
    d["accessors"][0]["componentType"] = 5123.into();
    cases.push(d);
    let mut d = original.clone();
    d["bufferViews"][0]["byteLength"] = 1000.into();
    cases.push(d);
    let mut d = original.clone();
    d["accessors"][0]["count"] = 1000000000u64.into();
    cases.push(d);
    let mut d = original.clone();
    d["buffers"][0]["uri"] = "../../escape.bin".into();
    cases.push(d);
    let mut d = original.clone();
    d["buffers"][0]["uri"] = "%2fetc/passwd".into();
    cases.push(d);
    let mut d = original.clone();
    d["buffers"][0]["uri"] = "https://example.com/mesh.bin".into();
    cases.push(d);
    let mut d = original.clone();
    d["nodes"][0]["children"] = serde_json::json!([0]);
    cases.push(d);
    let mut d = original.clone();
    d["extensionsRequired"] = serde_json::json!(["EXT_unimplemented"]);
    cases.push(d);
    for d in cases {
        assert!(
            server(&d, bytes.clone())
                .load::<GltfLoader>("models/triangle.gltf")
                .is_err()
        );
    }
    let mut bad = bytes.clone();
    bad[36..38].copy_from_slice(&50u16.to_le_bytes());
    assert!(
        server(&original, bad)
            .load::<GltfLoader>("models/triangle.gltf")
            .is_err()
    );
    let mut server = server(&original, bytes);
    server.register(GltfLoader {
        limits: ModelLimits {
            max_vertices: 2,
            ..ModelLimits::default()
        },
        ..GltfLoader::default()
    });
    assert_eq!(
        server
            .load::<GltfLoader>("models/triangle.gltf")
            .unwrap_err()
            .kind(),
        AssetErrorKind::Budget
    );
    server.register(GltfLoader {
        limits: ModelLimits {
            max_source_bytes: 1,
            ..ModelLimits::default()
        },
        ..GltfLoader::default()
    });
    assert_eq!(
        server
            .load::<GltfLoader>("models/triangle.gltf")
            .unwrap_err()
            .kind(),
        AssetErrorKind::Budget
    );
}
#[test]
fn authored_assets_and_explicit_default_pose_policy_load() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/nightwatch/assets");
    let mut server = AssetServer::new();
    server.mount_file("", root).unwrap();
    server.register(GltfLoader::default());
    let pistol = server
        .load::<GltfLoader>("blaster-kit/blaster-d.glb")
        .unwrap();
    assert_eq!(pistol.instances.len(), 2);
    assert_eq!(pistol.images.len(), 1);
    assert!(
        pistol
            .primitives
            .iter()
            .map(|p| p.indices.len())
            .sum::<usize>()
            > 100
    );
    assert!(
        server
            .load::<GltfLoader>("blaster-kit/crate-medium.glb")
            .is_err()
    );
    server.register(GltfLoader {
        allow_animation_default_pose: true,
        ..GltfLoader::default()
    });
    let crate_model = server
        .load::<GltfLoader>("blaster-kit/crate-medium.glb")
        .unwrap();
    assert!(
        crate_model
            .warnings
            .iter()
            .any(|w| w.contains("animation clips"))
    );
    assert_eq!(crate_model.instances.len(), 2);
}
#[test]
fn color_and_normal_mips_have_explicit_semantics_and_budgets() {
    let image = ModelImage {
        width: 2,
        height: 1,
        rgba: vec![0, 0, 0, 255, 255, 255, 255, 255],
    };
    assert_eq!(
        image.mip_chain(MipColor::Linear, true, 12).unwrap()[1].rgba,
        [128, 128, 128, 255]
    );
    assert!((187..=189).contains(&image.mip_chain(MipColor::Srgb, true, 12).unwrap()[1].rgba[0]));
    assert!(image.mip_chain(MipColor::Srgb, true, 11).is_err());
    let image = ModelImage {
        width: 2,
        height: 1,
        rgba: vec![255, 128, 128, 255, 128, 255, 128, 255],
    };
    let levels = image.mip_chain(MipColor::Normal, true, 12).unwrap();
    let v = &levels[1].rgba;
    let length = v[..3]
        .iter()
        .map(|n| (*n as f32 / 255. * 2. - 1.).powi(2))
        .sum::<f32>()
        .sqrt();
    assert!((length - 1.).abs() < 0.02);
    let odd = ModelImage {
        width: 3,
        height: 1,
        rgba: vec![0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 255],
    };
    assert_eq!(
        odd.mip_chain(MipColor::Linear, true, 16).unwrap()[1].rgba,
        [85, 85, 85, 255]
    );
}
#[test]
fn image_dependency_reload_preserves_old_handles() {
    use image::ImageEncoder;
    let dir = tempfile::tempdir().unwrap();
    let (mut document, mut bytes) = triangle(Some("mesh.bin"));
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    let uv_offset = bytes.len();
    for uv in [[0f32, 0.], [1., 0.], [0.5, 1.]] {
        for v in uv {
            bytes.extend(v.to_le_bytes());
        }
    }
    document["buffers"][0]["byteLength"] = bytes.len().into();
    document["bufferViews"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"buffer":0,"byteOffset":uv_offset,"byteLength":24}));
    document["accessors"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"bufferView":2,"componentType":5126,"count":3,"type":"VEC2"}));
    document["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"] = 2.into();
    document["meshes"][0]["primitives"][0]["material"] = 0.into();
    document["materials"] =
        serde_json::json!([{"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}]);
    document["textures"] = serde_json::json!([{"source":0}]);
    document["images"] = serde_json::json!([{"uri":"palette.png"}]);
    std::fs::write(
        dir.path().join("model.gltf"),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.path().join("mesh.bin"), bytes).unwrap();
    let png = |rgba: &[u8]| {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(rgba, 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        bytes
    };
    std::fs::write(dir.path().join("palette.png"), png(&[255, 0, 0, 255])).unwrap();
    let mut server = AssetServer::new();
    server.mount_file("", dir.path()).unwrap();
    server.register(GltfLoader::default());
    let old = server.load::<GltfLoader>("model.gltf").unwrap();
    std::fs::write(dir.path().join("palette.png"), png(&[0, 255, 0, 255])).unwrap();
    server.invalidate("palette.png", true).unwrap();
    let new = server.load::<GltfLoader>("model.gltf").unwrap();
    assert_eq!(old.images[0].rgba, [255, 0, 0, 255]);
    assert_eq!(new.images[0].rgba, [0, 255, 0, 255]);
    assert!(new.revision() > old.revision());
}

#[cfg(feature = "async")]
#[test]
fn model_loading_reuses_the_existing_bounded_async_worker() {
    let (document, bytes) = triangle(Some("mesh.bin"));
    let server = server(&document, bytes);
    let queue = AsyncAssetServer::new(server, AsyncConfig { queue_capacity: 2 }).unwrap();
    let mut request = queue.load::<GltfLoader>("models/triangle.gltf").unwrap();
    let first = request
        .wait(std::time::Duration::from_secs(5))
        .unwrap()
        .unwrap();
    assert_eq!(first.handle.instances.len(), 1);
    let mut request = queue
        .reload::<GltfLoader>("models/triangle.gltf", true)
        .unwrap();
    let second = request
        .wait(std::time::Duration::from_secs(5))
        .unwrap()
        .unwrap();
    assert!(second.revision > first.revision);
    assert_eq!(
        first.handle.primitives[0].vertices,
        second.handle.primitives[0].vertices
    );
}
