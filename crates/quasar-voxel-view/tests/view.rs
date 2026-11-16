use quasar_math::{IVec3, Mat4, Vec3};
use quasar_voxel::{Cube16, Face, GridId, Layer, Schema, Snapshot, StorageKind, Volume};
use quasar_voxel_view::*;
use std::collections::BTreeSet;

fn blocks() -> (Volume<Cube16>, quasar_voxel::ChannelId<u16>) {
    let mut schema = Schema::new();
    let block = schema.channel("block", StorageKind::Paletted, 0u16);
    (Volume::new(schema), block)
}

#[test]
fn connectivity_follows_open_space_inside_a_chunk() {
    let (mut volume, block) = blocks();
    let opaque = |v: u16| v != 0;
    let mut scratch = Vec::new();
    assert_eq!(
        chunk_connectivity(&volume, block, IVec3::ZERO, &opaque, &mut scratch),
        Connectivity::ALL
    );
    for y in 0..16 {
        for z in 0..16 {
            volume.set(block, IVec3::new(8, y, z), 1);
        }
    }
    let wall = chunk_connectivity(&volume, block, IVec3::ZERO, &opaque, &mut scratch);
    assert!(!wall.connects(Face::NegX, Face::PosX));
    assert!(wall.connects(Face::NegY, Face::PosY));
    assert!(wall.connects(Face::NegX, Face::PosZ));
    assert!(wall.connects(Face::PosX, Face::NegZ));
    for x in 0..16 {
        for y in 0..16 {
            for z in 0..16 {
                volume.set(block, IVec3::new(x, y, z), 1);
            }
        }
    }
    let mut chunk = volume.chunk(IVec3::ZERO).unwrap().clone();
    chunk.compact(volume.schema());
    volume.insert_chunk(IVec3::ZERO, chunk);
    assert_eq!(
        chunk_connectivity(&volume, block, IVec3::ZERO, &opaque, &mut scratch),
        Connectivity::NONE
    );
}

struct Admit(Box<dyn Fn(IVec3) -> bool>, [Face; 6], bool);
impl TraversalPolicy for Admit {
    fn admit(&self, chunk: IVec3) -> bool {
        (self.0)(chunk)
    }
    fn face_order(&self) -> [Face; 6] {
        self.1
    }
    fn use_connectivity(&self) -> bool {
        self.2
    }
}

#[test]
fn traversal_respects_connectivity_direction_and_policy_order() {
    let mut graph = VisibilityGraph::new();
    for x in -3..=3 {
        for y in -1..=1 {
            for z in -1..=1 {
                graph.set(IVec3::new(x, y, z), Connectivity::ALL);
            }
        }
    }
    // A sealed chunk at x = 2 blocks everything beyond it along +X.
    graph.set(IVec3::new(2, 0, 0), Connectivity::NONE);
    let only_line = Admit(Box::new(|c: IVec3| c.y == 0 && c.z == 0), Face::ALL, true);
    let visited = graph.traverse(IVec3::ZERO, &only_line);
    assert!(
        visited.contains(&IVec3::new(2, 0, 0)),
        "the sealed chunk itself is visible"
    );
    assert!(
        !visited.contains(&IVec3::new(3, 0, 0)),
        "but not entered through"
    );
    assert!(visited.contains(&IVec3::new(-3, 0, 0)));
    let plain = Admit(Box::new(|c: IVec3| c.y == 0 && c.z == 0), Face::ALL, false);
    assert!(
        graph
            .traverse(IVec3::ZERO, &plain)
            .contains(&IVec3::new(3, 0, 0))
    );
    // Face order decides the visit order of neighbours.
    let reversed = {
        let mut f = Face::ALL;
        f.reverse();
        f
    };
    let forward = graph.traverse(IVec3::ZERO, &Admit(Box::new(|_| true), Face::ALL, true));
    let backward = graph.traverse(IVec3::ZERO, &Admit(Box::new(|_| true), reversed, true));
    assert_eq!(forward[1], IVec3::new(-1, 0, 0));
    assert_eq!(backward[1], IVec3::new(0, 0, 1));
    assert_eq!(
        forward
            .iter()
            .copied()
            .map(|c| (c.x, c.y, c.z))
            .collect::<BTreeSet<_>>(),
        backward
            .iter()
            .copied()
            .map(|c| (c.x, c.y, c.z))
            .collect::<BTreeSet<_>>(),
        "same set, different order"
    );
    // A U-turn needs the reverse direction, which traversal never takes.
    let mut corridor = VisibilityGraph::new();
    for x in 0..3 {
        corridor.set(IVec3::new(x, 0, 0), Connectivity::ALL);
    }
    corridor.set(IVec3::new(2, 1, 0), Connectivity::ALL);
    corridor.set(IVec3::new(1, 1, 0), Connectivity::ALL);
    let seen = corridor.traverse(IVec3::ZERO, &Admit(Box::new(|_| true), Face::ALL, true));
    assert!(seen.contains(&IVec3::new(2, 1, 0)));
    assert!(
        seen.contains(&IVec3::new(1, 1, 0)),
        "reachable via +Y from (1,0,0)"
    );
}

#[test]
fn frustum_rejects_chunks_behind_and_beyond_range() {
    let view = quasar_math::look_to(Vec3::new(8.0, 8.0, 8.0), -Vec3::Z, Vec3::Y).unwrap();
    let proj = quasar_math::perspective(1.2, 1.0, 0.1, 500.0).unwrap();
    let frustum = ChunkFrustum::new(proj * view, 16, Vec3::splat(8.0), 160.0);
    assert!(frustum.contains(IVec3::ZERO));
    assert!(frustum.contains(IVec3::new(0, 0, -3)));
    assert!(!frustum.contains(IVec3::new(0, 0, 4)), "behind the camera");
    assert!(!frustum.contains(IVec3::new(0, 0, -20)), "beyond range");
    let mut graph = VisibilityGraph::new();
    for z in -12..=12 {
        graph.set(IVec3::new(0, 0, z), Connectivity::ALL);
    }
    let visible = graph.traverse(
        IVec3::ZERO,
        &FrustumPolicy {
            frustum: &frustum,
            connectivity: true,
        },
    );
    assert!(visible.iter().all(|c| c.z <= 1));
    assert!(visible.contains(&IVec3::new(0, 0, -9)));
}

#[test]
fn streaming_plans_nearest_first_with_budgets_and_hysteresis() {
    let planner = StreamingPlanner::new(StreamBudget {
        max_loads: 10,
        max_unloads: 100,
    });
    let mut viewer = Viewer::new(Vec3::new(8.0, 8.0, 8.0), 2);
    viewer.vertical = Some((0, 0));
    let plan = planner.plan(16, &[viewer], &[]);
    assert_eq!(plan.load.len(), 10);
    assert_eq!(plan.load[0], IVec3::ZERO);
    assert_eq!(plan.remaining, 21 - 10, "a radius-2 disc has 21 chunks");
    let all = planner.plan(16, &[viewer], &[]);
    assert_eq!(plan, all, "deterministic");
    // Forward bias: chunks ahead load before equally distant ones behind.
    viewer.forward = Some(Vec3::X);
    let biased = planner.plan(16, &[viewer], &[IVec3::ZERO]);
    let ahead = biased
        .load
        .iter()
        .position(|c| *c == IVec3::new(1, 0, 0))
        .unwrap();
    let behind = biased
        .load
        .iter()
        .position(|c| *c == IVec3::new(-1, 0, 0))
        .unwrap_or(usize::MAX);
    assert!(ahead < behind);
    let loaded = [
        IVec3::new(0, 0, 3),
        IVec3::new(0, 0, 4),
        IVec3::new(0, 1, 0),
    ];
    let plan = planner.plan(16, &[viewer], &loaded);
    assert_eq!(
        plan.unload,
        vec![IVec3::new(0, 0, 4)],
        "the keep margin holds z = 3 and y = 1"
    );
}

#[test]
fn lod_selection_covers_space_once_and_flags_seams() {
    let config = LodConfig {
        cells: 8,
        max_level: 3,
        split: 1.5,
        hysteresis: 0.2,
    };
    let mut selector = LodSelector::new(config);
    let roots = [IVec3::ZERO, IVec3::new(1, 0, 0)];
    let camera = Vec3::new(2.0, 2.0, 2.0);
    let selected = selector.select(&roots, camera);
    // Coverage: every voxel of both roots belongs to exactly one leaf.
    let root_size = 8 << 3;
    let mut covered = 0i64;
    for s in &selected {
        covered += i64::from(s.node.size(8)).pow(3);
        assert!(s.node.origin(8).x >= 0 && s.node.origin(8).x + s.node.size(8) <= 2 * root_size);
    }
    assert_eq!(covered, 2 * i64::from(root_size).pow(3));
    let near = selected
        .iter()
        .find(|s| s.node.contains_voxel(8, camera.as_ivec3()))
        .unwrap();
    assert_eq!(near.node.level, 0, "finest at the camera");
    let far = selected
        .iter()
        .find(|s| s.node.contains_voxel(8, IVec3::new(120, 60, 60)))
        .unwrap();
    assert!(far.node.level >= 2, "coarse far away");
    assert!(selected.iter().any(|s| s.seam_faces != 0));
    let flat: Vec<_> = selected
        .iter()
        .filter(|s| s.node.level == 0 && s.seam_faces == 0)
        .collect();
    assert!(!flat.is_empty(), "interior fine nodes have no seams");
    // Hysteresis: a tiny move keeps the same selection.
    let again = selector.select(&roots, camera + Vec3::splat(0.3));
    assert_eq!(again, selected);
    assert_eq!(
        LodNode {
            level: 1,
            coord: IVec3::new(-1, 0, 3)
        }
        .parent(),
        LodNode {
            level: 2,
            coord: IVec3::new(-1, 0, 1)
        }
    );
}

#[test]
fn lod_sampling_and_majority_downsampling() {
    let mut schema = Schema::new();
    let density = schema.channel("density", StorageKind::Dense, -127i8);
    let block = schema.channel("block", StorageKind::Paletted, 0u16);
    let mut volume = Volume::<Cube16>::new(schema);
    for x in 0..32 {
        for z in 0..32 {
            for y in 0..10 {
                volume.set(density, IVec3::new(x, y, z), 50);
                volume.set(block, IVec3::new(x, y, z), if x < 16 { 3 } else { 4 });
            }
        }
    }
    let mut d = Snapshot::new();
    let mut m: Snapshot<u16> = Snapshot::new();
    sample_lod(
        &volume,
        density,
        Some(block),
        LodNode {
            level: 1,
            coord: IVec3::ZERO,
        },
        8,
        1,
        &mut d,
        Some(&mut m),
    );
    assert_eq!(d.edge(), 10);
    assert_eq!(d.at(0, 4, 0) as i8, 50, "y = 8 voxels");
    assert_eq!(d.at(0, 5, 0) as i8, -127, "y = 10 voxels");
    assert_eq!(m.at(8, 0, 0), 4, "x = 16 voxels (the +X padding cell)");
    assert_eq!(m.at(7, 0, 0), 3, "x = 14 voxels");
    let mut coarse = Snapshot::new();
    downsample_majority(&volume, block, IVec3::ZERO, 2, 8, 1, 0u16, &mut coarse);
    assert_eq!(coarse.at(0, 0, 0), 3);
    assert_eq!(coarse.at(4, 0, 0), 4);
    assert_eq!(
        coarse.at(0, 2, 0),
        3,
        "y 8..12: half filled (8, 9) keeps material"
    );
    assert_eq!(coarse.at(0, 3, 0), 0, "y 12..16 is empty");
}

#[test]
fn range_allocator_never_overlaps_and_coalesces() {
    let mut alloc = RangeAllocator::new(1 << 20);
    let mut rng = quasar_core::Rng::seed_from_u64(4);
    let mut live: Vec<Allocation> = Vec::new();
    for step in 0..5000 {
        if !live.is_empty() && (rng.chance(0.45) || step > 4000) {
            let i = rng.range_usize(0..live.len());
            let a = live.swap_remove(i);
            if rng.chance(0.5) {
                alloc.free(a);
            } else {
                alloc.free_after(a, step);
            }
            alloc.retire(step.saturating_sub(3));
        } else if let Some(a) =
            alloc.allocate(rng.range_u64(1..4096), [1, 4, 256][rng.range_usize(0..3)])
        {
            live.push(a);
        }
        let mut sorted = live.clone();
        sorted.sort();
        for pair in sorted.windows(2) {
            assert!(
                pair[0].offset + pair[0].size <= pair[1].offset,
                "overlap at step {step}"
            );
        }
    }
    for a in live.drain(..) {
        alloc.free(a);
    }
    alloc.retire(u64::MAX);
    assert_eq!(alloc.used(), 0);
    assert_eq!(alloc.largest_free(), 1 << 20, "fully coalesced");
    let aligned = alloc.allocate(10, 256).unwrap();
    assert_eq!(aligned.offset % 256, 0);
    assert!(alloc.allocate(2 << 20, 1).is_none());
    alloc.grow(4 << 20);
    assert!(alloc.allocate(2 << 20, 1).is_some());
    let mut arena = PagedArena::new(1024);
    let (a, new_page) = arena.allocate(800, 4).unwrap();
    assert!(new_page);
    let (b, second) = arena.allocate(800, 4).unwrap();
    assert!(second && b.page == 1);
    arena.free_after(a, 7);
    assert_eq!(arena.retire(6), 0);
    assert_eq!(arena.retire(7), 1);
    let (c, reused) = arena.allocate(800, 4).unwrap();
    assert!(!reused && c.page == 0);
    assert!(arena.allocate(2000, 1).is_none());
}

#[test]
fn draw_lists_group_and_serialise_commands() {
    let mut list = DrawList::new();
    let key = |x: i32, layer: Layer, stream: u8| DrawKey {
        grid: GridId(1),
        chunk: IVec3::new(x, 0, 0),
        level: 0,
        layer,
        stream,
    };
    let record = |k: DrawKey, page: u32, first: u32, count: u32| DrawRecord {
        key: k,
        page,
        first,
        count,
        base_vertex: 0,
        min: Vec3::ZERO,
        max: Vec3::splat(16.0),
        instance: k.chunk.x as u32,
    };
    list.insert(record(key(0, Layer::Opaque, 0), 0, 0, 10));
    list.insert(record(key(1, Layer::Opaque, 0), 0, 10, 5));
    list.insert(record(key(2, Layer::Opaque, 0), 1, 0, 3));
    list.insert(record(key(0, Layer::Translucent, 0), 0, 15, 2));
    let batches = list.build(|_| true, |stream| if stream == 0 { 6 } else { 1 });
    assert_eq!(batches.len(), 3);
    assert_eq!(batches[0].draws.len(), 2);
    assert_eq!(
        batches[0].draws[1],
        DrawIndirect {
            vertex_count: 30,
            instance_count: 1,
            first_vertex: 60,
            first_instance: 1
        }
    );
    assert_eq!(batches[0].draw_bytes().len(), 32);
    assert_eq!(batches[0].indexed_bytes().len(), 40);
    let culled = list.build(|r| r.key.chunk.x != 1, |_| 6);
    assert_eq!(culled[0].draws.len(), 1);
    assert_eq!(list.remove_chunk(GridId(1), IVec3::ZERO).len(), 2);
    assert_eq!(list.len(), 2);
    let _ = Mat4::IDENTITY;
}
