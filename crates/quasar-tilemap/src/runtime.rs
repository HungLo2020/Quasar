use crate::*;
use std::collections::BTreeMap;
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Bounds {
    pub fn valid(self) -> bool {
        [
            self.x,
            self.y,
            self.width,
            self.height,
            self.x + self.width,
            self.y + self.height,
        ]
        .iter()
        .all(|v| v.is_finite())
            && self.width >= 0.
            && self.height >= 0.
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub x: i32,
    pub y: i32,
    pub gid: u32,
}
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct QueryStats {
    pub visited_rows: usize,
    pub visited_cells: usize,
    pub nonempty_cells: usize,
}
struct Span {
    x: i32,
    end: i32,
    chunk: usize,
    row: u32,
}
/// Immutable validated map with a row/span index. Queries visit visible rows and
/// intersecting spans, not all chunks or cells. Recompile after explicit edits.
pub struct Runtime {
    map: Map,
    rows: BTreeMap<u32, BTreeMap<i32, Vec<Span>>>,
    animations: Vec<BTreeMap<u32, (Vec<u64>, u64)>>,
    stored_cells: usize,
}
impl Runtime {
    pub fn new(map: Map) -> Result<Self> {
        map.validate()?;
        let mut rows = BTreeMap::new();
        let mut stored_cells = 0;
        for layer in &map.layers {
            if let LayerContent::Tiles { chunks } = &layer.content {
                let mut index: BTreeMap<i32, Vec<Span>> = BTreeMap::new();
                for (i, c) in chunks.iter().enumerate() {
                    stored_cells += c.cells.len();
                    for row in 0..c.height {
                        index.entry(c.y + row as i32).or_default().push(Span {
                            x: c.x,
                            end: c.x + c.width as i32,
                            chunk: i,
                            row,
                        });
                    }
                }
                for spans in index.values_mut() {
                    spans.sort_by_key(|s| s.x);
                }
                rows.insert(layer.id, index);
            }
        }
        let animations = map
            .tilesets
            .iter()
            .map(|s| {
                s.tiles
                    .iter()
                    .filter(|(_, t)| !t.animation.is_empty())
                    .map(|(id, t)| {
                        let mut end = 0;
                        let ends = t
                            .animation
                            .iter()
                            .map(|f| {
                                end += u64::from(f.duration_ms);
                                end
                            })
                            .collect();
                        (*id, (ends, end))
                    })
                    .collect()
            })
            .collect();
        Ok(Self {
            map,
            rows,
            animations,
            stored_cells,
        })
    }
    pub fn map(&self) -> &Map {
        &self.map
    }
    pub fn stored_cells(&self) -> usize {
        self.stored_cells
    }
    pub fn tile(&self, layer: u32, x: i32, y: i32) -> Option<Cell> {
        let layer_data = self.map.layer(layer)?;
        let LayerContent::Tiles { chunks } = &layer_data.content else {
            return None;
        };
        let spans = self.rows.get(&layer)?.get(&y)?;
        let i = spans.partition_point(|s| s.x <= x).checked_sub(1)?;
        let span = &spans[i];
        if x >= span.end {
            return None;
        }
        let c = &chunks[span.chunk];
        let gid = c.cells[(span.row * c.width + (x - span.x) as u32) as usize];
        Some(Cell { x, y, gid })
    }
    /// Half-open world-space bounds. Layer offsets are included; parallax is a
    /// rendering concern and never changes physical collision coordinates.
    pub fn for_each_cell(
        &self,
        layer: u32,
        bounds: Bounds,
        mut visitor: impl FnMut(Cell),
    ) -> Result<QueryStats> {
        if !bounds.valid() {
            return Err(error("bounds", "invalid query rectangle"));
        }
        let data = self
            .map
            .layer(layer)
            .ok_or_else(|| error("layer", "unknown layer"))?;
        let LayerContent::Tiles { chunks } = &data.content else {
            return Err(error("layer", "expected tile layer"));
        };
        let mut stats = QueryStats::default();
        if bounds.width == 0. || bounds.height == 0. {
            return Ok(stats);
        }
        let x0 = ((bounds.x - f64::from(data.offset[0])) / f64::from(self.map.tile_width))
            .floor()
            .clamp(i32::MIN as f64, i32::MAX as f64) as i32;
        let x1 = ((bounds.x + bounds.width - f64::from(data.offset[0]))
            / f64::from(self.map.tile_width))
        .ceil()
        .clamp(i32::MIN as f64, i32::MAX as f64) as i32;
        let y0 = ((bounds.y - f64::from(data.offset[1])) / f64::from(self.map.tile_height))
            .floor()
            .clamp(i32::MIN as f64, i32::MAX as f64) as i32;
        let y1 = ((bounds.y + bounds.height - f64::from(data.offset[1]))
            / f64::from(self.map.tile_height))
        .ceil()
        .clamp(i32::MIN as f64, i32::MAX as f64) as i32;
        let rows = &self.rows[&layer];
        for (&y, spans) in rows.range(y0..y1) {
            stats.visited_rows += 1;
            let first = spans.partition_point(|s| s.end <= x0);
            for span in &spans[first..] {
                if span.x >= x1 {
                    break;
                }
                let c = &chunks[span.chunk];
                for x in span.x.max(x0)..span.end.min(x1) {
                    stats.visited_cells += 1;
                    let gid = c.cells[(span.row * c.width + (x - span.x) as u32) as usize];
                    if gid & GID_MASK != 0 {
                        stats.nonempty_cells += 1;
                        visitor(Cell { x, y, gid });
                    }
                }
            }
        }
        Ok(stats)
    }
    pub fn collides(&self, layer: u32, bounds: Bounds, property_name: &str) -> Result<bool> {
        let layer_solid = self
            .map
            .layer(layer)
            .and_then(|l| property(&l.properties, property_name))
            .and_then(serde_json::Value::as_bool)
            == Some(true);
        let mut hit = false;
        self.for_each_cell(layer, bounds, |cell| {
            let (set, tile) = self.map.resolve(cell.gid).expect("validated GID").unwrap();
            if layer_solid
                || self.map.tilesets[set]
                    .tiles
                    .get(&tile)
                    .and_then(|t| property(&t.properties, property_name))
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
            {
                hit = true;
            }
        })?;
        Ok(hit)
    }
    pub fn objects(&self, layer: u32) -> Result<&[Object]> {
        match &self
            .map
            .layer(layer)
            .ok_or_else(|| error("layer", "unknown object layer"))?
            .content
        {
            LayerContent::Objects { objects } => Ok(objects),
            _ => Err(error("layer", "expected object layer")),
        }
    }
    pub fn objects_in_bounds(&self, layer: u32, bounds: Bounds) -> Result<Vec<&Object>> {
        if !bounds.valid() {
            return Err(error("bounds", "invalid object query"));
        }
        if bounds.width == 0. || bounds.height == 0. {
            return Ok(Vec::new());
        }
        let offset = self
            .map
            .layer(layer)
            .ok_or_else(|| error("layer", "unknown layer"))?
            .offset;
        Ok(self
            .objects(layer)?
            .iter()
            .filter(|o| {
                let x = o.x + f64::from(offset[0]);
                let y = o.y + f64::from(offset[1]);
                if o.point {
                    x >= bounds.x
                        && y >= bounds.y
                        && x < bounds.x + bounds.width
                        && y < bounds.y + bounds.height
                } else {
                    o.width > 0.
                        && o.height > 0.
                        && x < bounds.x + bounds.width
                        && y < bounds.y + bounds.height
                        && x + o.width > bounds.x
                        && y + o.height > bounds.y
                }
            })
            .collect())
    }
    pub fn animated_tile(&self, set: usize, local: u32, elapsed_ms: u64) -> Option<u32> {
        let tileset = self.map.tilesets.get(set)?;
        if local >= tileset.tile_count {
            return None;
        }
        Some(
            if let Some((ends, total)) = self.animations[set].get(&local) {
                let index = ends.partition_point(|end| *end <= elapsed_ms % total);
                self.map.tilesets[set].tiles[&local].animation[index].tile_id
            } else {
                local
            },
        )
    }
}
