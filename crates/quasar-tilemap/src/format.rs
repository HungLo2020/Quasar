use crate::{Result, error};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
pub const VERSION: u32 = 1;
pub const MAX_CELLS: usize = 4_194_304;
pub const GID_MASK: u32 = 0x0fff_ffff;
pub const FLIP_X: u32 = 0x8000_0000;
pub const FLIP_Y: u32 = 0x4000_0000;
pub const TRANSPOSE: u32 = 0x2000_0000;
pub type Metadata = BTreeMap<String, Value>;
pub type Properties = Vec<Property>;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Property {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub value: Value,
    #[serde(default)]
    pub metadata: Metadata,
}
pub fn property<'a>(properties: &'a [Property], name: &str) -> Option<&'a Value> {
    properties.iter().find(|p| p.name == name).map(|p| &p.value)
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Map {
    pub quasar_version: u32,
    pub tile_width: u32,
    pub tile_height: u32,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub infinite: bool,
    #[serde(default)]
    pub background: [u8; 4],
    pub tilesets: Vec<Tileset>,
    pub layers: Vec<Layer>,
    #[serde(default)]
    pub properties: Properties,
    #[serde(default)]
    pub metadata: Metadata,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tileset {
    pub first_gid: u32,
    pub name: String,
    pub image: String,
    pub image_width: u32,
    pub image_height: u32,
    pub tile_width: u32,
    pub tile_height: u32,
    pub columns: u32,
    pub tile_count: u32,
    #[serde(default)]
    pub margin: u32,
    #[serde(default)]
    pub spacing: u32,
    #[serde(default)]
    pub tiles: BTreeMap<u32, TileInfo>,
    #[serde(default)]
    pub properties: Properties,
    #[serde(default)]
    pub metadata: Metadata,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct TileInfo {
    #[serde(default)]
    pub animation: Vec<Frame>,
    #[serde(default)]
    pub properties: Properties,
    #[serde(default)]
    pub metadata: Metadata,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub tile_id: u32,
    pub duration_ms: u32,
}
fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.
}
fn parallax() -> [f32; 2] {
    [1., 1.]
}
fn white() -> [u8; 4] {
    [255; 4]
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub id: u32,
    pub name: String,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default = "one")]
    pub opacity: f32,
    #[serde(default)]
    pub offset: [f32; 2],
    #[serde(default = "parallax")]
    pub parallax: [f32; 2],
    #[serde(default = "white")]
    pub tint: [u8; 4],
    pub content: LayerContent,
    #[serde(default)]
    pub properties: Properties,
    #[serde(default)]
    pub metadata: Metadata,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LayerContent {
    Tiles { chunks: Vec<Chunk> },
    Objects { objects: Vec<Object> },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Chunk {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub cells: Vec<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Object {
    pub id: u32,
    pub name: String,
    #[serde(default)]
    pub class: String,
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub width: f64,
    #[serde(default)]
    pub height: f64,
    #[serde(default)]
    pub point: bool,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default)]
    pub properties: Properties,
    #[serde(default)]
    pub metadata: Metadata,
}
impl Map {
    pub fn validate(&self) -> Result<()> {
        if self.quasar_version != VERSION {
            return Err(error(
                "version",
                format!(
                    "unsupported map version {}; supported: {VERSION}",
                    self.quasar_version
                ),
            ));
        }
        if self.tile_width == 0
            || self.tile_height == 0
            || self.tile_width > 1024
            || self.tile_height > 1024
            || (!self.infinite && (self.width == 0 || self.height == 0))
            || self.width > 1_000_000
            || self.height > 1_000_000
            || self.layers.len() > 1024
            || self.tilesets.len() > 1024
        {
            return Err(error(
                "bounds",
                "invalid map dimensions or too many layers/tilesets",
            ));
        }
        validate_properties(&self.properties)?;
        let mut end = 1u32;
        for set in &self.tilesets {
            if set.first_gid < end
                || set.tile_count == 0
                || set
                    .first_gid
                    .checked_add(set.tile_count)
                    .is_none_or(|v| v > GID_MASK)
                || set.columns == 0
                || set.tile_width != self.tile_width
                || set.tile_height != self.tile_height
                || set.image_width > 16384
                || set.image_height > 16384
            {
                return Err(error(
                    "tileset",
                    "tilesets must have sorted nonoverlapping GIDs and uniform map-sized tiles",
                ));
            }
            let columns = grid_count(set.image_width, set.tile_width, set.margin, set.spacing)?;
            let rows = grid_count(set.image_height, set.tile_height, set.margin, set.spacing)?;
            if columns != set.columns || set.tile_count > columns * rows {
                return Err(error(
                    "atlas",
                    "tileset dimensions/columns do not fit its atlas",
                ));
            }
            crate::io::relative("", &set.image)?;
            validate_properties(&set.properties)?;
            for (&id, info) in &set.tiles {
                if id >= set.tile_count
                    || info.animation.len() > 65536
                    || info
                        .animation
                        .iter()
                        .any(|f| f.tile_id >= set.tile_count || f.duration_ms == 0)
                {
                    return Err(error("animation", "invalid tile or animation frame"));
                }
                validate_properties(&info.properties)?;
            }
            end = set.first_gid + set.tile_count;
        }
        let mut ids = BTreeSet::new();
        let mut objects = BTreeSet::new();
        let mut total = 0usize;
        for layer in &self.layers {
            if layer.id == 0
                || !ids.insert(layer.id)
                || !layer.opacity.is_finite()
                || !(0.0..=1.0).contains(&layer.opacity)
                || layer
                    .offset
                    .iter()
                    .chain(&layer.parallax)
                    .any(|v| !v.is_finite() || v.abs() > 16_000_000.)
            {
                return Err(error(
                    "layer",
                    "invalid layer identity, opacity or transform",
                ));
            }
            validate_properties(&layer.properties)?;
            match &layer.content {
                LayerContent::Tiles { chunks } => {
                    if chunks.len() > 65536 {
                        return Err(error("bounds", "too many chunks"));
                    }
                    let mut rows: BTreeMap<i32, Vec<(i32, i32)>> = BTreeMap::new();
                    for c in chunks {
                        let count = (c.width as usize)
                            .checked_mul(c.height as usize)
                            .ok_or_else(|| error("bounds", "chunk overflow"))?;
                        if c.width == 0
                            || c.height == 0
                            || c.width > 65536
                            || c.height > 65536
                            || count != c.cells.len()
                            || c.x.abs_diff(0) > 1_000_000
                            || c.y.abs_diff(0) > 1_000_000
                        {
                            return Err(error("chunk", "invalid chunk bounds or cell count"));
                        }
                        let right =
                            c.x.checked_add(c.width as i32)
                                .ok_or_else(|| error("bounds", "chunk X overflow"))?;
                        let bottom =
                            c.y.checked_add(c.height as i32)
                                .ok_or_else(|| error("bounds", "chunk Y overflow"))?;
                        if !self.infinite
                            && (c.x < 0
                                || c.y < 0
                                || right > self.width as i32
                                || bottom > self.height as i32)
                        {
                            return Err(error("chunk", "finite chunk exceeds map dimensions"));
                        }
                        total = total
                            .checked_add(count)
                            .ok_or_else(|| error("bounds", "cell count overflow"))?;
                        if total > MAX_CELLS {
                            return Err(error("bounds", "map exceeds 4194304 stored cells"));
                        }
                        for &gid in &c.cells {
                            self.resolve(gid)?;
                            if gid & TRANSPOSE != 0 && self.tile_width != self.tile_height {
                                return Err(error(
                                    "unsupported",
                                    "diagonal tile transforms require square tiles",
                                ));
                            }
                        }
                        for y in c.y..bottom {
                            rows.entry(y).or_default().push((c.x, right));
                        }
                    }
                    for spans in rows.values_mut() {
                        spans.sort_unstable();
                        if spans.windows(2).any(|w| w[0].1 > w[1].0) {
                            return Err(error("chunk", "overlapping chunks in a layer"));
                        }
                    }
                }
                LayerContent::Objects { objects: items } => {
                    if items.len() > 65536 {
                        return Err(error("bounds", "too many objects"));
                    }
                    for o in items {
                        if o.id == 0
                            || !objects.insert(o.id)
                            || [o.x, o.y, o.width, o.height]
                                .iter()
                                .any(|v| !v.is_finite() || v.abs() > 1e9)
                            || o.width < 0.
                            || o.height < 0.
                        {
                            return Err(error("object", "invalid object identity or bounds"));
                        }
                        validate_properties(&o.properties)?;
                    }
                }
            }
        }
        Ok(())
    }
    pub fn resolve(&self, raw: u32) -> Result<Option<(usize, u32)>> {
        let gid = raw & GID_MASK;
        if gid == 0 {
            return Ok(None);
        }
        let index = self
            .tilesets
            .partition_point(|s| s.first_gid <= gid)
            .checked_sub(1)
            .ok_or_else(|| error("gid", format!("unknown tile GID {gid}")))?;
        let local = gid
            .checked_sub(self.tilesets[index].first_gid)
            .ok_or_else(|| error("tileset", "tilesets are not sorted"))?;
        if local >= self.tilesets[index].tile_count {
            return Err(error("gid", format!("unknown tile GID {gid}")));
        }
        Ok(Some((index, local)))
    }
    pub fn layer(&self, id: u32) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }
    /// Edits a stored cell or adds a sparse one-cell chunk. Rebuild Runtime after edits.
    pub fn set_tile(&mut self, layer: u32, x: i32, y: i32, gid: u32) -> Result<()> {
        self.validate()?;
        self.resolve(gid)?;
        let mut candidate = self.clone();
        let layer = candidate
            .layers
            .iter_mut()
            .find(|l| l.id == layer)
            .ok_or_else(|| error("layer", "unknown layer"))?;
        let LayerContent::Tiles { chunks } = &mut layer.content else {
            return Err(error("layer", "layer is not a tile layer"));
        };
        let mut changed = false;
        for c in chunks.iter_mut() {
            let dx = i64::from(x) - i64::from(c.x);
            let dy = i64::from(y) - i64::from(c.y);
            if dx >= 0 && dy >= 0 && dx < i64::from(c.width) && dy < i64::from(c.height) {
                c.cells[(dy as u32 * c.width + dx as u32) as usize] = gid;
                changed = true;
                break;
            }
        }
        if !changed {
            chunks.push(Chunk {
                x,
                y,
                width: 1,
                height: 1,
                cells: vec![gid],
            });
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    /// Declared image and typed file-property paths, sorted and deduplicated.
    pub fn asset_paths(&self) -> Vec<String> {
        let mut assets = BTreeSet::new();
        for set in &self.tilesets {
            assets.insert(set.image.clone());
        }
        let mut add = |properties: &Properties| {
            for p in properties {
                if p.kind == "file"
                    && let Some(path) = p.value.as_str()
                    && !path.is_empty()
                {
                    assets.insert(path.to_owned());
                }
            }
        };
        add(&self.properties);
        for set in &self.tilesets {
            add(&set.properties);
            for tile in set.tiles.values() {
                add(&tile.properties);
            }
        }
        for layer in &self.layers {
            add(&layer.properties);
            if let LayerContent::Objects { objects } = &layer.content {
                for object in objects {
                    add(&object.properties);
                }
            }
        }
        assets.into_iter().collect()
    }
    pub fn canonical_json(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut bytes = serde_json::to_vec_pretty(self).map_err(crate::json_error)?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}
fn grid_count(image: u32, tile: u32, margin: u32, spacing: u32) -> Result<u32> {
    let usable = image
        .checked_sub(
            margin
                .checked_mul(2)
                .ok_or_else(|| error("atlas", "margin overflow"))?,
        )
        .and_then(|n| n.checked_add(spacing))
        .ok_or_else(|| error("atlas", "atlas bounds overflow"))?;
    let stride = tile
        .checked_add(spacing)
        .ok_or_else(|| error("atlas", "spacing overflow"))?;
    if usable == 0 || !usable.is_multiple_of(stride) {
        return Err(error("atlas", "atlas contains partial cells"));
    }
    Ok(usable / stride)
}
pub(crate) fn validate_properties(props: &[Property]) -> Result<()> {
    let mut names = BTreeSet::new();
    for p in props {
        if p.name.is_empty() || !names.insert(&p.name) {
            return Err(error("property", "empty or duplicate property name"));
        }
        let valid = match p.kind.as_str() {
            "bool" => p.value.is_boolean(),
            "int" | "object" => p.value.as_i64().is_some(),
            "float" => p.value.is_number(),
            "string" | "file" | "color" => p.value.is_string(),
            _ => true,
        };
        if !valid {
            return Err(error(
                "property",
                format!("invalid value for {} property {}", p.kind, p.name),
            ));
        }
        if p.kind == "file" && !p.value.as_str().unwrap().is_empty() {
            crate::io::relative("", p.value.as_str().unwrap())?;
        }
    }
    Ok(())
}
