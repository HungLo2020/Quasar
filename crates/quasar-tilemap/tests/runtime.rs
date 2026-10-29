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
#[cfg(feature = "png")]
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

#[test]
fn live_batches_are_transactional_and_preserve_existing_storage() {
    let mut runtime = Runtime::new(parse_fixture(fixture())).unwrap();
    let buffer = match &runtime.map().layers[0].content {
        LayerContent::Tiles { chunks } => chunks[0].cells.as_ptr(),
        _ => unreachable!(),
    };
    let before = runtime.map().clone();
    assert!(
        runtime
            .set_tiles(&[
                TileEdit {
                    layer: 1,
                    x: 0,
                    y: 0,
                    gid: 2
                },
                TileEdit {
                    layer: 99,
                    x: 1,
                    y: 0,
                    gid: 1
                }
            ])
            .is_err()
    );
    assert_eq!(*runtime.map(), before);
    assert_eq!(runtime.tile(1, 0, 0).unwrap().gid, 1);
    assert!(
        runtime
            .set_tiles(&[TileEdit {
                layer: 1,
                x: 0,
                y: 0,
                gid: 999
            }])
            .is_err()
    );
    assert!(runtime.set_tile(1, -1, 0, 1).is_err());
    let stats = runtime
        .set_tiles(&[
            TileEdit {
                layer: 1,
                x: 0,
                y: 0,
                gid: 2,
            },
            TileEdit {
                layer: 1,
                x: 0,
                y: 0,
                gid: 1 | FLIP_X,
            },
        ])
        .unwrap();
    assert_eq!(stats.changed_cells, 1);
    assert!(
        runtime
            .collides(
                1,
                Bounds {
                    x: 0.,
                    y: 0.,
                    width: 2.,
                    height: 2.
                },
                "solid"
            )
            .unwrap()
    );
    runtime.set_tile(1, 0, 0, 2).unwrap();
    assert!(
        !runtime
            .collides(
                1,
                Bounds {
                    x: 0.,
                    y: 0.,
                    width: 2.,
                    height: 2.
                },
                "solid"
            )
            .unwrap()
    );
    assert_eq!(stats.indexed_spans, 0);
    assert_eq!(stats.inserted_cells, 0);
    let after = match &runtime.map().layers[0].content {
        LayerContent::Tiles { chunks } => chunks[0].cells.as_ptr(),
        _ => unreachable!(),
    };
    assert_eq!(buffer, after);
    assert_eq!(runtime.set_tiles(&[]).unwrap(), EditStats::default());
    runtime.map().validate().unwrap();
}

#[test]
fn sparse_incremental_index_matches_full_rebuild_after_random_batches() {
    let mut map = parse_fixture(fixture());
    map.infinite = true;
    let mut runtime = Runtime::new(map).unwrap();
    let mut seed = 123456789u64;
    for _ in 0..40 {
        let mut edits = Vec::new();
        for _ in 0..64 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            edits.push(TileEdit {
                layer: 1,
                x: ((seed >> 16) % 40) as i32 - 20,
                y: ((seed >> 32) % 20) as i32 - 10,
                gid: (seed % 3) as u32 | if seed & 4 != 0 { FLIP_X } else { 0 },
            });
        }
        runtime.set_tiles(&edits).unwrap();
        let rebuilt = Runtime::new(runtime.map().clone()).unwrap();
        assert_eq!(runtime.stored_cells(), rebuilt.stored_cells());
        for y in -11..11 {
            for x in -21..21 {
                assert_eq!(runtime.tile(1, x, y), rebuilt.tile(1, x, y));
            }
        }
        let bounds = Bounds {
            x: -42.,
            y: -22.,
            width: 84.,
            height: 44.,
        };
        let mut a = Vec::new();
        let mut b = Vec::new();
        let sa = runtime.for_each_cell(1, bounds, |c| a.push(c)).unwrap();
        let sb = rebuilt.for_each_cell(1, bounds, |c| b.push(c)).unwrap();
        assert_eq!(a, b);
        assert_eq!(sa.visited_cells, sb.visited_cells);
        assert_eq!(
            runtime.animated_tile(0, 0, 60),
            rebuilt.animated_tile(0, 0, 60)
        );
    }
}

#[test]
fn map_batch_validates_every_edit_without_copying_or_partial_mutation() {
    let mut map = parse_fixture(fixture());
    let before = map.clone();
    assert!(
        map.set_tiles(&[
            TileEdit {
                layer: 1,
                x: 0,
                y: 0,
                gid: 2
            },
            TileEdit {
                layer: 1,
                x: 4,
                y: 0,
                gid: 1
            }
        ])
        .is_err()
    );
    assert_eq!(map, before);
    map.set_tiles(&[
        TileEdit {
            layer: 1,
            x: 0,
            y: 0,
            gid: 2,
        },
        TileEdit {
            layer: 1,
            x: 1,
            y: 0,
            gid: 2,
        },
    ])
    .unwrap();
    map.validate().unwrap();
    let mut rectangular = map;
    rectangular.tile_height = 1;
    rectangular.tilesets[0].tile_height = 1;
    rectangular.validate().unwrap();
    let mut runtime = Runtime::new(rectangular).unwrap();
    assert!(runtime.set_tile(1, 0, 0, 1 | TRANSPOSE).is_err());
    assert!(runtime.set_tile(1, i32::MIN, 0, 1).is_err());
}

#[cfg(feature = "render-2d")]
#[test]
fn preloaded_tilesets_share_identity_without_io_and_validate_all_declarations() {
    use quasar_render_2d::{Image, Textures};
    use quasar_tilemap::render::LoadedTilesets;
    let mut map = parse_fixture(fixture());
    let image = Image::new(4, 2, vec![255; 32]).unwrap();
    let mut textures = Textures::default();
    let id = textures.insert(image.clone());
    assert_eq!(
        image.rgba().as_ptr(),
        textures.get(id).unwrap().rgba().as_ptr()
    );
    let mut set = map.tilesets[0].clone();
    set.first_gid = 3;
    set.image = "./textures/../atlas.png".into();
    map.tilesets.push(set);
    let mut calls = 0;
    let loaded = LoadedTilesets::from_textures(&map, &textures, |path| {
        calls += 1;
        assert_eq!(path, "atlas.png");
        Ok(id)
    })
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(
        loaded
            .atlas(0)
            .unwrap()
            .sprite(0, quasar_render_2d::Rect::new(0., 0., 2., 2.))
            .unwrap()
            .texture,
        id
    );
    assert_eq!(
        loaded
            .atlas(1)
            .unwrap()
            .sprite(0, quasar_render_2d::Rect::new(0., 0., 2., 2.))
            .unwrap()
            .texture,
        id
    );
    let bad = textures.declare_external(8, 2).unwrap();
    assert!(LoadedTilesets::from_textures(&map, &textures, |_| Ok(bad)).is_err());
    textures.remove(id);
    assert!(LoadedTilesets::from_textures(&map, &textures, |_| Ok(id)).is_err());
}

#[test]
#[ignore = "release CPU performance comparison; run explicitly with --nocapture"]
fn benchmark_live_edits_versus_legacy_clone_validate_rebuild() {
    use std::{hint::black_box, time::Instant};
    for size in [100u32, 1000, 2048] {
        let mut map = parse_fixture(fixture());
        map.width = size;
        map.height = size;
        map.layers.truncate(1);
        map.layers[0].content = LayerContent::Tiles {
            chunks: vec![Chunk {
                x: 0,
                y: 0,
                width: size,
                height: size,
                cells: vec![1; (size * size) as usize],
            }],
        };
        let mut runtime = Runtime::new(map.clone()).unwrap();
        let edits: Vec<_> = (0..1000)
            .map(|i| TileEdit {
                layer: 1,
                x: i % (size as i32),
                y: i / (size as i32),
                gid: 2,
            })
            .collect();
        let start = Instant::now();
        for e in &edits {
            let stats = runtime.set_tiles(std::slice::from_ref(e)).unwrap();
            assert_eq!(stats.indexed_spans, 0);
            black_box(stats);
        }
        std::hint::black_box(runtime.map());
        let live = start.elapsed();
        let start = Instant::now();
        // Exact old one-cell path: full validation, clone, mutate, full validation,
        // followed by Runtime::new (another validation and index rebuild).
        for e in edits.iter().take(10) {
            map.validate().unwrap();
            let mut candidate = map.clone();
            let LayerContent::Tiles { chunks } = &mut candidate.layers[0].content else {
                unreachable!()
            };
            chunks[0].cells[(e.y as u32 * size + e.x as u32) as usize] = e.gid;
            candidate.validate().unwrap();
            let compiled = black_box(Runtime::new(candidate).unwrap());
            map = compiled.into_map();
        }
        let legacy = start.elapsed();
        println!(
            "cells={} live_ns_per_edit={} legacy_ns_per_edit={} speedup={:.1}",
            size * size,
            live.as_nanos() / 1000,
            legacy.as_nanos() / 10,
            legacy.as_secs_f64() / 10. / (live.as_secs_f64() / 1000.)
        );
    }
}

#[test]
fn live_edit_budgets_reject_complete_batch_before_any_write() {
    let mut map = parse_fixture(fixture());
    map.infinite = true;
    map.layers[0].content = LayerContent::Tiles {
        chunks: vec![Chunk {
            x: 0,
            y: 0,
            width: 2048,
            height: 2048,
            cells: vec![1; MAX_CELLS],
        }],
    };
    let mut runtime = Runtime::new(map).unwrap();
    assert!(
        runtime
            .set_tiles(&[
                TileEdit {
                    layer: 1,
                    x: 0,
                    y: 0,
                    gid: 2
                },
                TileEdit {
                    layer: 1,
                    x: -1,
                    y: 0,
                    gid: 2
                }
            ])
            .is_err()
    );
    assert_eq!(runtime.tile(1, 0, 0).unwrap().gid, 1);
    assert_eq!(runtime.stored_cells(), MAX_CELLS);
    assert!(runtime.tile(1, -1, 0).is_none());
    drop(runtime);
    let mut map = parse_fixture(fixture());
    map.infinite = true;
    map.layers[0].content = LayerContent::Tiles {
        chunks: (0..65536)
            .map(|x| Chunk {
                x: x * 2,
                y: 0,
                width: 1,
                height: 1,
                cells: vec![1],
            })
            .collect(),
    };
    let mut runtime = Runtime::new(map).unwrap();
    assert!(runtime.set_tile(1, 1, 0, 2).is_err());
    assert!(runtime.tile(1, 1, 0).is_none());
    // Replacing cells never consumes chunk budget, including empty cells.
    assert_eq!(runtime.set_tile(1, 0, 0, 0).unwrap().indexed_spans, 0);
    assert_eq!(runtime.tile(1, 0, 0).unwrap().gid, 0);
}

#[test]
fn editing_far_edge_of_valid_infinite_chunk_preserves_origin_bound_semantics() {
    let mut map = parse_fixture(fixture());
    map.infinite = true;
    map.layers[0].content = LayerContent::Tiles {
        chunks: vec![Chunk {
            x: 1_000_000,
            y: 1_000_000,
            width: 2,
            height: 2,
            cells: vec![1; 4],
        }],
    };
    map.set_tile(1, 1_000_001, 1_000_001, 2).unwrap();
    let mut runtime = Runtime::new(map).unwrap();
    runtime.set_tile(1, 1_000_001, 1_000_001, 1).unwrap();
    assert_eq!(runtime.tile(1, 1_000_001, 1_000_001).unwrap().gid, 1);
    assert!(runtime.set_tile(1, 1_000_002, 1_000_001, 2).is_err());
    runtime.map().validate().unwrap();
}
