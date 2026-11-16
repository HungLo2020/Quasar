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
    let reference = world.reference_light();
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

/// Asserts both light channels equal a from-scratch recompute everywhere.
fn assert_light_is_exact(world: &BlockWorld, context: &str) {
    let reference = world.reference_light();
    for (cx, cz) in world.columns() {
        for y in 0..voxel_blocks::terrain::WORLD_TOP {
            for z in 0..16 {
                for x in 0..16 {
                    let p = IVec3::new(cx * 16 + x, y, cz * 16 + z);
                    for (name, channel) in
                        [("sky", world.ch.sky_light), ("block", world.ch.block_light)]
                    {
                        assert_eq!(
                            world.volume.get(channel, p),
                            reference.get(channel, p),
                            "{context}: {name} light at {p}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn player_edit_sequences_keep_light_exact() {
    // Seeded, player-like edits across chunk borders (x and z = 16): dig and
    // tunnel, build with opaque, translucent, filtering, emitting and partial
    // blocks, seal pockets and reopen them. Both light channels must equal a
    // full recompute at every checkpoint.
    use quasar_voxel::{AIR, EditBatch, Orientation};
    let top = voxel_blocks::terrain::WORLD_TOP;
    let mut world = BlockWorld::new(5, 1, 1);
    world.sync_meshing = true;
    world.settle();
    let b = world.content.blocks;
    let palette = [b.stone, b.glass, b.leaves, b.lamp, b.slab, b.planks, b.lamp];
    let surface = |world: &BlockWorld, x: i32, z: i32| {
        (0..top)
            .rev()
            .find(|&y| world.volume.get(world.ch.block, IVec3::new(x, y, z)) != AIR)
            .unwrap_or(0)
    };
    let mut rng = quasar_core::Rng::seed_from_u64(9);
    let mut dug: Vec<IVec3> = Vec::new();
    let (mut removed, mut placed) = (0, 0);
    for step in 0..96 {
        let x = rng.range_i32(10..22);
        let z = rng.range_i32(10..22);
        let s = surface(&world, x, z);
        let mut batch = EditBatch::new();
        let catalog = &world.content.catalog;
        let ch = world.ch.blocks();
        let edit = match rng.range_u32(0..6) {
            // Dig down, or tunnel sideways under the surface.
            0 | 1 => Some((IVec3::new(x, s, z), None)),
            2 => Some((IVec3::new(x, (s - 2).max(1), z), None)),
            // Build on the surface.
            3 | 4 => Some((
                IVec3::new(x, s + 1, z),
                Some(palette[rng.range_u32(0..palette.len() as u32) as usize]),
            )),
            // Seal or refill an earlier hole (lamps light sealed pockets).
            _ => dug.pop().map(|p| (p, Some(palette[step % palette.len()]))),
        };
        let Some((cell, block)) = edit else {
            continue;
        };
        let applied = match block {
            None => catalog
                .remove(&mut batch, &world.volume, ch, cell)
                .is_some(),
            Some(id) => catalog
                .place(
                    &mut batch,
                    &world.volume,
                    ch,
                    id,
                    Orientation::IDENTITY,
                    cell,
                )
                .is_ok(),
        };
        if applied {
            world.apply(&batch);
            match block {
                None => {
                    dug.push(cell);
                    removed += 1;
                }
                Some(_) => placed += 1,
            }
        }
        if step % 32 == 31 {
            assert_light_is_exact(&world, &format!("after step {step}"));
        }
    }
    assert!(
        removed > 20 && placed > 20,
        "removed={removed} placed={placed}"
    );
}
