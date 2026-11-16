use quasar_math::IVec3;
use quasar_voxel::Snapshot;
use voxel_blocks::world::BlockWorld;

#[test]
fn snapshots_match_volume_reads_on_generated_terrain() {
    let mut world = BlockWorld::new(7, 2, 2);
    world.sync_meshing = true;
    world.settle();
    let mut snap = Snapshot::new();
    for coord in [
        IVec3::new(0, 1, 0),
        IVec3::new(0, 0, 0),
        IVec3::new(-1, 1, 1),
    ] {
        world.volume.snapshot(world.ch.block, coord, 1, &mut snap);
        for y in -1..17 {
            for z in -1..17 {
                for x in -1..17 {
                    let world_cell = coord * 16 + IVec3::new(x, y, z);
                    assert_eq!(
                        snap.at(x, y, z),
                        world.volume.get(world.ch.block, world_cell),
                        "{coord} {x},{y},{z}"
                    );
                }
            }
        }
    }
}

#[test]
fn chunk_meshes_contain_the_surface_and_pack_losslessly() {
    use quasar_math::Vec3;
    use quasar_voxel::{AIR, Face};
    use quasar_voxel_mesh::*;
    let mut world = BlockWorld::new(0xb10c, 2, 2);
    world.sync_meshing = true;
    world.settle();
    let coord = IVec3::new(0, 1, 0);
    let mut blocks = Snapshot::new();
    world.volume.snapshot(world.ch.block, coord, 1, &mut blocks);
    let content = &world.content;
    let mut out = MeshBuffers::new();
    CulledModelExtractor::default().extract(
        CulledInput {
            blocks: &blocks,
            orientation: None,
            block_light: None,
            sky_light: None,
            opaque: &content.opaque,
        },
        &CatalogModels {
            catalog: &content.catalog,
            library: &content.library,
        },
        &CatalogFaces(&content.catalog),
        &VertexShade::default(),
        &CatalogLayers(&content.catalog),
        |_| true,
        &mut out,
    );
    // Every exposed top face of a surface column appears exactly once.
    let mut tops = 0;
    for z in 0..16 {
        for x in 0..16 {
            for y in 0..16 {
                let cell = coord * 16 + IVec3::new(x, y, z);
                let b = world.volume.get(world.ch.block, cell);
                if b != AIR
                    && content.opaque[b as usize]
                    && world.volume.get(world.ch.block, cell + IVec3::Y) == AIR
                {
                    let top = Vec3::new(x as f32, y as f32 + 1.0, z as f32);
                    let found = out.layers.iter().flat_map(|(_, l)| &l.quads).any(|q| {
                        q.face == Some(Face::PosY)
                            && q.positions.iter().all(|p| (p.y - top.y).abs() < 1e-5)
                            && q.positions.iter().any(|p| (*p - top).length() < 1e-5)
                    });
                    assert!(found, "missing top face at {cell}");
                    tops += 1;
                }
            }
        }
    }
    assert!(tops > 0);
    for (_, layer) in &out.layers {
        for q in &layer.quads {
            if let Ok(p) = PackedQuad::encode(q) {
                let d = p.decode();
                for c in q.positions {
                    assert!(
                        d.positions.iter().any(|p| (*p - c).length() < 1e-5),
                        "{q:?} -> {d:?}"
                    );
                }
            }
        }
    }
}

fn run(seed: u64, ticks: u64, workers: usize) -> BlockWorld {
    let mut world = BlockWorld::new(seed, 3, workers);
    world.sync_meshing = true;
    world.settle();
    for _ in 0..ticks {
        let input = voxel_blocks::world::autoplay_input(&world);
        world.update(input);
    }
    world
}

#[test]
fn autoplay_is_deterministic_across_thread_counts_and_edits_the_world() {
    let a = run(3, 330, 1);
    let b = run(3, 330, 4);
    assert_eq!(a.state_hash(), b.state_hash());
    assert!(a.stats.placed > 0 && a.stats.removed > 0, "{:?}", a.stats);
    assert!(!a.volume.edited_chunks().is_empty());
    assert!(a.stats.meshed > 0 && a.stats.quads > 0);
}

#[test]
fn lamps_light_their_surroundings_and_removal_restores_darkness() {
    use quasar_voxel::{EditBatch, Orientation};
    let mut world = BlockWorld::new(11, 2, 2);
    world.sync_meshing = true;
    world.settle();
    // Carve a sealed pocket deep underground and light it.
    let centre = IVec3::new(8, 6, 8);
    let mut batch = EditBatch::new();
    batch.fill(
        world.ch.block,
        centre - IVec3::splat(2),
        centre + IVec3::splat(2),
        world.content.blocks.stone,
    );
    batch.fill(
        world.ch.block,
        centre - IVec3::ONE,
        centre + IVec3::ONE,
        quasar_voxel::AIR,
    );
    world.apply(&batch);
    assert_eq!(
        world.volume.get(world.ch.sky_light, centre),
        0,
        "sealed from the sky"
    );
    assert_eq!(world.volume.get(world.ch.block_light, centre), 0);
    let lamp = world.content.blocks.lamp;
    let mut place = EditBatch::new();
    world
        .content
        .catalog
        .place(
            &mut place,
            &world.volume,
            world.ch.blocks(),
            lamp,
            Orientation::IDENTITY,
            centre,
        )
        .unwrap();
    world.apply(&place);
    assert_eq!(
        world.volume.get(world.ch.block_light, centre + IVec3::X),
        13
    );
    assert_eq!(
        world
            .volume
            .get(world.ch.block_light, centre + IVec3::new(1, 1, 1)),
        11
    );
    let mut remove = EditBatch::new();
    world
        .content
        .catalog
        .remove(&mut remove, &world.volume, world.ch.blocks(), centre)
        .unwrap();
    world.apply(&remove);
    assert_eq!(world.volume.get(world.ch.block_light, centre + IVec3::X), 0);
}

#[test]
fn incremental_column_lighting_matches_a_full_recompute() {
    let mut world = BlockWorld::new(21, 3, 2);
    world.sync_meshing = true;
    world.settle();
    let reference = world.reference_sky_light();
    let mut checked = 0;
    for (cx, cz) in world.columns() {
        for y in 0..voxel_blocks::terrain::WORLD_TOP {
            for z in 0..16 {
                for x in 0..16 {
                    let p = IVec3::new(cx * 16 + x, y, cz * 16 + z);
                    assert_eq!(
                        world.volume.get(world.ch.sky_light, p),
                        reference.get(world.ch.sky_light, p),
                        "{p}"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 100_000);
}
