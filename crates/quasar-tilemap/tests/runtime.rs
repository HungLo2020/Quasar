use quasar_tilemap::*;
use serde_json::{Value, json};
fn fixture() -> Value {
    json!({"type":"map","version":"1.10","orientation":"orthogonal","renderorder":"right-down","tilewidth":2,"tileheight":2,"width":4,"height":3,"class":"test-level","custom":{"seed":7},"properties":[{"name":"story","type":"string","value":"test"}],"tilesets":[{"firstgid":1,"name":"atlas","image":"atlas.png","imagewidth":4,"imageheight":2,"tilewidth":2,"tileheight":2,"columns":2,"tilecount":2,"tiles":[{"id":0,"properties":[{"name":"solid","type":"bool","value":true}],"animation":[{"tileid":0,"duration":100},{"tileid":1,"duration":200}]}]}],"layers":[{"id":1,"name":"ground","type":"tilelayer","width":4,"height":3,"data":[1,2,0,0,2,1,0,0,0,0,0,0],"opacity":1,"visible":true},{"id":2,"name":"markers","type":"objectgroup","objects":[{"id":1,"name":"spawn","class":"start","x":1,"y":1,"point":true,"properties":[{"name":"payload","type":"class","propertytype":"Quest","value":{"count":2}}]},{"id":2,"name":"trigger","x":4,"y":0,"width":2,"height":2}]}]})
}
fn parse_fixture(value: Value) -> Map {
    let dir = tempfile::tempdir().unwrap();
    parse(
        &serde_json::to_vec(&value).unwrap(),
        &AssetRoot::new(dir.path()).unwrap(),
    )
    .unwrap()
}
#[test]
fn map_paths_without_a_parent_are_rejected_without_panicking() {
    assert!(quasar_tilemap::load("/").is_err());
}
#[test]
fn canonical_and_tiled_round_trips_preserve_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let root = AssetRoot::new(dir.path()).unwrap();
    let map = parse_fixture(fixture());
    let canonical = map.canonical_json().unwrap();
    let loaded = parse(&canonical, &root).unwrap();
    assert_eq!(map, loaded);
    assert_eq!(canonical, loaded.canonical_json().unwrap());
    let exported = to_tiled_json(&map).unwrap();
    assert_eq!(map, parse(&exported, &root).unwrap());
    assert_eq!(map.metadata["custom"]["seed"], 7);
    assert_eq!(property(&map.properties, "story"), Some(&json!("test")));
}
#[test]
fn collision_uses_half_open_bounds_and_layer_offsets() {
    let mut map = parse_fixture(fixture());
    map.layers[0].offset = [10., 20.];
    let runtime = Runtime::new(map).unwrap();
    assert!(
        runtime
            .collides(
                1,
                Bounds {
                    x: 10.,
                    y: 20.,
                    width: 1.,
                    height: 1.
                },
                "solid"
            )
            .unwrap()
    );
    assert!(
        !runtime
            .collides(
                1,
                Bounds {
                    x: 12.,
                    y: 20.,
                    width: 2.,
                    height: 2.
                },
                "solid"
            )
            .unwrap()
    );
    assert!(
        !runtime
            .collides(
                1,
                Bounds {
                    x: 10.,
                    y: 20.,
                    width: 0.,
                    height: 1.
                },
                "solid"
            )
            .unwrap()
    );
    assert_eq!(
        runtime
            .objects_in_bounds(
                2,
                Bounds {
                    x: 0.,
                    y: 0.,
                    width: 4.,
                    height: 2.
                }
            )
            .unwrap()
            .len(),
        1
    );
    assert!(
        runtime
            .objects_in_bounds(
                2,
                Bounds {
                    x: 1.,
                    y: 1.,
                    width: 0.,
                    height: 0.
                }
            )
            .unwrap()
            .is_empty()
    );
}
#[test]
fn sparse_negative_chunks_animate_and_edit_atomically() {
    let mut value = fixture();
    value["infinite"] = json!(true);
    value["width"] = json!(0);
    value["height"] = json!(0);
    let layer = &mut value["layers"][0];
    layer.as_object_mut().unwrap().remove("data");
    layer["chunks"] = json!([{"x":-3,"y":-2,"width":2,"height":2,"data":[1,0,2,1]}]);
    let mut map = parse_fixture(value);
    let runtime = Runtime::new(map.clone()).unwrap();
    assert_eq!(runtime.tile(1, -3, -2).unwrap().gid, 1);
    assert!(runtime.tile(1, 0, 0).is_none());
    assert_eq!(runtime.animated_tile(usize::MAX, 0, 0), None);
    assert_eq!(runtime.animated_tile(0, u32::MAX, 0), None);
    assert_eq!(runtime.animated_tile(0, 0, 99), Some(0));
    assert_eq!(runtime.animated_tile(0, 0, 100), Some(1));
    assert_eq!(runtime.animated_tile(0, 0, 299), Some(1));
    assert_eq!(runtime.animated_tile(0, 0, 300), Some(0));
    let previous = map.clone();
    assert!(map.set_tile(1, -3, -2, 999).is_err());
    assert_eq!(map, previous);
    map.set_tile(1, -3, -2, 2 | FLIP_X | TRANSPOSE).unwrap();
    assert_eq!(
        Runtime::new(map.clone())
            .unwrap()
            .tile(1, -3, -2)
            .unwrap()
            .gid,
        2 | FLIP_X | TRANSPOSE
    );
    map.set_tile(1, 20, -10, 2).unwrap();
    assert_eq!(Runtime::new(map).unwrap().tile(1, 20, -10).unwrap().gid, 2);
}
#[test]
fn malformed_versions_regions_and_semantics_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let root = AssetRoot::new(dir.path()).unwrap();
    for mutate in 0..12 {
        let mut v = fixture();
        match mutate {
            0 => v["orientation"] = json!("isometric"),
            1 => v["layers"][0]["data"] = json!([1]),
            2 => v["layers"][0]["data"][0] = json!(99),
            3 => v["layers"][1]["id"] = json!(1),
            4 => v["tilesets"][0]["tiles"][0]["animation"][0]["duration"] = json!(0),
            5 => v["layers"][0]["type"] = json!("group"),
            6 => v["tilesets"][0]["image"] = json!("../outside.png"),
            7 => v["layers"][0]["opacity"] = json!(2),
            8 => v["layers"][0]["mode"] = json!("multiply"),
            9 => v["tilesets"][0]["transparentcolor"] = json!("#ffffff"),
            10 => v["type"] = json!("tileset"),
            _ => {
                v["infinite"] = json!(true);
                v["layers"][0]["x"] = json!(1);
                v["layers"][0].as_object_mut().unwrap().remove("data");
                v["layers"][0]["chunks"] = json!([{ "x":0,"y":0,"width":1,"height":1,"data":[1]}]);
            }
        };
        assert!(
            parse(&serde_json::to_vec(&v).unwrap(), &root).is_err(),
            "case {mutate}"
        );
    }
    let mut canonical = serde_json::to_value(parse_fixture(fixture())).unwrap();
    canonical["quasar_version"] = json!(999);
    assert!(parse(&serde_json::to_vec(&canonical).unwrap(), &root).is_err());
}
#[test]
fn overlapping_chunks_and_nonfinite_programmatic_queries_fail() {
    let mut map = parse_fixture(fixture());
    if let LayerContent::Tiles { chunks } = &mut map.layers[0].content {
        chunks.push(chunks[0].clone());
    }
    assert!(Runtime::new(map).is_err());
    let runtime = Runtime::new(parse_fixture(fixture())).unwrap();
    assert!(
        runtime
            .collides(
                1,
                Bounds {
                    x: f64::NAN,
                    y: 0.,
                    width: 1.,
                    height: 1.
                },
                "solid"
            )
            .is_err()
    );
}
#[test]
fn million_cell_culling_visits_only_the_visible_grid() {
    let mut map = parse_fixture(fixture());
    map.width = 1000;
    map.height = 1000;
    map.layers.truncate(1);
    map.layers[0].content = LayerContent::Tiles {
        chunks: vec![Chunk {
            x: 0,
            y: 0,
            width: 1000,
            height: 1000,
            cells: vec![2; 1_000_000],
        }],
    };
    let runtime = Runtime::new(map).unwrap();
    let mut count = 0;
    let stats = runtime
        .for_each_cell(
            1,
            Bounds {
                x: 600.,
                y: 800.,
                width: 32.,
                height: 32.,
            },
            |_| count += 1,
        )
        .unwrap();
    assert_eq!(count, 256);
    assert_eq!(stats.visited_cells, 256);
    assert_eq!(stats.visited_rows, 16);
    assert_eq!(runtime.stored_cells(), 1_000_000);
}
#[test]
fn external_tilesets_resolve_paths_and_bundle_without_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let root = AssetRoot::new(dir.path()).unwrap();
    std::fs::create_dir(dir.path().join("sets")).unwrap();
    let mut value = fixture();
    let mut set = value["tilesets"][0].clone();
    set.as_object_mut().unwrap().remove("firstgid");
    set["image"] = json!("../atlas.png");
    std::fs::write(
        dir.path().join("sets/atlas.tsj"),
        serde_json::to_vec(&set).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.path().join("atlas.png"), b"opaque test payload").unwrap();
    value["tilesets"] = json!([{"firstgid":1,"source":"sets/atlas.tsj"}]);
    let map = parse(&serde_json::to_vec(&value).unwrap(), &root).unwrap();
    assert_eq!(map.tilesets[0].image, "atlas.png");
    let output = tempfile::tempdir().unwrap();
    write_bundle(&map, &root, output.path().join("level.json"), false).unwrap();
    assert_eq!(load(output.path().join("level.json")).unwrap().0, map);
    assert_eq!(
        std::fs::read(output.path().join("atlas.png")).unwrap(),
        b"opaque test payload"
    );
    write_bundle(&map, &root, dir.path().join("converted.json"), false).unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("atlas.png")).unwrap(),
        b"opaque test payload"
    );
}
#[cfg(unix)]
#[test]
fn symlink_escapes_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("image.png"), b"x").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("image.png"),
        root.path().join("image.png"),
    )
    .unwrap();
    assert!(
        AssetRoot::new(root.path())
            .unwrap()
            .resolve("image.png")
            .is_err()
    );
    let map = parse_fixture(fixture());
    let assets = AssetRoot::new(root.path()).unwrap();
    std::fs::write(root.path().join("atlas.png"), b"atlas").unwrap();
    std::os::unix::fs::symlink(root.path().join("atlas.png"), root.path().join("map.json"))
        .unwrap();
    assert!(write_bundle(&map, &assets, root.path().join("map.json"), false).is_err());
    assert_eq!(
        std::fs::read(root.path().join("atlas.png")).unwrap(),
        b"atlas"
    );
    std::os::unix::fs::symlink(
        outside.path().join("not-yet-created.json"),
        root.path().join("dangling.json"),
    )
    .unwrap();
    assert!(write_bundle(&map, &assets, root.path().join("dangling.json"), false).is_err());
    assert!(!outside.path().join("not-yet-created.json").exists());
}
#[cfg(feature = "render-2d")]
#[test]
fn previews_decode_png_and_check_declared_atlas_dimensions() {
    let root = tempfile::tempdir().unwrap();
    let map = parse_fixture(fixture());
    let mut pixels = vec![0; 4 * 2 * 4];
    for p in pixels.as_chunks_mut::<4>().0.iter_mut() {
        p.copy_from_slice(&[200, 100, 50, 255]);
    }
    quasar_tilemap::render::write_png(root.path().join("atlas.png"), 4, 2, &pixels).unwrap();
    let runtime = Runtime::new(map.clone()).unwrap();
    let (image, stats) = quasar_tilemap::render::preview(
        &runtime,
        &AssetRoot::new(root.path()).unwrap(),
        quasar_tilemap::render::View {
            x: 0.,
            y: 0.,
            width: 8,
            height: 6,
            zoom: 1.,
        },
        0,
    )
    .unwrap();
    assert_eq!(&image[..4], &[200, 100, 50, 255]);
    assert_eq!(stats.emitted_tiles, 4);
    let mut bad = map;
    bad.tilesets[0].image_width = 6;
    bad.tilesets[0].columns = 3;
    let runtime = Runtime::new(bad).unwrap();
    assert!(
        quasar_tilemap::render::preview(
            &runtime,
            &AssetRoot::new(root.path()).unwrap(),
            quasar_tilemap::render::View {
                x: 0.,
                y: 0.,
                width: 8,
                height: 6,
                zoom: 1.
            },
            0
        )
        .is_err()
    );
}
