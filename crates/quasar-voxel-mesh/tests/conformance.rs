//! Recorded extractions replay byte-for-byte; policy changes are reported.
use quasar_math::IVec3;
use quasar_voxel::conformance::{Mismatch, Recording, record_snapshot, snapshot_from_bytes};
use quasar_voxel::{
    AIR, BlockCatalog, BlockDef, Cube16, Face, Schema, Snapshot, StorageKind, Volume,
};
use quasar_voxel_mesh::*;

fn run(blocks: &Snapshot<u16>, catalog: &BlockCatalog, order: IterationOrder) -> MeshBuffers {
    let library = ModelLibrary::new();
    let opaque = opaque_table(catalog);
    let mut out = MeshBuffers::new();
    CulledModelExtractor::new(order).extract(
        CulledInput {
            blocks,
            orientation: None,
            block_light: None,
            sky_light: None,
            opaque: &opaque,
        },
        &CatalogModels {
            catalog,
            library: &library,
        },
        &CatalogFaces(catalog),
        &VertexShade::default(),
        &CatalogLayers(catalog),
        |_| true,
        &mut out,
    );
    out
}

#[test]
fn recorded_extractions_replay_identically_and_policy_changes_are_caught() {
    let mut schema = Schema::new();
    let block = schema.channel("block", StorageKind::Paletted, AIR);
    let mut volume = Volume::<Cube16>::new(schema);
    let mut catalog = BlockCatalog::new();
    let stone = catalog.add(BlockDef::cube("stone", 1));
    let mut rng = quasar_core::Rng::seed_from_u64(12);
    for _ in 0..600 {
        volume.set(
            block,
            IVec3::new(
                rng.range_i32(0..16),
                rng.range_i32(0..16),
                rng.range_i32(0..16),
            ),
            stone,
        );
    }
    let mut snap = Snapshot::new();
    volume.snapshot(block, IVec3::ZERO, 1, &mut snap);
    let mut reference = Recording::new("culled-model");
    reference.push("input.blocks", record_snapshot(&snap)).push(
        "output",
        record_mesh(&run(&snap, &catalog, IterationOrder::default())),
    );
    let stored = reference.encode();
    // Replay from the recorded input only.
    let loaded = Recording::decode(&stored).unwrap();
    let input: Snapshot<u16> =
        snapshot_from_bytes(loaded.section("input.blocks").unwrap()).unwrap();
    let mut replay = Recording::new("culled-model");
    replay.push("input.blocks", record_snapshot(&input)).push(
        "output",
        record_mesh(&run(&input, &catalog, IterationOrder::default())),
    );
    assert_eq!(replay.compare(&loaded), None, "byte-identical replay");
    // A different iteration policy is detected at the first differing byte.
    let mut other = Recording::new("culled-model");
    other.push("input.blocks", record_snapshot(&input)).push(
        "output",
        record_mesh(&run(&input, &catalog, IterationOrder([0, 1, 2]))),
    );
    match other.compare(&loaded) {
        Some(Mismatch::Bytes {
            section, offset, ..
        }) => {
            assert_eq!(section, "output");
            assert!(offset > 0);
        }
        other => panic!("expected a byte mismatch, got {other:?}"),
    }
    let _ = Face::PosY;
}
