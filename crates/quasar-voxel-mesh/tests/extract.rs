use quasar_math::{IVec3, Vec3};
use quasar_voxel::{
    AIR, BlockCatalog, BlockDef, Brush, BrushOp, BrushShape, ChannelId, Cube16, EditBatch,
    EditOptions, Face, Layer, Orientation, Schema, Snapshot, StorageKind, Volume,
};
use quasar_voxel_mesh::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};

struct World {
    volume: Volume<Cube16>,
    block: ChannelId<u16>,
    orient: ChannelId<u8>,
    catalog: BlockCatalog,
    stone: u16,
    glass: u16,
    leaves: u16,
}

fn world() -> World {
    let mut schema = Schema::new();
    let block = schema.channel("block", StorageKind::Paletted, AIR);
    let orient = schema.channel("orientation", StorageKind::Paletted, 0u8);
    let mut catalog = BlockCatalog::new();
    let stone = catalog.add(BlockDef::cube("stone", 1));
    let glass = catalog.add(BlockDef::cube("glass", 2).with_layer(Layer::Translucent));
    let leaves = catalog.add(BlockDef::cube("leaves", 3).with_layer(Layer::Cutout));
    World {
        volume: Volume::new(schema),
        block,
        orient,
        catalog,
        stone,
        glass,
        leaves,
    }
}

fn culled(w: &World, coord: IVec3) -> MeshBuffers {
    let mut blocks = Snapshot::new();
    w.volume.snapshot(w.block, coord, 1, &mut blocks);
    let mut orient = Snapshot::new();
    w.volume.snapshot(w.orient, coord, 1, &mut orient);
    let library = ModelLibrary::new();
    let opaque = opaque_table(&w.catalog);
    let mut out = MeshBuffers::new();
    CulledModelExtractor::default().extract(
        CulledInput {
            blocks: &blocks,
            orientation: Some(&orient),
            block_light: None,
            sky_light: None,
            opaque: &opaque,
        },
        &CatalogModels {
            catalog: &w.catalog,
            library: &library,
        },
        &CatalogFaces(&w.catalog),
        &VertexShade::default(),
        &CatalogLayers(&w.catalog),
        |_| true,
        &mut out,
    );
    out
}

fn assert_outward(quad: &Quad) {
    let p = quad.positions;
    let n = (p[1] - p[0]).cross(p[3] - p[0]).normalize();
    assert!(
        n.dot(quad.normal) > 0.99,
        "winding agrees with normal: {quad:?}"
    );
}

#[test]
fn culled_cubes_hide_shared_faces_by_layer_rules() {
    let mut w = world();
    w.volume.set(w.block, IVec3::new(5, 5, 5), w.stone);
    let one = culled(&w, IVec3::ZERO);
    assert_eq!(one.quad_count(), 6);
    for quad in &one.layer(Layer::Opaque).unwrap().quads {
        assert_outward(quad);
        assert_eq!(quad.ao, [3; 4], "nothing nearby");
    }
    w.volume.set(w.block, IVec3::new(6, 5, 5), w.stone);
    assert_eq!(culled(&w, IVec3::ZERO).quad_count(), 10);
    // Glass beside stone: stone shows its face, glass hides its face.
    w.volume.set(w.block, IVec3::new(7, 5, 5), w.glass);
    let mixed = culled(&w, IVec3::ZERO);
    assert_eq!(mixed.layer(Layer::Translucent).unwrap().quads.len(), 5);
    assert_eq!(mixed.layer(Layer::Opaque).unwrap().quads.len(), 10);
    // Glass beside glass hides the shared pair.
    w.volume.set(w.block, IVec3::new(8, 5, 5), w.glass);
    // 4 faces on the first (stone and glass neighbours hide two), 5 on the second.
    assert_eq!(
        culled(&w, IVec3::ZERO)
            .layer(Layer::Translucent)
            .unwrap()
            .quads
            .len(),
        9
    );
    // Cutout never hides neighbours.
    w.volume.set(w.block, IVec3::new(9, 5, 5), w.leaves);
    let leafy = culled(&w, IVec3::ZERO);
    assert_eq!(leafy.layer(Layer::Translucent).unwrap().quads.len(), 9);
    assert_eq!(
        leafy.layer(Layer::Cutout).unwrap().quads.len(),
        6,
        "glass does not hide the leaves"
    );
}

#[test]
fn ambient_occlusion_darkens_corners_and_flips_diagonals() {
    let mut w = world();
    for x in 0..3 {
        for z in 0..3 {
            w.volume.set(w.block, IVec3::new(x + 2, 2, z + 2), w.stone);
        }
    }
    w.volume.set(w.block, IVec3::new(3, 3, 3), w.stone);
    let mesh = culled(&w, IVec3::ZERO);
    let tops: Vec<&Quad> = mesh
        .layer(Layer::Opaque)
        .unwrap()
        .quads
        .iter()
        .filter(|q| q.face == Some(Face::PosY) && q.positions[0].y == 3.0)
        .collect();
    assert_eq!(tops.len(), 8, "the centre top is covered");
    assert!(tops.iter().any(|q| q.ao.contains(&1) || q.ao.contains(&2)));
    for q in &tops {
        let flip =
            u16::from(q.ao[0]) + u16::from(q.ao[2]) < u16::from(q.ao[1]) + u16::from(q.ao[3]);
        assert_eq!(q.flipped, flip);
    }
}

/// Parity hooks: a custom policy set fully determines output and order.
#[test]
fn policies_and_iteration_order_control_output_exactly() {
    struct NeverHide;
    impl FacePolicy for NeverHide {
        fn hidden(&self, _: CellInfo, _: CellInfo, _: Face) -> bool {
            false
        }
    }
    struct OneQuad;
    impl ModelSource for OneQuad {
        fn quads(&self, block: u16, _: Orientation, out: &mut Vec<ModelQuad>) {
            let mut q = cube_quad(Face::PosY, u32::from(block) * 10);
            q.shade = false;
            out.push(q);
        }
    }
    struct Flat;
    impl ShadeSource for Flat {
        fn shade(&self, _: &ShadeInput<'_>, cell: IVec3, _: &ModelQuad, quad: &mut Quad) {
            quad.light = [(cell.x as u8) & 15; 4];
        }
    }
    struct ByBlock;
    impl LayerRouter for ByBlock {
        fn layer(&self, block: u16, _: &ModelQuad) -> Layer {
            Layer::Custom(block as u8)
        }
    }
    let mut w = world();
    let cells = [
        IVec3::new(1, 0, 0),
        IVec3::new(0, 1, 0),
        IVec3::new(0, 0, 1),
        IVec3::new(2, 0, 0),
    ];
    for (i, c) in cells.iter().enumerate() {
        w.volume.set(w.block, *c, 1 + i as u16);
    }
    let mut blocks = Snapshot::new();
    w.volume.snapshot(w.block, IVec3::ZERO, 1, &mut blocks);
    let run = |order: IterationOrder| {
        let mut out = MeshBuffers::new();
        CulledModelExtractor::new(order).extract(
            CulledInput {
                blocks: &blocks,
                orientation: None,
                block_light: None,
                sky_light: None,
                opaque: &[],
            },
            &OneQuad,
            &NeverHide,
            &Flat,
            &ByBlock,
            |_| true,
            &mut out,
        );
        out.layers
            .iter()
            .flat_map(|(layer, b)| {
                b.quads
                    .iter()
                    .map(move |q| (*layer, q.material, q.positions[0].as_ivec3()))
            })
            .collect::<Vec<_>>()
    };
    // Default Y, Z, X order and custom X, Y, Z order differ deterministically.
    let default = run(IterationOrder::default());
    assert_eq!(default.len(), 4);
    let mut sink_order = Vec::new();
    {
        struct Recorder<'a>(&'a mut Vec<u32>);
        impl MeshSink for Recorder<'_> {
            fn quad(&mut self, _: Layer, q: &Quad) {
                self.0.push(q.material);
            }
        }
        let mut rec = Recorder(&mut sink_order);
        CulledModelExtractor::new(IterationOrder([0, 1, 2])).extract(
            CulledInput {
                blocks: &blocks,
                orientation: None,
                block_light: None,
                sky_light: None,
                opaque: &[],
            },
            &OneQuad,
            &NeverHide,
            &Flat,
            &ByBlock,
            |_| true,
            &mut rec,
        );
    }
    assert_eq!(
        sink_order,
        vec![30, 20, 10, 40],
        "x slowest: (0,0,1), (0,1,0), (1,0,0), (2,0,0)"
    );
    let mut yzx = Vec::new();
    {
        struct Recorder<'a>(&'a mut Vec<u32>);
        impl MeshSink for Recorder<'_> {
            fn quad(&mut self, _: Layer, q: &Quad) {
                self.0.push(q.material);
            }
        }
        let mut rec = Recorder(&mut yzx);
        CulledModelExtractor::default().extract(
            CulledInput {
                blocks: &blocks,
                orientation: None,
                block_light: None,
                sky_light: None,
                opaque: &[],
            },
            &OneQuad,
            &NeverHide,
            &Flat,
            &ByBlock,
            |_| true,
            &mut rec,
        );
    }
    assert_eq!(yzx, vec![10, 40, 30, 20], "y slowest then z then x");
    assert!(
        default
            .iter()
            .all(|(layer, m, _)| *layer == Layer::Custom((*m / 10) as u8))
    );
}

#[test]
fn rotated_blocks_and_custom_models_follow_orientation() {
    let mut w = world();
    let mut library = ModelLibrary::new();
    let slab = library.add(box_quads(Vec3::ZERO, Vec3::new(1.0, 0.5, 1.0), [7; 6]));
    let def = BlockDef::cube("slab", 7)
        .with_model(slab)
        .with_faces([quasar_voxel::FaceShape::Shape(1); 6]);
    let slab_id = w.catalog.add(def);
    let up_x = Orientation::from_up_forward(Face::PosX, Face::PosZ).unwrap();
    w.volume.set(w.block, IVec3::new(4, 4, 4), slab_id);
    w.volume.set(w.orient, IVec3::new(4, 4, 4), up_x.0);
    let mut blocks = Snapshot::new();
    w.volume.snapshot(w.block, IVec3::ZERO, 1, &mut blocks);
    let mut orient = Snapshot::new();
    w.volume.snapshot(w.orient, IVec3::ZERO, 1, &mut orient);
    let opaque = opaque_table(&w.catalog);
    let mut out = MeshBuffers::new();
    CulledModelExtractor::default().extract(
        CulledInput {
            blocks: &blocks,
            orientation: Some(&orient),
            block_light: None,
            sky_light: None,
            opaque: &opaque,
        },
        &CatalogModels {
            catalog: &w.catalog,
            library: &library,
        },
        &CatalogFaces(&w.catalog),
        &VertexShade::default(),
        &CatalogLayers(&w.catalog),
        |_| true,
        &mut out,
    );
    let quads = &out.layer(Layer::Opaque).unwrap().quads;
    assert_eq!(quads.len(), 6);
    let max_x = quads
        .iter()
        .flat_map(|q| q.positions)
        .map(|p| p.x)
        .fold(f32::MIN, f32::max);
    let max_y = quads
        .iter()
        .flat_map(|q| q.positions)
        .map(|p| p.y)
        .fold(f32::MIN, f32::max);
    assert!(
        (max_x - 4.5).abs() < 1e-5 && (max_y - 5.0).abs() < 1e-5,
        "{max_x} {max_y}"
    );
    let min_x = quads
        .iter()
        .flat_map(|q| q.positions)
        .map(|p| p.x)
        .fold(f32::MAX, f32::min);
    assert!(
        (min_x - 4.0).abs() < 1e-5,
        "the slab's bottom now faces -X: {min_x}"
    );
    quads.iter().for_each(assert_outward);
}

/// Every visible unit face of greedy output, as (face, cell).
fn faces_of(quads: &[Quad]) -> BTreeMap<(u8, (i32, i32, i32)), u32> {
    let mut out = BTreeMap::new();
    for q in quads {
        let face = q.face.unwrap();
        let min = q
            .positions
            .iter()
            .fold(Vec3::splat(f32::MAX), |a, p| a.min(*p));
        let max = q
            .positions
            .iter()
            .fold(Vec3::splat(f32::MIN), |a, p| a.max(*p));
        let axis = face.axis();
        let mut cell = min.as_ivec3();
        if face.is_positive() {
            cell[axis] -= 1;
        }
        let (ua, va) = match axis {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        for du in 0..(max[ua] - min[ua]) as i32 {
            for dv in 0..(max[va] - min[va]) as i32 {
                let mut c = cell;
                c[ua] += du;
                c[va] += dv;
                let previous = out.insert((face as u8, (c.x, c.y, c.z)), q.material);
                assert!(previous.is_none(), "faces never overlap");
            }
        }
    }
    out
}

#[test]
fn greedy_meshing_covers_exactly_the_visible_faces() {
    let mut w = world();
    let mut rng = quasar_core::Rng::seed_from_u64(9);
    for x in -1..17 {
        for z in -1..17 {
            let height = 3 + rng.range_i32(0..6);
            for y in -1..height {
                let b = if rng.chance(0.08) { w.glass } else { w.stone };
                w.volume.set(w.block, IVec3::new(x, y, z), b);
            }
        }
    }
    let reference = culled(&w, IVec3::ZERO);
    let mut blocks = Snapshot::new();
    w.volume.snapshot(w.block, IVec3::ZERO, 1, &mut blocks);
    let opaque = opaque_table(&w.catalog);
    for ao in [false, true] {
        let mut greedy = MeshBuffers::new();
        let emitted = GreedyExtractor::new(GreedyOptions { ao, light: false }).extract(
            &blocks,
            None,
            None,
            None,
            &opaque,
            &CatalogGreedy::new(&w.catalog),
            &mut greedy,
        );
        assert_eq!(emitted, greedy.quad_count());
        for layer in [Layer::Opaque, Layer::Translucent] {
            let expected = faces_of(&reference.layer(layer).unwrap().quads);
            let got = faces_of(&greedy.layer(layer).unwrap().quads);
            assert_eq!(got, expected, "{layer:?} with ao={ao}");
        }
        greedy
            .layer(Layer::Opaque)
            .unwrap()
            .quads
            .iter()
            .for_each(assert_outward);
        assert!(
            greedy.quad_count() < reference.quad_count(),
            "merging reduces quads: {} vs {}",
            greedy.quad_count(),
            reference.quad_count()
        );
    }
    // A flat slab's top becomes one quad without AO.
    let mut flat = world();
    for x in 0..16 {
        for z in 0..16 {
            flat.volume.set(flat.block, IVec3::new(x, 0, z), flat.stone);
        }
    }
    let mut blocks = Snapshot::new();
    flat.volume
        .snapshot(flat.block, IVec3::ZERO, 1, &mut blocks);
    let mut out = MeshBuffers::new();
    GreedyExtractor::new(GreedyOptions {
        ao: false,
        light: false,
    })
    .extract(
        &blocks,
        None,
        None,
        None,
        &opaque_table(&flat.catalog),
        &CatalogGreedy::new(&flat.catalog),
        &mut out,
    );
    let tops: Vec<_> = out
        .layer(Layer::Opaque)
        .unwrap()
        .quads
        .iter()
        .filter(|q| q.face == Some(Face::PosY))
        .collect();
    assert_eq!(tops.len(), 1);
    assert_eq!(tops[0].size, [16, 16]);
    assert_eq!(out.quad_count(), 6);
}

fn sphere_volume(center: Vec3, radius: f32) -> (Volume<Cube16>, ChannelId<i8>, ChannelId<u8>) {
    let mut schema = Schema::new();
    let density = schema.channel("density", StorageKind::Dense, -127i8);
    let material = schema.channel("material", StorageKind::Paletted, 0u8);
    let mut volume = Volume::new(schema);
    let mut batch = EditBatch::new();
    batch.brush(
        Brush::new(BrushShape::Sphere { center, radius }, BrushOp::Add, density)
            .with_material(material, 4u8),
    );
    batch.brush(
        Brush::new(
            BrushShape::Sphere {
                center: center + Vec3::X * radius,
                radius: radius * 0.5,
            },
            BrushOp::Paint,
            density,
        )
        .with_material(material, 9u8),
    );
    volume.apply(&batch, EditOptions::default());
    (volume, density, material)
}

fn smooth_mesh(
    volume: &Volume<Cube16>,
    density: ChannelId<i8>,
    material: ChannelId<u8>,
    coord: IVec3,
    options: SmoothOptions,
) -> (MeshBuffers, SmoothStats) {
    let mut d = Snapshot::new();
    let mut raw: Snapshot<u8> = Snapshot::new();
    let _ = &mut raw;
    volume.snapshot(density, coord, 1, &mut d);
    let mut m = Snapshot::new();
    volume.snapshot(material, coord, 1, &mut m);
    let mut out = MeshBuffers::new();
    let stats = SurfaceNetsExtractor::new().extract(&d, Some(&m), options, &mut out);
    (out, stats)
}

#[test]
fn surface_nets_are_closed_outward_and_accurate() {
    let center = Vec3::splat(8.0);
    let (volume, density, material) = sphere_volume(center, 5.0);
    let (mesh, stats) = smooth_mesh(
        &volume,
        density,
        material,
        IVec3::ZERO,
        SmoothOptions::default(),
    );
    let layer = mesh.layer(Layer::Opaque).unwrap();
    assert!(stats.triangles > 200 && stats.skirt_triangles == 0);
    // Closed: every edge shared by exactly two triangles.
    let mut edges: HashMap<(u32, u32), i32> = HashMap::new();
    for t in layer.indices.chunks(3) {
        for i in 0..3 {
            let (a, b) = (t[i], t[(i + 1) % 3]);
            *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
        }
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| layer.vertices[i as usize]);
        let n = (b.position - a.position).cross(c.position - a.position);
        let centroid = (a.position + b.position + c.position) / 3.0;
        assert!(
            n.dot(centroid - center) > 0.0,
            "triangles face outward: {a:?} {b:?} {c:?}"
        );
        assert!(a.normal.dot(a.position - center) > 0.8);
    }
    assert!(edges.values().all(|c| *c == 2), "watertight");
    for v in &layer.vertices {
        let r = (v.position - center).length();
        assert!((r - 5.0).abs() < 0.35, "on the sphere: {r}");
    }
    assert!(
        layer
            .vertices
            .iter()
            .any(|v| v.materials[0] == 9 || v.materials[1] == 9),
        "painted cap"
    );
    assert!(layer.vertices.iter().any(|v| v.materials == [4, 4]));
}

#[test]
fn neighbouring_chunks_meet_without_cracks_and_skirts_cover_seams() {
    let center = Vec3::new(16.0, 8.0, 8.0);
    let (volume, density, material) = sphere_volume(center, 6.0);
    let mut positions: Vec<[Vec3; 3]> = Vec::new();
    for coord in [IVec3::ZERO, IVec3::X] {
        let (mesh, stats) =
            smooth_mesh(&volume, density, material, coord, SmoothOptions::default());
        assert_eq!(stats.skirt_triangles, 0);
        let layer = mesh.layer(Layer::Opaque).unwrap();
        let origin = coord.as_vec3() * 16.0;
        for t in layer.indices.chunks(3) {
            positions
                .push([t[0], t[1], t[2]].map(|i| layer.vertices[i as usize].position + origin));
        }
    }
    // Watertight across the chunk border: match edges by position.
    let k = |p: Vec3| {
        (
            (p.x * 1000.0).round() as i64,
            (p.y * 1000.0).round() as i64,
            (p.z * 1000.0).round() as i64,
        )
    };
    let mut edges: HashMap<_, i32> = HashMap::new();
    for t in &positions {
        for i in 0..3 {
            let (a, b) = (k(t[i]), k(t[(i + 1) % 3]));
            *edges
                .entry(if a < b { (a, b) } else { (b, a) })
                .or_insert(0) += 1;
        }
    }
    assert!(edges.values().all(|c| *c == 2), "seamless across chunks");
    let seam = SmoothOptions {
        seam_faces: Face::PosX.bit(),
        ..Default::default()
    };
    let (_, with_skirt) = smooth_mesh(&volume, density, material, IVec3::ZERO, seam);
    assert!(with_skirt.skirt_triangles > 0);
    let other_face = SmoothOptions {
        seam_faces: Face::NegY.bit(),
        ..Default::default()
    };
    let (_, untouched) = smooth_mesh(&volume, density, material, IVec3::ZERO, other_face);
    assert_eq!(
        untouched.skirt_triangles, 0,
        "the sphere does not cross the -Y face"
    );
    let empty_schema = sphere_volume(Vec3::splat(100.0), 1.0);
    let (empty, stats) = smooth_mesh(
        &empty_schema.0,
        empty_schema.1,
        empty_schema.2,
        IVec3::ZERO,
        SmoothOptions::default(),
    );
    assert!(empty.is_empty() && stats.vertices == 0);
}

#[test]
fn gpu_formats_round_trip() {
    let mut w = world();
    for x in 0..5 {
        for z in 0..3 {
            w.volume.set(w.block, IVec3::new(x, 0, z), w.stone);
        }
    }
    w.volume.set(w.block, IVec3::new(2, 1, 1), w.stone);
    let mut blocks = Snapshot::new();
    w.volume.snapshot(w.block, IVec3::ZERO, 1, &mut blocks);
    let mut out = MeshBuffers::new();
    GreedyExtractor::new(GreedyOptions {
        ao: true,
        light: false,
    })
    .extract(
        &blocks,
        None,
        None,
        None,
        &opaque_table(&w.catalog),
        &CatalogGreedy::new(&w.catalog),
        &mut out,
    );
    for quad in &out.layer(Layer::Opaque).unwrap().quads {
        let packed = PackedQuad::encode(quad).unwrap();
        let decoded = packed.decode();
        let set = |q: &Quad| {
            q.positions
                .iter()
                .map(|p| (p.as_ivec3().to_array(), 0))
                .collect::<BTreeSet<_>>()
        };
        assert_eq!(set(&decoded), set(quad), "same corners");
        for (i, p) in decoded.positions.iter().enumerate() {
            let source = quad.positions.iter().position(|s| s == p).unwrap();
            assert_eq!(decoded.ao[i], quad.ao[source]);
        }
        assert_eq!(decoded.material, quad.material);
        assert_eq!(decoded.face, quad.face);
        assert_outward(&decoded);
        assert_eq!(PackedQuad::encode(&decoded).unwrap(), packed);
    }
    let culled_mesh = culled(&w, IVec3::ZERO);
    let mut off = culled_mesh.layer(Layer::Opaque).unwrap().quads[0];
    off.positions[0].x += 0.5;
    assert_eq!(PackedQuad::encode(&off), Err(PackError::OffLattice));
    let v = ModelVertex::encode(
        Vec3::new(-0.75, 33.5, 12.125),
        Vec3::new(0.0, 0.6, 0.8),
        quasar_math::Vec2::new(0.25, 2.5),
        513,
        2,
        0x3c,
        0xff00ff00,
    );
    let (p, n, uv, m, ao, light, tint) = v.decode();
    assert!((p - Vec3::new(-0.75, 33.5, 12.125)).length() < 1e-3);
    assert!(n.dot(Vec3::new(0.0, 0.6, 0.8)) > 0.999);
    assert!((uv.x - 0.25).abs() < 1e-3 && (uv.y - 2.5).abs() < 1e-3);
    assert_eq!((m, ao, light, tint), (513, 2, 0x3c, 0xff00ff00));
    let mut rng = quasar_core::Rng::seed_from_u64(2);
    for _ in 0..1000 {
        let d = Vec3::new(
            rng.range_f32(-1.0..1.0),
            rng.range_f32(-1.0..1.0),
            rng.range_f32(-1.0..1.0),
        )
        .normalize_or(Vec3::Y);
        assert!(oct_decode(oct_encode(d)).dot(d) > 0.999, "{d}");
    }
    let s = SmoothVertex {
        position: Vec3::new(3.3, -2.0, 40.0),
        normal: Vec3::X,
        materials: [3, 900],
        blend: 77,
    };
    let back = PackedSmoothVertex::encode(&s).decode();
    assert!((back.position - s.position).length() < 1e-3);
    assert_eq!((back.materials, back.blend), (s.materials, s.blend));
    let gpu = GpuLayer::encode(culled_mesh.layer(Layer::Opaque).unwrap());
    assert_eq!(
        gpu.packed.len(),
        culled_mesh.layer(Layer::Opaque).unwrap().quads.len()
    );
    assert!(WGSL_FORMATS.contains("fn qv_packed_quad_vertex"));
}

#[test]
fn translucent_sorters_order_back_to_front() {
    let quad_at = |z: f32| Quad {
        positions: [
            Vec3::new(0., 0., z),
            Vec3::new(1., 0., z),
            Vec3::new(1., 1., z),
            Vec3::new(0., 1., z),
        ],
        normal: Vec3::Z,
        face: Some(Face::PosZ),
        uv: [quasar_math::Vec2::ZERO; 4],
        material: 0,
        ao: [3; 4],
        light: [0; 4],
        tint: u32::MAX,
        flipped: false,
        size: [1, 1],
    };
    let quads = vec![quad_at(1.0), quad_at(5.0), quad_at(3.0)];
    let mut order = Vec::new();
    assert!(CentroidSorter::default().sort(&quads, Vec3::new(0.5, 0.5, 10.0), &mut order));
    assert_eq!(order, vec![0, 2, 1]);
    let mut plane = PlaneSorter::default();
    let mut order = Vec::new();
    assert!(plane.sort(&quads, Vec3::new(0.5, 0.5, 10.0), &mut order));
    assert!(
        !plane.sort(&quads, Vec3::new(0.7, 0.5, 9.0), &mut order),
        "no plane crossed"
    );
    assert!(
        plane.sort(&quads, Vec3::new(0.5, 0.5, 4.0), &mut order),
        "crossed z = 5"
    );
    assert_eq!(
        order,
        vec![0, 1, 2],
        "farthest first; equal distances keep emission order"
    );
}

#[cfg(feature = "jobs")]
#[test]
fn remesh_queue_drops_stale_results() {
    let pool = quasar_jobs::JobPool::new(quasar_jobs::JobPoolConfig::new(2, 64)).unwrap();
    let mut queue = RemeshQueue::<(i32, i32, i32), u64>::new();
    assert!(queue.request(&pool, (0, 0, 0), 1, 0, || 1).unwrap());
    assert!(
        !queue.request(&pool, (0, 0, 0), 1, 0, || 1).unwrap(),
        "duplicate"
    );
    queue.request(&pool, (0, 0, 0), 2, 0, || 2).unwrap();
    queue.request(&pool, (1, 0, 0), 5, 0, || 5).unwrap();
    let done = queue.wait_all();
    assert_eq!(done, vec![((0, 0, 0), 2, 2), ((1, 0, 0), 5, 5)]);
    queue.request(&pool, (2, 0, 0), 1, 0, || 7).unwrap();
    queue.forget((2, 0, 0));
    assert!(queue.wait_all().is_empty());
}

#[test]
fn packed_quads_texture_like_model_quads() {
    use quasar_voxel::Face;
    for face in Face::ALL {
        let model = cube_quad(face, 3);
        let quad = Quad {
            positions: model.positions.map(|p| p + Vec3::new(2.0, 5.0, 7.0)),
            normal: face.normal().as_vec3(),
            face: Some(face),
            uv: model.uv,
            material: 3,
            ao: [3; 4],
            light: [0; 4],
            tint: u32::MAX,
            flipped: false,
            size: [1, 1],
        };
        let decoded = PackedQuad::encode(&quad).unwrap().decode();
        for (p, uv) in decoded.positions.iter().zip(decoded.uv) {
            let i = quad
                .positions
                .iter()
                .position(|q| (*q - *p).length() < 1e-5)
                .unwrap();
            assert_eq!(uv, quad.uv[i], "{face:?} corner {p}");
        }
    }
}

#[test]
fn partial_height_blocks_mesh_as_boxes_and_cull_by_derived_shapes() {
    use quasar_voxel::FaceShape;
    let mut w = world();
    let wall = w.catalog.add(BlockDef::cube("wall", 4).with_height(0.6));
    let def = w.catalog.get(wall).unwrap();
    assert!(def.is_partial() && !def.full_cube && !def.opaque);
    assert_eq!(def.faces[Face::NegY.index()], FaceShape::Full);
    assert_eq!(def.faces[Face::PosY.index()], FaceShape::None);
    assert!(matches!(def.faces[Face::NegX.index()], FaceShape::Shape(_)));
    assert_eq!(def.faces[Face::NegX.index()], def.faces[Face::PosZ.index()]);
    let slab = BlockDef::partial("slab", 4, [0.; 3], [1., 0.5, 1.]);
    assert_ne!(
        slab.faces[Face::NegX.index()],
        def.faces[Face::NegX.index()]
    );
    assert!(!CatalogGreedy::new(&w.catalog).handles(wall));
    let a = IVec3::new(5, 5, 5);
    w.volume.set(w.block, a, wall);
    w.volume.set(w.block, IVec3::new(6, 5, 5), wall);
    w.volume.set(w.block, IVec3::new(5, 5, 6), w.stone);
    w.volume.set(w.block, IVec3::new(5, 4, 5), w.stone);
    let mesh = culled(&w, IVec3::ZERO);
    let quads = &mesh.layer(Layer::Opaque).unwrap().quads;
    // Wall A: walls cull each other, the stone beside and below hide its
    // side and bottom; its inset top always draws (3). Wall B: 5. The
    // stone beside the wall keeps the face the low wall cannot cover (6);
    // the stone below is covered by the wall's full bottom (5).
    assert_eq!(quads.len(), 19);
    let tops: Vec<_> = quads
        .iter()
        .filter(|q| q.normal.y > 0.9 && (q.positions[0].y - 5.6).abs() < 1e-4)
        .collect();
    assert_eq!(tops.len(), 2, "both walls draw their top at 0.6");
    for quad in quads {
        assert_outward(quad);
        assert!(quad.positions.iter().all(|p| p.y <= 6.0 + 1e-4));
    }
}
