//! The block world: column streaming, block and sky light, threaded
//! meshing, connectivity culling, the player and block edits.
use crate::content::Content;
use crate::terrain::{HEIGHT_CHUNKS, Terrain, WORLD_TOP};
use quasar_jobs::{JobPool, JobPoolConfig};
use quasar_math::{IVec3, Mat4, Vec3};
use quasar_voxel::{
    AIR, BlockChannels, ChannelId, Cube16, Cursor, EditBatch, EditOptions, Face, Orientation,
    PropagationRules, Propagator, QueueDiscipline, RayHit, Schema, Snapshot, StorageKind, TieBreak,
    Volume, codec,
};
use quasar_voxel_mesh::{
    CatalogFaces, CatalogLayers, CatalogModels, CulledInput, CulledModelExtractor, MeshBuffers,
    RemeshQueue, VertexShade,
};
use quasar_voxel_view::{
    ChunkFrustum, FrustumPolicy, StreamBudget, StreamingPlanner, Viewer, VisibilityGraph,
    chunk_connectivity,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub const EDGE: i32 = 16;
const GRAVITY: f32 = -28.0;
const JUMP: f32 = 8.4;
const WALK: f32 = 4.6;
const FLY: f32 = 11.0;
const HALF_WIDTH: f32 = 0.3;
const HEIGHT: f32 = 1.8;
pub const EYE: f32 = 1.62;
pub const REACH: f32 = 6.0;
const DT: f32 = 1.0 / 60.0;

/// The world's channels.
#[derive(Clone, Copy, Debug)]
pub struct Channels {
    pub block: ChannelId<u16>,
    pub orientation: ChannelId<u8>,
    pub part: ChannelId<u32>,
    pub block_light: ChannelId<u8>,
    pub sky_light: ChannelId<u8>,
}

impl Channels {
    pub fn blocks(&self) -> BlockChannels {
        BlockChannels {
            block: self.block,
            orientation: self.orientation,
            part: self.part,
        }
    }
}

/// Block and sky light rules: opaque blocks stop light, leaves dim it, sky
/// light falls straight down without loss.
struct LightRules {
    block: ChannelId<u16>,
    content: Arc<Content>,
    sky: bool,
}

impl PropagationRules<Cube16> for LightRules {
    fn cost(
        &self,
        volume: &Volume<Cube16>,
        cursor: &mut Cursor,
        to: IVec3,
        travel: Face,
    ) -> Option<u8> {
        if to.y < 0 || to.y >= WORLD_TOP {
            return None;
        }
        let b = volume.get_with(cursor, self.block, to) as usize;
        if self.content.opaque.get(b).copied().unwrap_or(false) {
            return None;
        }
        let base = if self.sky && travel == Face::NegY {
            0
        } else {
            1
        };
        Some(base + self.content.filter.get(b).copied().unwrap_or(0))
    }
    fn emission(&self, volume: &Volume<Cube16>, cursor: &mut Cursor, at: IVec3) -> u8 {
        let b = volume.get_with(cursor, self.block, at) as usize;
        if self.sky {
            let open = !self.content.opaque.get(b).copied().unwrap_or(false);
            if at.y == WORLD_TOP - 1 && open { 15 } else { 0 }
        } else {
            self.content.emission.get(b).copied().unwrap_or(0)
        }
    }
}

/// One tick of player intent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    pub forward: f32,
    pub strafe: f32,
    /// Look delta in radians (yaw, pitch).
    pub look: [f32; 2],
    pub jump: bool,
    pub descend: bool,
    pub place: bool,
    pub remove: bool,
    pub slot: Option<usize>,
    pub cycle: i32,
    pub toggle_fly: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Player {
    /// Feet position (centre of the bottom face).
    pub position: Vec3,
    pub velocity: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
    pub flying: bool,
}

impl Player {
    pub fn eye(&self) -> Vec3 {
        self.position + Vec3::Y * EYE
    }
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, -cy * cp)
    }
    fn bounds(&self, at: Vec3) -> (Vec3, Vec3) {
        (
            at - Vec3::new(HALF_WIDTH, 0.0, HALF_WIDTH),
            at + Vec3::new(HALF_WIDTH, HEIGHT, HALF_WIDTH),
        )
    }
}

/// Counters for the HUD, logs and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorldStats {
    pub columns: usize,
    pub chunks: usize,
    pub meshed: usize,
    pub pending_meshes: usize,
    pub meshes_built: u64,
    pub quads: usize,
    pub placed: u64,
    pub removed: u64,
    pub light_changes: u64,
    pub visible: usize,
}

/// Geometry changes for a renderer to apply.
#[derive(Default)]
pub struct MeshUpdates {
    pub upload: Vec<(IVec3, MeshBuffers)>,
    pub remove: Vec<IVec3>,
}

struct MeshJob {
    blocks: Snapshot<u16>,
    block_light: Snapshot<u8>,
    sky_light: Snapshot<u8>,
}

pub struct BlockWorld {
    pub volume: Volume<Cube16>,
    pub ch: Channels,
    pub content: Arc<Content>,
    pub terrain: Terrain,
    pub player: Player,
    pub selected: usize,
    pub target: Option<RayHit>,
    pub view_radius: i32,
    pub stats: WorldStats,
    /// Wait for every meshing job each tick (deterministic headless runs).
    pub sync_meshing: bool,
    sky_rules: LightRules,
    block_rules: LightRules,
    propagator: Propagator,
    planner: StreamingPlanner,
    columns: BTreeSet<(i32, i32)>,
    dirty: BTreeSet<(i32, i32, i32)>,
    visibility: VisibilityGraph,
    connectivity_scratch: Vec<u32>,
    remesh: RemeshQueue<(i32, i32, i32), MeshBuffers>,
    revision: u64,
    meshed: BTreeMap<(i32, i32, i32), usize>,
    pool: JobPool,
    updates: MeshUpdates,
    tick: u64,
}

fn key(c: IVec3) -> (i32, i32, i32) {
    (c.x, c.y, c.z)
}

impl BlockWorld {
    /// `workers` meshing/generation threads (0 = available parallelism).
    pub fn new(seed: u64, view_radius: i32, workers: usize) -> Self {
        let content = Arc::new(Content::new());
        let mut schema = Schema::new();
        let block = schema.channel("block", StorageKind::Paletted, AIR);
        let orientation = schema.channel("orientation", StorageKind::Paletted, 0u8);
        let part = schema.channel("part", StorageKind::Sparse, 0u32);
        let block_light = schema.derived_channel("block_light", StorageKind::Nibble, 0u8);
        let sky_light = schema.derived_channel("sky_light", StorageKind::Nibble, 0u8);
        let ch = Channels {
            block,
            orientation,
            part,
            block_light,
            sky_light,
        };
        let terrain = Terrain {
            seed,
            block,
            blocks: content.blocks,
        };
        let volume = Volume::new(schema).with_source(Arc::new(terrain.clone()));
        let spawn_height = terrain.height(8, 8) + 1;
        let pool = if workers == 0 {
            JobPool::new(JobPoolConfig::default())
        } else {
            JobPool::new(JobPoolConfig::new(workers, 4096))
        }
        .expect("job pool");
        Self {
            volume,
            ch,
            sky_rules: LightRules {
                block,
                content: content.clone(),
                sky: true,
            },
            block_rules: LightRules {
                block,
                content: content.clone(),
                sky: false,
            },
            content,
            terrain,
            player: Player {
                position: Vec3::new(8.5, spawn_height as f32, 8.5),
                velocity: Vec3::ZERO,
                yaw: 0.6,
                pitch: -0.25,
                on_ground: false,
                flying: false,
            },
            selected: 0,
            target: None,
            view_radius,
            stats: WorldStats::default(),
            sync_meshing: false,
            propagator: Propagator::new(QueueDiscipline::BucketFifo),
            planner: StreamingPlanner::new(StreamBudget {
                max_loads: 4,
                max_unloads: 32,
            }),
            columns: BTreeSet::new(),
            dirty: BTreeSet::new(),
            visibility: VisibilityGraph::new(),
            connectivity_scratch: Vec::new(),
            remesh: RemeshQueue::new(),
            revision: 0,
            meshed: BTreeMap::new(),
            pool,
            updates: MeshUpdates::default(),
            tick: 0,
        }
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }
    pub fn selected_block(&self) -> u16 {
        self.content.hotbar[self.selected]
    }

    /// Advances one fixed step.
    pub fn update(&mut self, input: Input) {
        self.tick += 1;
        if let Some(slot) = input.slot
            && slot < self.content.hotbar.len()
        {
            self.selected = slot;
        }
        if input.cycle != 0 {
            let n = self.content.hotbar.len() as i32;
            self.selected = (self.selected as i32 + input.cycle).rem_euclid(n) as usize;
        }
        if input.toggle_fly {
            self.player.flying = !self.player.flying;
            self.player.velocity = Vec3::ZERO;
        }
        self.move_player(input);
        self.target = self.raycast();
        if input.remove {
            self.remove_target();
        } else if input.place {
            self.place_target();
        }
        self.stream();
        self.schedule_meshing();
        self.collect_meshes();
    }

    fn solid_height(&self, cell: IVec3) -> f32 {
        let b = self.volume.get(self.ch.block, cell) as usize;
        self.content.collision.get(b).copied().unwrap_or(0.0)
    }

    fn collides(&self, min: Vec3, max: Vec3) -> bool {
        let lo = min.floor().as_ivec3();
        let hi = (max - Vec3::splat(1e-4)).floor().as_ivec3();
        for y in lo.y..=hi.y {
            for z in lo.z..=hi.z {
                for x in lo.x..=hi.x {
                    let cell = IVec3::new(x, y, z);
                    // Unloaded columns are solid so the player never falls out.
                    if !self
                        .columns
                        .contains(&(x.div_euclid(EDGE), z.div_euclid(EDGE)))
                        && (0..WORLD_TOP).contains(&y)
                    {
                        return true;
                    }
                    let h = self.solid_height(cell);
                    if h > 0.0 && min.y < y as f32 + h && max.y > y as f32 {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn move_player(&mut self, input: Input) {
        let p = &mut self.player;
        p.yaw += input.look[0];
        p.pitch = (p.pitch + input.look[1]).clamp(-1.55, 1.55);
        let (sy, cy) = p.yaw.sin_cos();
        let forward = Vec3::new(sy, 0.0, -cy);
        let right = Vec3::new(cy, 0.0, sy);
        let wish = (forward * input.forward + right * input.strafe).clamp_length_max(1.0);
        if p.flying {
            let vertical = f32::from(input.jump) - f32::from(input.descend);
            p.velocity = wish * FLY + Vec3::Y * vertical * FLY;
        } else {
            p.velocity.x = wish.x * WALK;
            p.velocity.z = wish.z * WALK;
            p.velocity.y = (p.velocity.y + GRAVITY * DT).max(-50.0);
            if input.jump && p.on_ground {
                p.velocity.y = JUMP;
            }
        }
        let mut position = self.player.position;
        let mut velocity = self.player.velocity;
        let mut on_ground = false;
        for axis in [1usize, 0, 2] {
            let mut next = position;
            next[axis] += velocity[axis] * DT;
            let (min, max) = self.player.bounds(next);
            if self.collides(min, max) {
                // Step up onto half blocks while walking.
                if axis != 1 && self.player.on_ground && !self.player.flying {
                    let stepped = next + Vec3::Y * 0.51;
                    let (smin, smax) = self.player.bounds(stepped);
                    if !self.collides(smin, smax) {
                        position = stepped;
                        continue;
                    }
                }
                if axis == 1 && velocity.y < 0.0 {
                    on_ground = true;
                }
                velocity[axis] = 0.0;
            } else {
                position = next;
            }
        }
        let p = &mut self.player;
        p.position = position;
        p.velocity = velocity;
        p.on_ground = on_ground;
    }

    fn raycast(&self) -> Option<RayHit> {
        let p = &self.player;
        self.volume.raycast(
            self.ch.block,
            p.eye(),
            p.forward(),
            REACH,
            TieBreak::default(),
            |_, b| b != AIR,
        )
    }

    fn place_target(&mut self) {
        let Some(hit) = self.target else {
            return;
        };
        let cell = hit.adjacent();
        if !(0..WORLD_TOP).contains(&cell.y) {
            return;
        }
        let id = self.selected_block();
        let (min, max) = self.player.bounds(self.player.position);
        let h = self.content.collision[id as usize];
        let c = cell.as_vec3();
        let overlaps_player = h > 0.0
            && min.x < c.x + 1.0
            && max.x > c.x
            && min.z < c.z + 1.0
            && max.z > c.z
            && min.y < c.y + h
            && max.y > c.y;
        if overlaps_player {
            return;
        }
        let mut batch = EditBatch::new();
        if self
            .content
            .catalog
            .place(
                &mut batch,
                &self.volume,
                self.ch.blocks(),
                id,
                Orientation::IDENTITY,
                cell,
            )
            .is_ok()
        {
            self.apply(&batch);
            self.stats.placed += 1;
        }
    }

    fn remove_target(&mut self) {
        let Some(hit) = self.target else {
            return;
        };
        let mut batch = EditBatch::new();
        if self
            .content
            .catalog
            .remove(&mut batch, &self.volume, self.ch.blocks(), hit.voxel)
            .is_some()
        {
            self.apply(&batch);
            self.stats.removed += 1;
        }
    }

    /// Applies an edit and relights the changed cells.
    pub fn apply(&mut self, batch: &EditBatch) {
        let result = self.volume.apply(batch, EditOptions::default());
        let mut cells: Vec<IVec3> = result
            .changes
            .iter()
            .filter(|c| c.channel as usize == self.ch.block.index())
            .map(|c| c.voxel)
            .collect();
        cells.dedup();
        let sky =
            self.propagator
                .update(&mut self.volume, self.ch.sky_light, &cells, &self.sky_rules);
        let block = self.propagator.update(
            &mut self.volume,
            self.ch.block_light,
            &cells,
            &self.block_rules,
        );
        self.stats.light_changes += (sky.changed + block.changed) as u64;
        for c in result.dirty.iter().chain(&sky.dirty).chain(&block.dirty) {
            self.dirty.insert(key(*c));
        }
    }

    fn stream(&mut self) {
        let viewer = Viewer {
            position: self.player.position,
            forward: Some(self.player.forward()),
            radius: self.view_radius,
            vertical: Some((0, 0)),
            keep_margin: 1,
        };
        let loaded: Vec<IVec3> = self
            .columns
            .iter()
            .map(|&(x, z)| IVec3::new(x, 0, z))
            .collect();
        let plan = self.planner.plan(EDGE as usize, &[viewer], &loaded);
        if !plan.unload.is_empty() {
            for column in &plan.unload {
                self.columns.remove(&(column.x, column.z));
                for y in 0..HEIGHT_CHUNKS {
                    let c = IVec3::new(column.x, y, column.z);
                    self.visibility.remove(c);
                    self.remesh.forget(key(c));
                    self.dirty.remove(&key(c));
                    if self.meshed.remove(&key(c)).is_some() {
                        self.updates.remove.push(c);
                    }
                }
            }
            // Edited chunks stay resident (the example keeps edits in memory).
            let columns = &self.columns;
            self.volume
                .evict_generated(|c| columns.contains(&(c.x, c.z)));
        }
        if plan.load.is_empty() {
            return;
        }
        let coords: Vec<IVec3> = plan
            .load
            .iter()
            .flat_map(|c| (0..HEIGHT_CHUNKS).map(move |y| IVec3::new(c.x, y, c.z)))
            .collect();
        self.volume
            .ensure_chunks_parallel(&self.pool, &coords)
            .expect("chunk generation");
        for column in &plan.load {
            self.load_column(column.x, column.z);
        }
    }

    fn load_column(&mut self, cx: i32, cz: i32) {
        self.columns.insert((cx, cz));
        let (x0, z0) = (cx * EDGE, cz * EDGE);
        let mut sky_seeds = Vec::with_capacity(2048);
        let mut block_seeds = Vec::new();
        // Fast path: all-air chunks at the top of the column are fully sky
        // lit, so fill them uniformly instead of flooding cell by cell, and
        // seed only their bottom and side faces (the same final light).
        let mut open_from = HEIGHT_CHUNKS;
        for y in (0..HEIGHT_CHUNKS).rev() {
            let c = IVec3::new(cx, y, cz);
            let block = self.ch.block;
            let Some(chunk) = self.volume.chunk_mut(c) else {
                break;
            };
            if chunk.storage(block).uniform() != Some(AIR) {
                break;
            }
            chunk.fill(self.ch.sky_light, 15);
            open_from = y;
        }
        if open_from == HEIGHT_CHUNKS {
            for z in z0..z0 + EDGE {
                for x in x0..x0 + EDGE {
                    sky_seeds.push((IVec3::new(x, WORLD_TOP - 1, z), 15));
                }
            }
        } else {
            let bottom = open_from * EDGE;
            for z in z0..z0 + EDGE {
                for x in x0..x0 + EDGE {
                    sky_seeds.push((IVec3::new(x, bottom, z), 15));
                }
            }
            for y in bottom..WORLD_TOP {
                for i in 0..EDGE {
                    sky_seeds.push((IVec3::new(x0, y, z0 + i), 15));
                    sky_seeds.push((IVec3::new(x0 + EDGE - 1, y, z0 + i), 15));
                    sky_seeds.push((IVec3::new(x0 + i, y, z0), 15));
                    sky_seeds.push((IVec3::new(x0 + i, y, z0 + EDGE - 1), 15));
                }
            }
        }
        // Edited chunks kept from an earlier visit re-seed their emitters.
        for y in 0..HEIGHT_CHUNKS {
            let c = IVec3::new(cx, y, cz);
            if self.volume.chunk(c).is_some_and(|chunk| chunk.is_edited()) {
                for i in 0..4096 {
                    let p = c * EDGE + <Cube16 as quasar_voxel::ChunkLayout>::local(i).as_ivec3();
                    let e = self.content.emission[self.volume.get(self.ch.block, p) as usize];
                    if e > 0 {
                        block_seeds.push((p, e));
                    }
                }
            }
        }
        // Existing light in loaded neighbours floods into the new column.
        for (dx, dz, face_x, face_z) in [
            (-1, 0, Some(x0 - 1), None),
            (1, 0, Some(x0 + EDGE), None),
            (0, -1, None, Some(z0 - 1)),
            (0, 1, None, Some(z0 + EDGE)),
        ] {
            if !self.columns.contains(&(cx + dx, cz + dz)) {
                continue;
            }
            for y in 0..WORLD_TOP {
                for i in 0..EDGE {
                    let p = match (face_x, face_z) {
                        (Some(x), _) => IVec3::new(x, y, z0 + i),
                        (_, Some(z)) => IVec3::new(x0 + i, y, z),
                        _ => unreachable!(),
                    };
                    let s = self.volume.get(self.ch.sky_light, p);
                    if s > 1 {
                        sky_seeds.push((p, s));
                    }
                    let b = self.volume.get(self.ch.block_light, p);
                    if b > 1 {
                        block_seeds.push((p, b));
                    }
                }
            }
            for y in 0..HEIGHT_CHUNKS {
                self.dirty.insert((cx + dx, y, cz + dz));
            }
        }
        let sky = self.propagator.add(
            &mut self.volume,
            self.ch.sky_light,
            &sky_seeds,
            &self.sky_rules,
        );
        let block = self.propagator.add(
            &mut self.volume,
            self.ch.block_light,
            &block_seeds,
            &self.block_rules,
        );
        self.stats.light_changes += (sky.changed + block.changed) as u64;
        for c in sky.dirty.iter().chain(&block.dirty) {
            self.dirty.insert(key(*c));
        }
        for y in 0..HEIGHT_CHUNKS {
            self.dirty.insert((cx, y, cz));
        }
    }

    /// A chunk can be meshed once its four horizontal neighbours are loaded
    /// (so border faces and light are final).
    fn meshable(&self, c: (i32, i32, i32)) -> bool {
        (0..HEIGHT_CHUNKS).contains(&c.1)
            && [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)]
                .iter()
                .all(|(dx, dz)| self.columns.contains(&(c.0 + dx, c.2 + dz)))
    }

    fn schedule_meshing(&mut self) {
        let eye = self.player.eye();
        let mut ready: Vec<(i64, (i32, i32, i32))> = self
            .dirty
            .iter()
            .copied()
            .filter(|c| self.meshable(*c))
            .map(|c| {
                let centre = (IVec3::new(c.0, c.1, c.2).as_vec3() + 0.5) * EDGE as f32;
                ((centre - eye).length_squared() as i64, c)
            })
            .collect();
        ready.sort();
        for (distance, c) in ready.into_iter().take(96) {
            self.dirty.remove(&c);
            let coord = IVec3::new(c.0, c.1, c.2);
            let opaque = &self.content.opaque;
            let connectivity = chunk_connectivity(
                &self.volume,
                self.ch.block,
                coord,
                &|b: u16| opaque.get(b as usize).copied().unwrap_or(false),
                &mut self.connectivity_scratch,
            );
            self.visibility.set(coord, connectivity);
            let empty = self
                .volume
                .chunk(coord)
                .is_none_or(|chunk| chunk.storage(self.ch.block).uniform() == Some(AIR));
            if empty {
                self.remesh.forget(c);
                if self.meshed.remove(&c).is_some() {
                    self.updates.remove.push(coord);
                }
                continue;
            }
            let mut job = MeshJob {
                blocks: Snapshot::new(),
                block_light: Snapshot::new(),
                sky_light: Snapshot::new(),
            };
            self.volume
                .snapshot(self.ch.block, coord, 1, &mut job.blocks);
            self.volume
                .snapshot(self.ch.block_light, coord, 1, &mut job.block_light);
            self.volume
                .snapshot(self.ch.sky_light, coord, 1, &mut job.sky_light);
            let content = self.content.clone();
            self.revision += 1;
            let priority = -(distance.min(i32::MAX as i64) as i32);
            self.remesh
                .request(&self.pool, c, self.revision, priority, move || {
                    mesh_chunk(&content, &job)
                })
                .expect("mesh job");
        }
        if self.sync_meshing {
            let done = self.remesh.wait_all();
            self.finish(done);
        }
    }

    fn collect_meshes(&mut self) {
        let done = self.remesh.poll();
        self.finish(done);
        self.stats.columns = self.columns.len();
        self.stats.chunks = self.volume.chunk_count();
        self.stats.meshed = self.meshed.len();
        self.stats.pending_meshes = self.remesh.pending();
        self.stats.quads = self.meshed.values().sum();
    }

    fn finish(&mut self, done: Vec<((i32, i32, i32), u64, MeshBuffers)>) {
        for (c, _, mesh) in done {
            self.stats.meshes_built += 1;
            let coord = IVec3::new(c.0, c.1, c.2);
            if mesh.is_empty() {
                if self.meshed.remove(&c).is_some() {
                    self.updates.remove.push(coord);
                }
            } else {
                self.meshed.insert(c, mesh.quad_count());
                self.updates.upload.push((coord, mesh));
            }
        }
    }

    /// Forgets uploaded geometry and remeshes every loaded chunk (after the
    /// renderer was recreated).
    pub fn request_full_remesh(&mut self) {
        self.meshed.clear();
        self.updates = MeshUpdates::default();
        for &(x, z) in &self.columns {
            for y in 0..HEIGHT_CHUNKS {
                self.dirty.insert((x, y, z));
            }
        }
    }

    /// Takes the geometry changes since the last call.
    pub fn take_mesh_updates(&mut self) -> MeshUpdates {
        std::mem::take(&mut self.updates)
    }

    /// Blocks until every chunk around the player is generated, lit and
    /// meshed (initial load, screenshots, benchmarks).
    pub fn settle(&mut self) {
        for _ in 0..10_000 {
            self.stream();
            self.schedule_meshing();
            let done = self.remesh.wait_all();
            self.finish(done);
            self.collect_meshes();
            let remaining = self.dirty.iter().any(|c| self.meshable(*c));
            let missing = self.planner.plan(
                EDGE as usize,
                &[Viewer {
                    position: self.player.position,
                    forward: None,
                    radius: self.view_radius,
                    vertical: Some((0, 0)),
                    keep_margin: 1,
                }],
                &self
                    .columns
                    .iter()
                    .map(|&(x, z)| IVec3::new(x, 0, z))
                    .collect::<Vec<_>>(),
            );
            if !remaining && missing.load.is_empty() && self.remesh.pending() == 0 {
                break;
            }
        }
    }

    /// Chunks visible from the camera through the connectivity graph, or
    /// `None` when the camera is outside the graph (draw everything).
    pub fn visible_chunks(
        &mut self,
        view_projection: Mat4,
        max_distance: f32,
    ) -> Option<Vec<IVec3>> {
        let eye = self.player.eye();
        let start = eye.floor().as_ivec3().div_euclid(IVec3::splat(EDGE));
        self.visibility.get(start)?;
        let frustum = ChunkFrustum::new(view_projection, EDGE as usize, eye, max_distance);
        let visible = self.visibility.traverse(
            start,
            &FrustumPolicy {
                frustum: &frustum,
                connectivity: true,
            },
        );
        self.stats.visible = visible.len();
        Some(visible)
    }

    /// Sky and block light recomputed from scratch over every loaded column
    /// (a reference for tests of incremental lighting: column loads, edits).
    #[doc(hidden)]
    pub fn reference_light(&self) -> Volume<Cube16> {
        let mut volume = self.volume.clone();
        for c in volume.chunk_coords() {
            if let Some(chunk) = volume.chunk_mut(c) {
                chunk.fill(self.ch.sky_light, 0);
                chunk.fill(self.ch.block_light, 0);
            }
        }
        let mut sky_seeds = Vec::new();
        let mut block_seeds = Vec::new();
        let mut cursor = Cursor::default();
        for &(cx, cz) in &self.columns {
            for z in cz * EDGE..(cz + 1) * EDGE {
                for x in cx * EDGE..(cx + 1) * EDGE {
                    sky_seeds.push((IVec3::new(x, WORLD_TOP - 1, z), 15));
                    for y in 0..WORLD_TOP {
                        let p = IVec3::new(x, y, z);
                        let block = volume.get_with(&mut cursor, self.ch.block, p) as usize;
                        let emitted = self.content.emission.get(block).copied().unwrap_or(0);
                        if emitted > 0 {
                            block_seeds.push((p, emitted));
                        }
                    }
                }
            }
        }
        let mut propagator = Propagator::new(QueueDiscipline::BucketFifo);
        propagator.add(&mut volume, self.ch.sky_light, &sky_seeds, &self.sky_rules);
        propagator.add(
            &mut volume,
            self.ch.block_light,
            &block_seeds,
            &self.block_rules,
        );
        volume
    }

    /// Loaded column coordinates.
    pub fn columns(&self) -> Vec<(i32, i32)> {
        self.columns.iter().copied().collect()
    }

    /// Deterministic hash of the edited world, its light and the player.
    pub fn state_hash(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut eat = |bytes: &[u8]| {
            for b in bytes {
                h = (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3);
            }
        };
        for c in self.volume.edited_chunks() {
            eat(&c.x.to_le_bytes());
            eat(&c.y.to_le_bytes());
            eat(&c.z.to_le_bytes());
            if let Some(chunk) = self.volume.chunk(c) {
                eat(&codec::encode_chunk(self.volume.schema(), chunk, true));
            }
        }
        for v in self.player.position.to_array() {
            eat(&v.to_bits().to_le_bytes());
        }
        eat(&(self.selected as u32).to_le_bytes());
        h
    }
}

fn mesh_chunk(content: &Content, job: &MeshJob) -> MeshBuffers {
    let mut out = MeshBuffers::new();
    CulledModelExtractor::default().extract(
        CulledInput {
            blocks: &job.blocks,
            orientation: None,
            block_light: Some(&job.block_light),
            sky_light: Some(&job.sky_light),
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
    out
}

/// Scripted input for headless runs: walks a loop, digs, builds a small
/// glass-and-lamp hut and flies.
pub fn autoplay_input(world: &BlockWorld) -> Input {
    let t = world.tick();
    let phase = t % 600;
    let mut input = Input {
        forward: 1.0,
        look: [0.004, 0.0],
        ..Input::default()
    };
    if phase < 20 {
        input.look[1] = -0.03; // look down
    }
    if (120..200).contains(&phase) {
        input.forward = 0.0;
        input.remove = phase.is_multiple_of(10);
    }
    if (200..320).contains(&phase) {
        input.forward = 0.0;
        input.look = [0.05, 0.0];
        let slot = [3, 5, 7, 4][((phase / 30) % 4) as usize];
        input.slot = Some(slot);
        input.place = phase.is_multiple_of(15);
    }
    if (320..330).contains(&phase) {
        input.look[1] = 0.03;
    }
    input.jump = phase % 90 == 45;
    if phase == 400 || phase == 500 {
        input.toggle_fly = true;
    }
    if (400..450).contains(&phase) {
        input.jump = true;
    }
    input
}
