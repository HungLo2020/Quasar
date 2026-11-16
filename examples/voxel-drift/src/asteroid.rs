//! A smooth, diggable asteroid: density + material channels, Surface Nets
//! meshing per LOD node with seam skirts, and trimesh colliders.
use crate::GeometryUpdate;
use quasar_jobs::JobPool;
use quasar_math::{IVec3, Vec3};
use quasar_physics::dim3::RigidPose3;
use quasar_physics::rapier3d::pipeline::PhysicsWorld as RapierWorld;
use quasar_physics::voxel3::VoxelColliders;
use quasar_voxel::{
    Brush, BrushOp, BrushShape, ChannelId, Chunk, ChunkLayout, Cube32, EditBatch, EditOptions,
    Grid, GridId, Schema, Snapshot, StorageKind, Volume, VoxelSource, quantize_density,
};
use quasar_voxel_mesh::{MeshBuffers, SmoothOptions, SurfaceNetsExtractor};
use quasar_voxel_view::{LodConfig, LodNode, LodSelector, sample_lod};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub const ASTEROID: GridId = GridId(1);
pub const CELLS: u32 = 32;
/// Chunks generated per axis (`-SPAN..SPAN`).
const SPAN: i32 = 2;
const MAX_LEVEL: u32 = 2;

fn hash3(seed: u64, x: i32, y: i32, z: i32) -> u32 {
    let mut h = seed
        ^ (x as u32 as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (y as u32 as u64).wrapping_mul(0xd6e8_feb8_6659_fd93)
        ^ (z as u32 as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
    h ^= h >> 31;
    h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^= h >> 29;
    h as u32
}

/// Trilinear value noise in `[0, 1]`.
fn noise3(seed: u64, p: Vec3, scale: f32) -> f32 {
    let f = p / scale;
    let i = f.floor();
    let t = f - i;
    let t = t * t * (Vec3::splat(3.0) - 2.0 * t);
    let (ix, iy, iz) = (i.x as i32, i.y as i32, i.z as i32);
    let v = |dx: i32, dy: i32, dz: i32| {
        (hash3(seed, ix + dx, iy + dy, iz + dz) & 0xffff) as f32 / 65535.0
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(v(0, 0, 0), v(1, 0, 0), t.x);
    let x10 = lerp(v(0, 1, 0), v(1, 1, 0), t.x);
    let x01 = lerp(v(0, 0, 1), v(1, 0, 1), t.x);
    let x11 = lerp(v(0, 1, 1), v(1, 1, 1), t.x);
    lerp(lerp(x00, x10, t.y), lerp(x01, x11, t.y), t.z)
}

#[derive(Clone)]
pub struct AsteroidSource {
    pub seed: u64,
    pub radius: f32,
    pub density: ChannelId<i8>,
    pub material: ChannelId<u8>,
}

impl AsteroidSource {
    /// Signed distance inside the surface (voxels) and material at `p`.
    pub fn sample(&self, p: Vec3) -> (f32, u8) {
        let lumpy = noise3(self.seed, p, 23.0) * 12.0
            + noise3(self.seed ^ 3, p, 9.0) * 4.0
            + noise3(self.seed ^ 7, p, 3.5) * 1.2;
        // Slightly flattened, lumpy body.
        let squashed = Vec3::new(p.x, p.y * 1.35, p.z);
        let distance = self.radius + lumpy - 8.0 - squashed.length();
        let ore = noise3(self.seed ^ 11, p, 7.0);
        let material = if p.y > self.radius * 0.55 + noise3(self.seed ^ 5, p, 11.0) * 6.0 {
            crate::content::mat::ICE as u8
        } else if ore > 0.72 {
            crate::content::mat::ORE as u8
        } else {
            crate::content::mat::ROCK as u8
        };
        (distance, material)
    }
}

impl VoxelSource<Cube32> for AsteroidSource {
    fn generate(&self, schema: &Schema, coord: IVec3, _lod: u32, chunk: &mut Chunk<Cube32>) {
        let origin = coord * CELLS as i32;
        // Skip chunks entirely outside the largest possible surface.
        let centre = (origin.as_vec3() + 16.0).length();
        if centre - 28.0 > self.radius + 10.0 {
            return;
        }
        let mut density = vec![0i8; Cube32::VOLUME];
        let mut material = vec![0u8; Cube32::VOLUME];
        for i in 0..Cube32::VOLUME {
            let p = (origin + Cube32::local(i).as_ivec3()).as_vec3();
            let (d, m) = self.sample(p);
            density[i] = quantize_density(d);
            material[i] = m;
        }
        chunk.write_all(schema, self.density, &density);
        chunk.write_all(schema, self.material, &material);
    }
}

/// Extraction inputs for one node.
struct NodeJob {
    density: Snapshot<u8>,
    material: Snapshot<u8>,
    options: SmoothOptions,
}

fn extract(job: &NodeJob) -> MeshBuffers {
    let mut mesh = MeshBuffers::new();
    SurfaceNetsExtractor::new().extract(&job.density, Some(&job.material), job.options, &mut mesh);
    mesh
}

pub struct Asteroid {
    pub grid: Grid<Cube32>,
    pub density: ChannelId<i8>,
    pub material: ChannelId<u8>,
    pub source: AsteroidSource,
    selector: LodSelector,
    roots: Vec<IVec3>,
    /// Resident LOD nodes and the seam mask they were meshed with.
    resident: BTreeMap<LodNode, u8>,
    dirty_nodes: BTreeSet<LodNode>,
    pub colliders: VoxelColliders,
    pub triangles: BTreeMap<LodNode, usize>,
    pub collider_chunks: usize,
}

impl Asteroid {
    pub fn new(seed: u64, pool: &JobPool) -> Self {
        let mut schema = Schema::new();
        let density = schema.channel("density", StorageKind::Dense, -127i8);
        let material = schema.channel("material", StorageKind::Paletted, 0u8);
        let source = AsteroidSource {
            seed,
            radius: 46.0,
            density,
            material,
        };
        let mut volume = Volume::new(schema).with_source(Arc::new(source.clone()));
        let mut coords = Vec::new();
        for y in -SPAN..SPAN {
            for z in -SPAN..SPAN {
                for x in -SPAN..SPAN {
                    coords.push(IVec3::new(x, y, z));
                }
            }
        }
        volume
            .ensure_chunks_parallel(pool, &coords)
            .expect("asteroid generation");
        let root_span = 1i32;
        let roots = (-root_span..root_span)
            .flat_map(|z| {
                (-root_span..root_span)
                    .flat_map(move |y| (-root_span..root_span).map(move |x| IVec3::new(x, y, z)))
            })
            .collect();
        Self {
            grid: Grid::new(ASTEROID, volume, 1.0),
            density,
            material,
            source,
            selector: LodSelector::new(LodConfig {
                cells: CELLS,
                max_level: MAX_LEVEL,
                split: 1.6,
                hysteresis: 0.15,
            }),
            roots,
            resident: BTreeMap::new(),
            dirty_nodes: BTreeSet::new(),
            colliders: VoxelColliders::new_static(1.0, RigidPose3::IDENTITY),
            triangles: BTreeMap::new(),
            collider_chunks: 0,
        }
    }

    /// Full-resolution collision meshes for `chunks` (meshed on the pool).
    pub fn rebuild_colliders(&mut self, world: &mut RapierWorld, pool: &JobPool, chunks: &[IVec3]) {
        let jobs: Vec<(IVec3, quasar_jobs::JobHandle<MeshBuffers>)> = chunks
            .iter()
            .filter(|c| self.grid.volume.contains(**c))
            .map(|&c| {
                let mut density = Snapshot::new();
                self.grid.volume.snapshot(self.density, c, 1, &mut density);
                let job = NodeJob {
                    density,
                    material: Snapshot::new(),
                    options: SmoothOptions::default(),
                };
                let handle = pool
                    .spawn(move |_| {
                        let mut mesh = MeshBuffers::new();
                        SurfaceNetsExtractor::new().extract::<u8>(
                            &job.density,
                            None,
                            job.options,
                            &mut mesh,
                        );
                        Ok::<_, quasar_jobs::JobError>(mesh)
                    })
                    .expect("collision job");
                (c, handle)
            })
            .collect();
        for (c, handle) in jobs {
            let mesh = handle.wait().expect("collision mesh");
            self.colliders
                .rebuild_smooth(world, c, &mesh, c * CELLS as i32, 1.0);
        }
        self.collider_chunks = self.colliders.len();
    }

    pub fn all_chunks(&self) -> Vec<IVec3> {
        self.grid.volume.chunk_coords()
    }

    /// Re-selects LOD nodes around `camera` (grid voxel space) and remeshes
    /// new, re-seamed and edited nodes on the pool.
    pub fn update_lod(&mut self, camera: Vec3, pool: &JobPool, out: &mut Vec<GeometryUpdate>) {
        let selected = self.selector.select(&self.roots, camera);
        let wanted: BTreeMap<LodNode, u8> =
            selected.iter().map(|s| (s.node, s.seam_faces)).collect();
        for node in self.resident.keys().copied().collect::<Vec<_>>() {
            if !wanted.contains_key(&node) {
                self.resident.remove(&node);
                self.triangles.remove(&node);
                out.push(GeometryUpdate::Remove {
                    grid: ASTEROID,
                    chunk: node.coord,
                    level: node.level,
                });
            }
        }
        let mut jobs = Vec::new();
        for (node, seams) in &wanted {
            let current = self.resident.get(node);
            if current == Some(seams) && !self.dirty_nodes.contains(node) {
                continue;
            }
            let mut density = Snapshot::new();
            let mut material = Snapshot::new();
            if node.level == 0 {
                self.grid
                    .volume
                    .snapshot(self.density, node.coord, 1, &mut density);
                self.grid
                    .volume
                    .snapshot(self.material, node.coord, 1, &mut material);
            } else {
                sample_lod(
                    &self.grid.volume,
                    self.density,
                    Some(self.material),
                    *node,
                    CELLS,
                    1,
                    &mut density,
                    Some(&mut material),
                );
            }
            let job = NodeJob {
                density,
                material,
                options: SmoothOptions {
                    seam_faces: *seams,
                    ..SmoothOptions::default()
                },
            };
            let handle = pool
                .spawn(move |_| Ok::<_, quasar_jobs::JobError>(extract(&job)))
                .expect("mesh job");
            jobs.push((*node, *seams, handle));
        }
        self.dirty_nodes.clear();
        for (node, seams, handle) in jobs {
            let mesh = handle.wait().expect("asteroid mesh");
            self.resident.insert(node, seams);
            self.triangles.insert(node, mesh.triangle_count());
            out.push(GeometryUpdate::Upload {
                grid: ASTEROID,
                chunk: node.coord,
                level: node.level,
                origin: node.origin(CELLS),
                scale: (1u32 << node.level) as f32,
                mesh,
            });
        }
    }

    /// Applies a spherical brush (`add` fills, otherwise digs) in voxel
    /// space. Returns the number of voxels changed.
    pub fn brush(
        &mut self,
        world: &mut RapierWorld,
        pool: &JobPool,
        centre: Vec3,
        radius: f32,
        add: bool,
        material: u8,
    ) -> usize {
        let op = if add { BrushOp::Add } else { BrushOp::Subtract };
        let mut brush = Brush::new(
            BrushShape::Sphere {
                center: centre,
                radius,
            },
            op,
            self.density,
        );
        if add {
            brush = brush.with_material(self.material, material);
        }
        let mut batch = EditBatch::new();
        batch.brush(brush);
        let result = self.grid.volume.apply(
            &batch,
            EditOptions {
                record_changes: false,
                ..EditOptions::default()
            },
        );
        // Every resident node overlapping the brush (plus a margin of one
        // node cell) must be remeshed.
        let lo = (centre - Vec3::splat(radius + 2.0)).floor().as_ivec3();
        let hi = (centre + Vec3::splat(radius + 2.0)).ceil().as_ivec3();
        for node in self.resident.keys() {
            let stride = 1 << node.level;
            let origin = node.origin(CELLS) - IVec3::splat(stride);
            let end = node.origin(CELLS) + IVec3::splat(node.size(CELLS) + stride);
            if lo.cmplt(end).all() && hi.cmpgt(origin).all() {
                self.dirty_nodes.insert(*node);
            }
        }
        self.rebuild_colliders(world, pool, &result.dirty);
        result.changed
    }

    /// Forgets uploaded nodes so the next LOD update re-emits them.
    pub fn forget_resident(&mut self) {
        self.resident.clear();
        self.triangles.clear();
    }

    pub fn total_triangles(&self) -> usize {
        self.triangles.values().sum()
    }
    pub fn node_count(&self) -> usize {
        self.resident.len()
    }
    pub fn level_counts(&self) -> [usize; 3] {
        let mut counts = [0; 3];
        for node in self.resident.keys() {
            counts[(node.level as usize).min(2)] += 1;
        }
        counts
    }
}
