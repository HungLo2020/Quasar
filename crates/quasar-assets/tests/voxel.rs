#![cfg(feature = "voxel")]
use quasar_assets::voxel::*;
use quasar_assets::{AssetServer, MemorySource};
use quasar_math::IVec3;
use quasar_voxel::{Cube16, Schema, StorageKind, Volume};
use std::sync::Arc;

#[test]
fn catalogs_and_chunk_deltas_load_through_the_asset_server() {
    let defs = vec![
        quasar_voxel::BlockDef::cube("stone", 1),
        quasar_voxel::BlockDef::cube("glass", 2).with_layer(quasar_voxel::Layer::Translucent),
    ];
    let mut schema = Schema::new();
    let block = schema.channel("block", StorageKind::Paletted, 0u16);
    let schema = Arc::new(schema);
    let mut volume = Volume::<Cube16>::with_schema(schema.clone());
    volume.set(block, IVec3::new(3, 4, 5), 2);
    let bytes =
        quasar_voxel::codec::encode_chunk(&schema, volume.chunk(IVec3::ZERO).unwrap(), false);
    let mut source = MemorySource::new("voxels");
    source
        .insert("blocks.json", serde_json::to_vec(&defs).unwrap())
        .unwrap();
    source.insert("chunks/0_0_0.qvc", bytes).unwrap();
    source.insert("bad.json", b"{".to_vec()).unwrap();
    let mut server = AssetServer::new();
    server.mount("", Arc::new(source)).unwrap();
    server.register(CatalogLoader);
    server.register(ChunkLoader::<Cube16>::new(schema.clone()));
    let catalog = server.load::<CatalogLoader>("blocks.json").unwrap();
    assert_eq!(catalog.0.find("glass"), Some(2));
    assert_eq!(
        catalog.0.get(2).unwrap().layer,
        quasar_voxel::Layer::Translucent
    );
    let chunk = server
        .load::<ChunkLoader<Cube16>>("chunks/0_0_0.qvc")
        .unwrap();
    assert_eq!(chunk.0.get(block, quasar_math::UVec3::new(3, 4, 5)), 2);
    assert!(chunk.0.is_edited());
    assert!(server.load::<CatalogLoader>("bad.json").is_err());
}
