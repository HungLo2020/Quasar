use crate::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
};
const MAX_JSON: usize = 64 * 1024 * 1024;
/// All declared files resolve inside this canonical root, including symlinks.
#[derive(Debug, Clone)]
pub struct AssetRoot {
    root: PathBuf,
}
impl AssetRoot {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let root = std::fs::canonicalize(path).map_err(|e| error("path", e.to_string()))?;
        if !root.is_dir() {
            return Err(error("path", "asset root is not a directory"));
        }
        Ok(Self { root })
    }
    pub fn resolve(&self, path: &str) -> Result<PathBuf> {
        let rel = relative("", path)?;
        let resolved = std::fs::canonicalize(self.root.join(rel))
            .map_err(|e| error("asset", format!("{path}: {e}")))?;
        if !resolved.starts_with(&self.root) || !resolved.is_file() {
            return Err(error(
                "path",
                format!("asset escapes root or is not a file: {path}"),
            ));
        }
        Ok(resolved)
    }
    pub fn path(&self) -> &Path {
        &self.root
    }
}
pub(crate) fn relative(base: &str, path: &str) -> Result<String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.contains('\0')
    {
        return Err(error(
            "path",
            format!("expected portable relative asset path: {path}"),
        ));
    }
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(error("path", "asset path escapes root"));
                }
            }
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        return Err(error("path", "asset path names the root"));
    }
    Ok(parts.join("/"))
}
fn read_json(path: &Path, budget: &mut usize) -> Result<Value> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| error("io", format!("{}: {e}", path.display())))?
        .take(*budget as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| error("io", e.to_string()))?;
    if bytes.len() > *budget {
        return Err(error(
            "bounds",
            "JSON inputs exceed the aggregate 64 MiB limit",
        ));
    }
    *budget -= bytes.len();
    serde_json::from_slice(&bytes).map_err(json_error)
}
pub fn load(path: impl AsRef<Path>) -> Result<(Map, AssetRoot)> {
    let path = std::fs::canonicalize(path).map_err(|e| error("io", e.to_string()))?;
    let parent = path
        .parent()
        .ok_or_else(|| error("path", "map path has no parent directory"))?;
    let root = AssetRoot::new(parent)?;
    let mut budget = MAX_JSON;
    let value = read_json(&path, &mut budget)?;
    let map = decode(value, &root, &mut budget)?;
    Ok((map, root))
}
pub fn parse(bytes: &[u8], root: &AssetRoot) -> Result<Map> {
    if bytes.len() > MAX_JSON {
        return Err(error("bounds", "JSON exceeds 64 MiB limit"));
    }
    decode(
        serde_json::from_slice(bytes).map_err(json_error)?,
        root,
        &mut (MAX_JSON - bytes.len()),
    )
}
fn decode(value: Value, root: &AssetRoot, budget: &mut usize) -> Result<Map> {
    let mut map: Map = if let Some(version) = value.get("quasar_version") {
        if version.as_u64() != Some(VERSION as u64) {
            return Err(error(
                "version",
                format!("unsupported map version {version}"),
            ));
        }
        serde_json::from_value(value).map_err(json_error)?
    } else {
        from_tiled(value, root, budget)?
    };
    for set in &mut map.tilesets {
        set.image = relative("", &set.image)?;
    }
    map.validate()?;
    Ok(map)
}
fn default_order() -> String {
    "right-down".into()
}
fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.
}
#[derive(Deserialize)]
struct RawMap {
    width: u32,
    height: u32,
    tilewidth: u32,
    tileheight: u32,
    orientation: String,
    #[serde(default = "default_order")]
    renderorder: String,
    #[serde(default)]
    infinite: bool,
    #[serde(default)]
    backgroundcolor: Option<String>,
    layers: Vec<RawLayer>,
    tilesets: Vec<Value>,
    #[serde(default)]
    properties: Vec<RawProperty>,
    #[serde(flatten)]
    extra: Metadata,
}
#[derive(Deserialize)]
struct RawLayer {
    id: u32,
    name: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default = "yes")]
    visible: bool,
    #[serde(default = "one")]
    opacity: f32,
    #[serde(default)]
    offsetx: f32,
    #[serde(default)]
    offsety: f32,
    #[serde(default = "one")]
    parallaxx: f32,
    #[serde(default = "one")]
    parallaxy: f32,
    #[serde(default)]
    tintcolor: Option<String>,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
    #[serde(default)]
    x: i32,
    #[serde(default)]
    y: i32,
    #[serde(default)]
    data: Option<Value>,
    #[serde(default)]
    chunks: Option<Vec<RawChunk>>,
    #[serde(default)]
    objects: Vec<RawObject>,
    #[serde(default)]
    properties: Vec<RawProperty>,
    #[serde(flatten)]
    extra: Metadata,
}
#[derive(Deserialize)]
struct RawChunk {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    data: Vec<u32>,
}
#[derive(Deserialize)]
struct RawSet {
    name: String,
    image: String,
    imagewidth: u32,
    imageheight: u32,
    tilewidth: u32,
    tileheight: u32,
    columns: u32,
    tilecount: u32,
    #[serde(default)]
    margin: u32,
    #[serde(default)]
    spacing: u32,
    #[serde(default)]
    tiles: Vec<RawTile>,
    #[serde(default)]
    properties: Vec<RawProperty>,
    #[serde(flatten)]
    extra: Metadata,
}
#[derive(Deserialize)]
struct RawTile {
    id: u32,
    #[serde(default)]
    animation: Vec<RawFrame>,
    #[serde(default)]
    properties: Vec<RawProperty>,
    #[serde(flatten)]
    extra: Metadata,
}
#[derive(Deserialize)]
struct RawFrame {
    tileid: u32,
    duration: u32,
}
#[derive(Deserialize)]
struct RawProperty {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    value: Value,
    #[serde(flatten)]
    extra: Metadata,
}
#[derive(Deserialize)]
struct RawObject {
    id: u32,
    #[serde(default)]
    name: String,
    #[serde(default)]
    class: String,
    #[serde(rename = "type", default)]
    kind: String,
    x: f64,
    y: f64,
    #[serde(default)]
    width: f64,
    #[serde(default)]
    height: f64,
    #[serde(default)]
    rotation: f32,
    #[serde(default)]
    point: bool,
    #[serde(default = "yes")]
    visible: bool,
    #[serde(default)]
    properties: Vec<RawProperty>,
    #[serde(flatten)]
    extra: Metadata,
}
fn props(raw: Vec<RawProperty>, base: &str) -> Result<Properties> {
    raw.into_iter()
        .map(|p| {
            let mut value = p.value;
            if p.kind == "file"
                && let Some(path) = value.as_str()
                && !path.is_empty()
            {
                value = Value::String(relative(base, path)?);
            }
            Ok(Property {
                name: p.name,
                kind: p.kind,
                value,
                metadata: p.extra,
            })
        })
        .collect()
}
fn unsupported(extra: &Metadata, keys: &[&str]) -> Result<()> {
    if let Some(key) = keys.iter().find(|k| extra.contains_key(**k)) {
        return Err(error(
            "unsupported",
            format!("unsupported Tiled field {key}"),
        ));
    }
    Ok(())
}
fn color(text: &str) -> Result<[u8; 4]> {
    let s = text
        .strip_prefix('#')
        .ok_or_else(|| error("color", "expected #RRGGBB or #AARRGGBB"))?;
    let number = u32::from_str_radix(s, 16).map_err(|_| error("color", "invalid hex color"))?;
    match s.len() {
        6 => Ok([(number >> 16) as u8, (number >> 8) as u8, number as u8, 255]),
        8 => Ok([
            (number >> 16) as u8,
            (number >> 8) as u8,
            number as u8,
            (number >> 24) as u8,
        ]),
        _ => Err(error("color", "expected #RRGGBB or #AARRGGBB")),
    }
}
fn from_tiled(value: Value, root: &AssetRoot, budget: &mut usize) -> Result<Map> {
    let raw: RawMap = serde_json::from_value(value).map_err(json_error)?;
    if raw
        .extra
        .get("type")
        .is_some_and(|v| v.as_str() != Some("map"))
    {
        return Err(error("unsupported", "expected Tiled map document"));
    }
    if raw.orientation != "orthogonal" || raw.renderorder != "right-down" {
        return Err(error(
            "unsupported",
            "only orthogonal right-down Tiled JSON is supported",
        ));
    }
    for key in ["parallaxoriginx", "parallaxoriginy"] {
        if raw.extra.get(key).is_some_and(|v| v.as_f64() != Some(0.)) {
            return Err(error(
                "unsupported",
                "nonzero parallax origins are unsupported",
            ));
        }
    }
    let mut tilesets = Vec::new();
    for value in raw.tilesets {
        let first_gid = value
            .get("firstgid")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| error("tileset", "missing firstgid"))?;
        let (mut definition, base) = if let Some(source) = value.get("source") {
            let source = relative(
                "",
                source
                    .as_str()
                    .ok_or_else(|| error("path", "tileset source must be a string"))?,
            )?;
            let base = source
                .rsplit_once('/')
                .map(|v| v.0)
                .unwrap_or("")
                .to_owned();
            (read_json(&root.resolve(&source)?, budget)?, base)
        } else {
            (value, String::new())
        };
        definition
            .as_object_mut()
            .ok_or_else(|| error("tileset", "invalid tileset"))?
            .remove("firstgid");
        let mut set: RawSet = serde_json::from_value(definition).map_err(json_error)?;
        unsupported(&set.extra, &["transparentcolor"])?;
        if set.extra.get("tileoffset").is_some_and(|v| {
            v.get("x").and_then(Value::as_i64) != Some(0)
                || v.get("y").and_then(Value::as_i64) != Some(0)
        }) {
            return Err(error("unsupported", "tileset offsets are unsupported"));
        }
        if set
            .extra
            .get("objectalignment")
            .is_some_and(|v| v != "unspecified" && v != "bottomleft")
        {
            return Err(error(
                "unsupported",
                "non-default object alignment is unsupported",
            ));
        }
        let mut tiles = BTreeMap::new();
        for tile in set.tiles {
            unsupported(&tile.extra, &["image", "objectgroup"])?;
            if tiles
                .insert(
                    tile.id,
                    TileInfo {
                        animation: tile
                            .animation
                            .into_iter()
                            .map(|f| Frame {
                                tile_id: f.tileid,
                                duration_ms: f.duration,
                            })
                            .collect(),
                        properties: props(tile.properties, &base)?,
                        metadata: tile.extra,
                    },
                )
                .is_some()
            {
                return Err(error("tileset", "duplicate tile metadata ID"));
            }
        }
        set.extra.remove("source");
        tilesets.push(Tileset {
            first_gid,
            name: set.name,
            image: relative(&base, &set.image)?,
            image_width: set.imagewidth,
            image_height: set.imageheight,
            tile_width: set.tilewidth,
            tile_height: set.tileheight,
            columns: set.columns,
            tile_count: set.tilecount,
            margin: set.margin,
            spacing: set.spacing,
            tiles,
            properties: props(set.properties, &base)?,
            metadata: set.extra,
        });
    }
    tilesets.sort_by_key(|s| s.first_gid);
    let mut layers = Vec::new();
    for raw in raw.layers {
        if raw.extra.get("compression").is_some_and(|v| v != "")
            || raw.extra.get("encoding").is_some_and(|v| v != "csv")
            || raw.extra.get("mode").is_some_and(|v| v != "normal")
        {
            return Err(error(
                "unsupported",
                "compressed/base64 data and non-normal blend modes are unsupported",
            ));
        }
        let content = match raw.kind.as_str() {
            "tilelayer" => {
                if !raw.objects.is_empty() {
                    return Err(error("layer", "tile layer contains objects"));
                }
                let chunks = if let Some(chunks) = raw.chunks {
                    if raw.x != 0 || raw.y != 0 {
                        return Err(error(
                            "unsupported",
                            "chunk layer x/y offsets must be zero; use offsetx/offsety",
                        ));
                    }
                    if raw.data.is_some() {
                        return Err(error("layer", "layer cannot contain both chunks and data"));
                    }
                    chunks
                        .into_iter()
                        .map(|c| Chunk {
                            x: c.x,
                            y: c.y,
                            width: c.width,
                            height: c.height,
                            cells: c.data,
                        })
                        .collect()
                } else {
                    let data: Vec<u32> = serde_json::from_value(
                        raw.data
                            .ok_or_else(|| error("layer", "missing tile data"))?,
                    )
                    .map_err(json_error)?;
                    vec![Chunk {
                        x: raw.x,
                        y: raw.y,
                        width: raw.width,
                        height: raw.height,
                        cells: data,
                    }]
                };
                LayerContent::Tiles { chunks }
            }
            "objectgroup" => {
                if raw.data.is_some() || raw.chunks.is_some() {
                    return Err(error("layer", "object layer contains tile data"));
                }
                let mut objects = Vec::new();
                for o in raw.objects {
                    if o.rotation != 0. {
                        return Err(error("unsupported", "rotated objects are unsupported"));
                    }
                    unsupported(
                        &o.extra,
                        &["ellipse", "polygon", "polyline", "gid", "text", "template"],
                    )?;
                    objects.push(Object {
                        id: o.id,
                        name: o.name,
                        class: if o.class.is_empty() { o.kind } else { o.class },
                        x: o.x,
                        y: o.y,
                        width: o.width,
                        height: o.height,
                        point: o.point,
                        visible: o.visible,
                        properties: props(o.properties, "")?,
                        metadata: o.extra,
                    });
                }
                LayerContent::Objects { objects }
            }
            _ => {
                return Err(error(
                    "unsupported",
                    format!("unsupported Tiled layer type {}", raw.kind),
                ));
            }
        };
        layers.push(Layer {
            id: raw.id,
            name: raw.name,
            visible: raw.visible,
            opacity: raw.opacity,
            offset: [raw.offsetx, raw.offsety],
            parallax: [raw.parallaxx, raw.parallaxy],
            tint: raw
                .tintcolor
                .as_deref()
                .map(color)
                .transpose()?
                .unwrap_or([255; 4]),
            content,
            properties: props(raw.properties, "")?,
            metadata: raw.extra,
        });
    }
    Ok(Map {
        quasar_version: VERSION,
        tile_width: raw.tilewidth,
        tile_height: raw.tileheight,
        width: raw.width,
        height: raw.height,
        infinite: raw.infinite,
        background: raw
            .backgroundcolor
            .as_deref()
            .map(color)
            .transpose()?
            .unwrap_or([0; 4]),
        tilesets,
        layers,
        properties: props(raw.properties, "")?,
        metadata: raw.extra,
    })
}
fn properties_json(props: &Properties) -> Vec<Value> {
    props
        .iter()
        .map(|p| {
            let mut v = p.metadata.clone();
            v.insert("name".into(), json!(p.name));
            v.insert("type".into(), json!(p.kind));
            v.insert("value".into(), p.value.clone());
            json!(v)
        })
        .collect()
}
fn hex(c: [u8; 4]) -> String {
    format!("#{:02x}{:02x}{:02x}{:02x}", c[3], c[0], c[1], c[2])
}
pub fn to_tiled_json(map: &Map) -> Result<Vec<u8>> {
    map.validate()?;
    let mut v = map.metadata.clone();
    v.extend(BTreeMap::from([
        ("type".into(), json!("map")),
        ("orientation".into(), json!("orthogonal")),
        ("renderorder".into(), json!("right-down")),
        ("width".into(), json!(map.width)),
        ("height".into(), json!(map.height)),
        ("tilewidth".into(), json!(map.tile_width)),
        ("tileheight".into(), json!(map.tile_height)),
        ("infinite".into(), json!(map.infinite)),
        ("backgroundcolor".into(), json!(hex(map.background))),
        ("properties".into(), json!(properties_json(&map.properties))),
    ]));
    let sets: Vec<Value> = map
        .tilesets
        .iter()
        .map(|s| {
            let mut t = s.metadata.clone();
            t.extend(BTreeMap::from([
                ("firstgid".into(), json!(s.first_gid)),
                ("name".into(), json!(s.name)),
                ("image".into(), json!(s.image)),
                ("imagewidth".into(), json!(s.image_width)),
                ("imageheight".into(), json!(s.image_height)),
                ("tilewidth".into(), json!(s.tile_width)),
                ("tileheight".into(), json!(s.tile_height)),
                ("tilecount".into(), json!(s.tile_count)),
                ("columns".into(), json!(s.columns)),
                ("margin".into(), json!(s.margin)),
                ("spacing".into(), json!(s.spacing)),
                ("properties".into(), json!(properties_json(&s.properties))),
            ]));
            let tiles: Vec<Value> = s
                .tiles
                .iter()
                .map(|(id, info)| {
                    let mut tile = info.metadata.clone();
                    tile.insert("id".into(), json!(id));
                    tile.insert(
                        "properties".into(),
                        json!(properties_json(&info.properties)),
                    );
                    if !info.animation.is_empty() {
                        tile.insert(
                            "animation".into(),
                            json!(
                                info.animation
                                    .iter()
                                    .map(|f| json!({"tileid":f.tile_id,"duration":f.duration_ms}))
                                    .collect::<Vec<_>>()
                            ),
                        );
                    }
                    json!(tile)
                })
                .collect();
            t.insert("tiles".into(), json!(tiles));
            json!(t)
        })
        .collect();
    v.insert("tilesets".into(), json!(sets));
    let mut layers = Vec::new();
    for layer in &map.layers {
        let mut l = layer.metadata.clone();
        l.extend(BTreeMap::from([
            ("id".into(), json!(layer.id)),
            ("name".into(), json!(layer.name)),
            ("visible".into(), json!(layer.visible)),
            ("opacity".into(), json!(layer.opacity)),
            ("offsetx".into(), json!(layer.offset[0])),
            ("offsety".into(), json!(layer.offset[1])),
            ("parallaxx".into(), json!(layer.parallax[0])),
            ("parallaxy".into(), json!(layer.parallax[1])),
            ("tintcolor".into(), json!(hex(layer.tint))),
            (
                "properties".into(),
                json!(properties_json(&layer.properties)),
            ),
        ]));
        match &layer.content {
            LayerContent::Tiles { chunks } => {
                l.insert("type".into(), json!("tilelayer"));
                if map.infinite {
                    l.insert("chunks".into(),json!(chunks.iter().map(|c|json!({"x":c.x,"y":c.y,"width":c.width,"height":c.height,"data":c.cells})).collect::<Vec<_>>()));
                } else {
                    let count = map.width as usize * map.height as usize;
                    if count > MAX_CELLS {
                        return Err(error("bounds", "finite Tiled export exceeds cell budget"));
                    }
                    let mut cells = vec![0; count];
                    for c in chunks {
                        for y in 0..c.height {
                            let dest = (c.y as u32 + y) * map.width + c.x as u32;
                            cells[dest as usize..dest as usize + c.width as usize].copy_from_slice(
                                &c.cells[(y * c.width) as usize..((y + 1) * c.width) as usize],
                            );
                        }
                    }
                    l.insert("data".into(), json!(cells));
                }
                l.insert("width".into(), json!(map.width));
                l.insert("height".into(), json!(map.height));
                l.insert("x".into(), json!(0));
                l.insert("y".into(), json!(0));
            }
            LayerContent::Objects { objects } => {
                l.insert("type".into(), json!("objectgroup"));
                l.insert(
                    "objects".into(),
                    json!(
                        objects
                            .iter()
                            .map(|o| {
                                let mut v = o.metadata.clone();
                                v.extend(BTreeMap::from([
                                    ("id".into(), json!(o.id)),
                                    ("name".into(), json!(o.name)),
                                    ("class".into(), json!(o.class)),
                                    ("x".into(), json!(o.x)),
                                    ("y".into(), json!(o.y)),
                                    ("width".into(), json!(o.width)),
                                    ("height".into(), json!(o.height)),
                                    ("point".into(), json!(o.point)),
                                    ("visible".into(), json!(o.visible)),
                                    ("properties".into(), json!(properties_json(&o.properties))),
                                ]));
                                json!(v)
                            })
                            .collect::<Vec<_>>()
                    ),
                );
            }
        }
        layers.push(json!(l));
    }
    v.insert("layers".into(), json!(layers));
    serde_json::to_vec_pretty(&v).map_err(json_error)
}
/// Writes a portable package: canonical map and all declared image/file assets.
pub fn write_bundle(
    map: &Map,
    root: &AssetRoot,
    output: impl AsRef<Path>,
    tiled: bool,
) -> Result<()> {
    map.validate()?;
    let output = output.as_ref();
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| error("io", e.to_string()))?;
    let dest = AssetRoot::new(parent)?;
    if std::fs::symlink_metadata(output).is_ok() {
        dest.resolve(
            output
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| error("path", "output filename must be UTF-8"))?,
        )?;
    }
    let assets = map.asset_paths();
    for asset in assets {
        let rel = relative("", &asset)?;
        let source = root.resolve(&asset)?;
        let target = dest.path().join(&rel);
        if target
            == dest.path().join(
                output
                    .file_name()
                    .ok_or_else(|| error("path", "missing output file name"))?,
            )
        {
            return Err(error("path", "map output would overwrite an asset"));
        }
        if output.exists()
            && target.exists()
            && std::fs::canonicalize(output).map_err(|e| error("io", e.to_string()))?
                == std::fs::canonicalize(&target).map_err(|e| error("io", e.to_string()))?
        {
            return Err(error("path", "map output aliases an asset"));
        }
        let dir = target
            .parent()
            .ok_or_else(|| error("path", "asset path has no parent directory"))?;
        std::fs::create_dir_all(dir).map_err(|e| error("io", e.to_string()))?;
        if !std::fs::canonicalize(dir)
            .map_err(|e| error("io", e.to_string()))?
            .starts_with(dest.path())
        {
            return Err(error("path", "output directory escapes root"));
        }
        if std::fs::symlink_metadata(&target).is_ok() {
            let existing = dest.resolve(&rel)?;
            if existing == source {
                continue;
            }
        }
        std::fs::copy(source, target).map_err(|e| error("io", e.to_string()))?;
    }
    let bytes = if tiled {
        to_tiled_json(map)?
    } else {
        map.canonical_json()?
    };
    std::fs::write(output, bytes).map_err(|e| error("io", e.to_string()))
}
