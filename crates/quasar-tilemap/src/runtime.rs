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
#[derive(Clone, Copy)]
struct Span {
    x: i32,
    end: i32,
    chunk: usize,
    row: u32,
}
/// Validated map with an incrementally maintained row/span index.
pub struct Runtime {
    map: Map,
    rows: BTreeMap<u32, BTreeMap<i32, BTreeMap<i32, Span>>>,
    layers: BTreeMap<u32, usize>,
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
                let mut index: BTreeMap<i32, BTreeMap<i32, Span>> = BTreeMap::new();
                for (i, c) in chunks.iter().enumerate() {
                    stored_cells += c.cells.len();
                    for row in 0..c.height {
                        index.entry(c.y + row as i32).or_default().insert(
                            c.x,
                            Span {
                                x: c.x,
                                end: c.x + c.width as i32,
                                chunk: i,
                                row,
                            },
                        );
                    }
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
            layers: map
                .layers
                .iter()
                .enumerate()
                .map(|(i, l)| (l.id, i))
                .collect(),
            map,
            rows,
            animations,
            stored_cells,
        })
    }
    /// Atomically applies a bounded batch. Duplicate coordinates use the last GID.
    /// Existing cells change in place; only newly occupied rows/spans are indexed.
    pub fn set_tiles(&mut self, edits: &[TileEdit]) -> Result<EditStats> {
        let plan = plan_edits(
            &self.map,
            &self.layers,
            self.stored_cells,
            edits,
            |layer, x, y| {
                let spans = self.rows.get(&layer)?.get(&y)?;
                let (_, span) = spans.range(..=x).next_back()?;
                (x < span.end).then_some((span.chunk, span.row, span.x))
            },
        )?;
        let stats = apply_plan(&mut self.map, &mut self.rows, plan);
        self.stored_cells += stats.inserted_cells;
        Ok(stats)
    }
    pub fn set_tile(&mut self, layer: u32, x: i32, y: i32, gid: u32) -> Result<EditStats> {
        self.set_tiles(&[TileEdit { layer, x, y, gid }])
    }
    /// Returns the edited document without copying cell storage.
    pub fn into_map(self) -> Map {
        self.map
    }
    pub fn map(&self) -> &Map {
        &self.map
    }
    pub fn stored_cells(&self) -> usize {
        self.stored_cells
    }
    pub fn tile(&self, layer: u32, x: i32, y: i32) -> Option<Cell> {
        let layer_data = &self.map.layers[*self.layers.get(&layer)?];
        let LayerContent::Tiles { chunks } = &layer_data.content else {
            return None;
        };
        let spans = self.rows.get(&layer)?.get(&y)?;
        let (_, span) = spans.range(..=x).next_back()?;
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
            let first = spans.range(..=x0).next_back().map_or(x0, |(&x, _)| x);
            for span in spans.range(first..x1).map(|(_, s)| s) {
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

/// One cell mutation in map coordinates (including negative infinite-map cells).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileEdit {
    pub layer: u32,
    pub x: i32,
    pub y: i32,
    pub gid: u32,
}
/// Structural work performed, independent of map size. No existing cell buffers are cloned.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct EditStats {
    pub changed_cells: usize,
    pub inserted_cells: usize,
    pub indexed_spans: usize,
}
struct Planned {
    layer: usize,
    x: i32,
    y: i32,
    gid: u32,
    cell: Option<(usize, usize)>,
}
fn plan_edits(
    map: &Map,
    layers: &BTreeMap<u32, usize>,
    stored: usize,
    edits: &[TileEdit],
    mut locate: impl FnMut(u32, i32, i32) -> Option<(usize, u32, i32)>,
) -> Result<Vec<Planned>> {
    if edits.len() > MAX_CELLS {
        return Err(error("bounds", "edit batch exceeds cell budget"));
    }
    let mut unique = BTreeMap::new();
    for e in edits {
        map.resolve(e.gid)?;
        if e.gid & TRANSPOSE != 0 && map.tile_width != map.tile_height {
            return Err(error(
                "unsupported",
                "diagonal tile transforms require square tiles",
            ));
        }
        if !map.infinite
            && (e.x < 0 || e.y < 0 || e.x as u32 >= map.width || e.y as u32 >= map.height)
        {
            return Err(error("bounds", "edit exceeds map coordinate bounds"));
        }
        let &layer = layers
            .get(&e.layer)
            .ok_or_else(|| error("layer", "unknown edit layer"))?;
        let LayerContent::Tiles { chunks } = &map.layers[layer].content else {
            return Err(error("layer", "expected tile layer"));
        };
        let cell = locate(e.layer, e.x, e.y)
            .map(|(c, row, x)| (c, (row * chunks[c].width + (e.x - x) as u32) as usize));
        // The schema bounds chunk origins, not every cell in an existing chunk.
        if cell.is_none() && (e.x.abs_diff(0) > 1_000_000 || e.y.abs_diff(0) > 1_000_000) {
            return Err(error("bounds", "new cell exceeds chunk origin bounds"));
        }
        unique.insert(
            (layer, e.y, e.x),
            Planned {
                layer,
                x: e.x,
                y: e.y,
                gid: e.gid,
                cell,
            },
        );
    }
    let mut added = BTreeMap::<usize, usize>::new();
    for p in unique.values().filter(|p| p.cell.is_none()) {
        *added.entry(p.layer).or_default() += 1;
    }
    if stored + added.values().sum::<usize>() > MAX_CELLS {
        return Err(error("bounds", "map exceeds stored cell budget"));
    }
    for (l, n) in added {
        let LayerContent::Tiles { chunks } = &map.layers[l].content else {
            unreachable!()
        };
        if chunks.len() + n > 65536 {
            return Err(error("bounds", "too many chunks"));
        }
    }
    Ok(unique.into_values().collect())
}
fn apply_plan(
    map: &mut Map,
    rows: &mut BTreeMap<u32, BTreeMap<i32, BTreeMap<i32, Span>>>,
    plan: Vec<Planned>,
) -> EditStats {
    let mut stats = EditStats::default();
    for p in plan {
        let layer = &mut map.layers[p.layer];
        let LayerContent::Tiles { chunks } = &mut layer.content else {
            unreachable!()
        };
        if let Some((c, i)) = p.cell {
            stats.changed_cells += usize::from(chunks[c].cells[i] != p.gid);
            chunks[c].cells[i] = p.gid;
        } else {
            let c = chunks.len();
            chunks.push(Chunk {
                x: p.x,
                y: p.y,
                width: 1,
                height: 1,
                cells: vec![p.gid],
            });
            rows.entry(layer.id)
                .or_default()
                .entry(p.y)
                .or_default()
                .insert(
                    p.x,
                    Span {
                        x: p.x,
                        end: p.x + 1,
                        chunk: c,
                        row: 0,
                    },
                );
            stats.inserted_cells += 1;
            stats.indexed_spans += 1;
        }
    }
    stats
}
/// Validates an editable public Map once, then commits a batch without cloning it.
pub(crate) fn edit_map(map: &mut Map, edits: &[TileEdit]) -> Result<EditStats> {
    map.validate()?;
    let stored = map
        .layers
        .iter()
        .map(|l| match &l.content {
            LayerContent::Tiles { chunks } => chunks.iter().map(|c| c.cells.len()).sum(),
            _ => 0,
        })
        .sum();
    let layers = map
        .layers
        .iter()
        .enumerate()
        .map(|(i, l)| (l.id, i))
        .collect();
    let plan = plan_edits(map, &layers, stored, edits, |id, x, y| {
        let LayerContent::Tiles { chunks } = &map.layer(id)?.content else {
            return None;
        };
        chunks.iter().enumerate().find_map(|(i, c)| {
            let dx = i64::from(x) - i64::from(c.x);
            let dy = i64::from(y) - i64::from(c.y);
            (dx >= 0 && dy >= 0 && dx < i64::from(c.width) && dy < i64::from(c.height))
                .then_some((i, dy as u32, c.x))
        })
    })?;
    Ok(apply_plan(map, &mut BTreeMap::new(), plan))
}
