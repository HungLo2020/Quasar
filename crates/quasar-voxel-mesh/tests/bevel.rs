//! Bevelled blocks: closed, outward-facing meshes whose cuts follow the
//! neighbourhood and agree between neighbours.
use quasar_core::noise::hash3;
use quasar_math::{IVec3, Vec3};
use quasar_voxel::{
    AIR, BlockCatalog, BlockDef, Cube16, Face, Schema, Snapshot, StorageKind, Volume,
};
use quasar_voxel_mesh::*;
use std::collections::HashMap;

const STONE: u16 = 1;
const GROUND: u16 = 2;
const GLASS: u16 = 3;

fn catalog() -> BlockCatalog {
    let mut catalog = BlockCatalog::new();
    catalog.add(BlockDef::cube("stone", 0));
    catalog.add(BlockDef::cube("ground", 1));
    catalog.add(BlockDef::cube("glass", 2).with_layer(quasar_voxel::Layer::Translucent));
    catalog
}

fn policy(block: u16) -> Option<f32> {
    match block {
        STONE => Some(1.0),
        GROUND => Some(0.0),
        _ => None,
    }
}

fn mesh(
    volume: &Volume<Cube16>,
    id: quasar_voxel::ChannelId<u16>,
    options: BevelOptions,
) -> MeshBuffers {
    let catalog = catalog();
    let mut snap = Snapshot::new();
    volume.snapshot(id, IVec3::ZERO, 1, &mut snap);
    let opaque = opaque_table(&catalog);
    let mut out = MeshBuffers::new();
    BevelExtractor::new(options).extract(
        BevelInput::new(&snap, &opaque),
        &catalog,
        policy,
        &mut out,
    );
    out
}

fn key(p: Vec3) -> [i64; 3] {
    [
        (p.x * 4096.).round() as i64,
        (p.y * 4096.).round() as i64,
        (p.z * 4096.).round() as i64,
    ]
}

/// Triangles of every opaque quad (degenerate ones dropped).
fn triangles(mesh: &MeshBuffers) -> Vec<[Vec3; 3]> {
    let mut out = Vec::new();
    for (layer, buffers) in &mesh.layers {
        if *layer != quasar_voxel::Layer::Opaque {
            continue;
        }
        for q in &buffers.quads {
            for t in [[0, 1, 2], [0, 2, 3]] {
                let tri = t.map(|i| q.positions[i]);
                if (tri[1] - tri[0]).cross(tri[2] - tri[0]).length_squared() > 1e-12 {
                    out.push(tri);
                }
            }
        }
    }
    out
}

/// Every directed edge must be matched by its reverse (a closed,
/// consistently wound surface: no cracks between facets or blocks and no
/// T-junctions).
fn assert_closed(tris: &[[Vec3; 3]]) {
    let mut edges: HashMap<([i64; 3], [i64; 3]), i32> = HashMap::new();
    for t in tris {
        for i in 0..3 {
            *edges.entry((key(t[i]), key(t[(i + 1) % 3]))).or_default() += 1;
        }
    }
    let open: Vec<_> = edges
        .iter()
        .filter(|((a, b), n)| edges.get(&(*b, *a)).copied().unwrap_or(0) != **n)
        .collect();
    assert!(
        open.is_empty(),
        "{} unmatched edges, e.g. {:?}",
        open.len(),
        open.first()
    );
}

fn volume_of(tris: &[[Vec3; 3]]) -> f32 {
    tris.iter().map(|t| t[0].dot(t[1].cross(t[2])) / 6.0).sum()
}

#[test]
fn a_single_stone_is_a_closed_bevelled_solid() {
    let mut schema = Schema::new();
    let id = schema.channel("block", StorageKind::Paletted, AIR);
    let mut volume = Volume::<Cube16>::new(schema);
    volume.set(id, IVec3::new(5, 5, 5), STONE);
    let out = mesh(&volume, id, BevelOptions::default());
    let quads = &out.layers[0].1.quads;
    // 6 faces + 12 chamfers + 8 corner facets.
    assert_eq!(quads.len(), 26);
    let tris = triangles(&out);
    assert_closed(&tris);
    let v = volume_of(&tris);
    assert!(
        v > 0.6 && v < 0.99,
        "outward, slightly less than a block: {v}"
    );
    // A flat block is a plain cube.
    let mut schema = Schema::new();
    let id = schema.channel("block", StorageKind::Paletted, AIR);
    let mut volume = Volume::<Cube16>::new(schema);
    volume.set(id, IVec3::new(5, 5, 5), GROUND);
    let out = mesh(&volume, id, BevelOptions::default());
    assert_eq!(out.layers[0].1.quads.len(), 6);
    assert!(out.layers[0].1.quads.iter().all(|q| q.face.is_some()));
    assert!((volume_of(&triangles(&out)) - 1.0).abs() < 1e-4);
}

#[test]
fn random_rock_with_flat_ground_meshes_watertight() {
    for seed in 0..6u32 {
        let mut schema = Schema::new();
        let id = schema.channel("block", StorageKind::Paletted, AIR);
        let mut volume = Volume::<Cube16>::new(schema);
        let mut count = 0;
        for x in 2..12 {
            for y in 2..10 {
                for z in 2..12 {
                    let h = hash3(seed, x, y, z);
                    let block = if y == 2 {
                        GROUND
                    } else if h.is_multiple_of(3) {
                        STONE
                    } else {
                        continue;
                    };
                    volume.set(id, IVec3::new(x, y, z), block);
                    count += 1;
                }
            }
        }
        let options = BevelOptions {
            seed,
            ..BevelOptions::default()
        };
        let out = mesh(&volume, id, options);
        let tris = triangles(&out);
        assert_closed(&tris);
        let v = volume_of(&tris);
        assert!(
            v > count as f32 * 0.7 && v <= count as f32 + 1e-3,
            "seed {seed}: {v} of {count}"
        );
    }
}

#[test]
fn cuts_follow_the_neighbourhood() {
    let mut schema = Schema::new();
    let id = schema.channel("block", StorageKind::Paletted, AIR);
    let mut volume = Volume::<Cube16>::new(schema);
    // A 3x1x1 bar: inner faces are never emitted, the middle block has no
    // corner facets (each corner touches a closed side), and the bar
    // still closes.
    for x in 4..7 {
        volume.set(id, IVec3::new(x, 5, 5), STONE);
    }
    let out = mesh(&volume, id, BevelOptions::default());
    let quads = &out.layers[0].1.quads;
    let faces = quads
        .iter()
        .filter(|q| {
            q.normal.length() > 0.99 && Face::ALL.iter().any(|f| f.normal().as_vec3() == q.normal)
        })
        .count();
    assert_eq!(faces, 3 * 4 + 2, "four sides per block plus the two ends");
    assert_closed(&triangles(&out));
    // Skipped blocks (policy None) are left to other extractors, and glass
    // neighbours (not opaque) keep faces open.
    volume.set(id, IVec3::new(7, 5, 5), GLASS);
    let with_glass = mesh(&volume, id, BevelOptions::default());
    assert_eq!(with_glass.layers.len(), 1, "glass is not meshed here");
    assert_eq!(with_glass.layers[0].1.quads.len(), quads.len());
    // Disabling variation and chips gives uniform cuts of `radius`.
    let uniform = BevelOptions {
        variation: 0.0,
        chip_chance: 0.0,
        tint_variation: 0.0,
        ..BevelOptions::default()
    };
    let out = mesh(&volume, id, uniform);
    let top_face = out.layers[0]
        .1
        .quads
        .iter()
        .find(|q| q.normal == Vec3::Y && q.positions[0].x < 5.0)
        .unwrap();
    let min_z = top_face
        .positions
        .iter()
        .map(|p| p.z)
        .fold(f32::INFINITY, f32::min);
    assert!((min_z - (5.0 + uniform.radius)).abs() < 1e-5, "{min_z}");
    assert!(out.layers[0].1.quads.iter().all(|q| q.tint == 0xffff_ffff));
}

#[test]
fn faces_take_light_from_the_cell_in_front() {
    let mut schema = Schema::new();
    let id = schema.channel("block", StorageKind::Paletted, AIR);
    let light = schema.channel("light", StorageKind::Paletted, 0u8);
    let mut volume = Volume::<Cube16>::new(schema);
    volume.set(id, IVec3::new(5, 5, 5), STONE);
    volume.set(light, IVec3::new(5, 6, 5), 9); // above the block
    let catalog = catalog();
    let mut snap = Snapshot::new();
    volume.snapshot(id, IVec3::ZERO, 1, &mut snap);
    let mut lights = Snapshot::new();
    volume.snapshot(light, IVec3::ZERO, 1, &mut lights);
    let opaque = opaque_table(&catalog);
    let mut out = MeshBuffers::new();
    BevelExtractor::default().extract(
        BevelInput {
            block_light: Some(&lights),
            ..BevelInput::new(&snap, &opaque)
        },
        &catalog,
        policy,
        &mut out,
    );
    let quads = &out.layers[0].1.quads;
    let top = quads.iter().find(|q| q.normal == Vec3::Y).unwrap();
    let bottom = quads.iter().find(|q| q.normal == -Vec3::Y).unwrap();
    // Block light 9, full sky (no sky snapshot).
    assert_eq!(top.light, [0xf9; 4]);
    assert_eq!(bottom.light, [0xf0; 4]);
    // Chamfers touching the top take its light.
    assert!(
        quads
            .iter()
            .any(|q| q.face.is_none() && q.normal.y > 0.3 && q.light[0] == 0xf9)
    );
}
