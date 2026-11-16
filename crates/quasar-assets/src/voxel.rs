//! Voxel asset loaders (feature `voxel`): block catalogs from JSON and chunk
//! deltas in the deterministic `quasar-voxel` codec.
use crate::{Asset, AssetError, AssetErrorKind, AssetLoader, LoadContext};
use quasar_voxel::{BlockCatalog, BlockDef, Chunk, ChunkLayout, Schema};
use std::marker::PhantomData;
use std::sync::Arc;

/// A loaded block catalog.
#[derive(Clone, Debug)]
pub struct CatalogAsset(pub BlockCatalog);

impl Asset for CatalogAsset {
    fn memory_size(&self) -> usize {
        self.0.len() * std::mem::size_of::<BlockDef>()
    }
}

/// Loads a JSON array of [`BlockDef`]s (air is added as block 0, so the
/// first definition becomes block 1).
#[derive(Clone, Copy, Debug, Default)]
pub struct CatalogLoader;

impl AssetLoader for CatalogLoader {
    type Asset = CatalogAsset;
    fn max_source_bytes(&self) -> usize {
        16 << 20
    }
    fn load(&self, context: &LoadContext<'_>) -> Result<CatalogAsset, AssetError> {
        let defs: Vec<BlockDef> = serde_json::from_slice(context.bytes())
            .map_err(|e| AssetError::new(AssetErrorKind::Loader, format!("block catalog: {e}")))?;
        let mut catalog = BlockCatalog::new();
        for def in defs {
            if !def.footprint.contains(&quasar_math::IVec3::ZERO) {
                return Err(AssetError::new(
                    AssetErrorKind::Loader,
                    format!("block {:?}: footprint must include the anchor", def.name),
                ));
            }
            catalog.add(def);
        }
        Ok(CatalogAsset(catalog))
    }
}

/// A decoded chunk delta.
#[derive(Clone, Debug)]
pub struct ChunkAsset<L: ChunkLayout>(pub Chunk<L>);

impl<L: ChunkLayout> Asset for ChunkAsset<L> {
    fn memory_size(&self) -> usize {
        self.0.memory_bytes()
    }
}

/// Loads `quasar_voxel::codec::encode_chunk` bytes for a fixed schema.
#[derive(Clone, Debug)]
pub struct ChunkLoader<L: ChunkLayout> {
    pub schema: Arc<Schema>,
    _layout: PhantomData<L>,
}

impl<L: ChunkLayout> ChunkLoader<L> {
    pub fn new(schema: Arc<Schema>) -> Self {
        Self {
            schema,
            _layout: PhantomData,
        }
    }
}

impl<L: ChunkLayout> AssetLoader for ChunkLoader<L> {
    type Asset = ChunkAsset<L>;
    fn load(&self, context: &LoadContext<'_>) -> Result<ChunkAsset<L>, AssetError> {
        quasar_voxel::codec::decode_chunk::<L>(&self.schema, context.bytes())
            .map(ChunkAsset)
            .map_err(|e| AssetError::new(AssetErrorKind::Loader, e.to_string()))
    }
}
