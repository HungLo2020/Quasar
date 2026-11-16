#![cfg(feature = "jobs")]
use quasar_math::IVec3;
use quasar_voxel::*;
use std::sync::Arc;

struct Hills;
impl VoxelSource<Cube16> for Hills {
    fn generate(&self, schema: &Schema, coord: IVec3, _lod: u32, chunk: &mut Chunk<Cube16>) {
        let block = schema.find::<u16>("block").unwrap();
        let values: Vec<u16> = (0..Cube16::VOLUME)
            .map(|i| {
                let p = Cube16::origin(coord) + Cube16::local(i).as_ivec3();
                u16::from(p.y < (p.x / 3 + p.z / 5).rem_euclid(9))
            })
            .collect();
        chunk.write_all(schema, block, &values);
    }
}

#[test]
fn parallel_generation_matches_serial_generation() {
    let mut schema = Schema::new();
    let block = schema.channel("block", StorageKind::Paletted, 0u16);
    let schema = Arc::new(schema);
    let coords: Vec<IVec3> = (0..4)
        .flat_map(|x| (0..4).map(move |z| IVec3::new(x, 0, z)))
        .collect();
    let mut serial = Volume::<Cube16>::with_schema(schema.clone()).with_source(Arc::new(Hills));
    for c in &coords {
        serial.ensure_chunk(*c);
    }
    let pool = quasar_jobs::JobPool::new(quasar_jobs::JobPoolConfig::new(4, 64)).unwrap();
    let mut parallel = Volume::<Cube16>::with_schema(schema.clone()).with_source(Arc::new(Hills));
    assert_eq!(parallel.ensure_chunks_parallel(&pool, &coords).unwrap(), 16);
    assert_eq!(
        parallel.ensure_chunks_parallel(&pool, &coords).unwrap(),
        0,
        "already loaded"
    );
    for c in &coords {
        let a = codec::encode_chunk(&schema, serial.chunk(*c).unwrap(), true);
        let b = codec::encode_chunk(&schema, parallel.chunk(*c).unwrap(), true);
        assert_eq!(a, b, "chunk {c}");
    }
    assert!(
        parallel.edited_chunks().is_empty(),
        "generated chunks are not deltas"
    );
    let _ = block;
}
