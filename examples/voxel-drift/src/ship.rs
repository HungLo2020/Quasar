//! Ships: dynamic block grids (2.5 m cells) with oriented, multi-cell and
//! sloped blocks that split into separate bodies when cut apart.
use crate::GeometryUpdate;
use crate::content::ShipContent;
use quasar_math::{DQuat, DVec3, IVec3, Vec3};
use quasar_physics::dim3::RigidPose3;
use quasar_physics::rapier3d::pipeline::PhysicsWorld as RapierWorld;
use quasar_physics::voxel3::VoxelColliders;
use quasar_voxel::{
    AIR, BlockChannels, ChannelId, Cube16, EditBatch, EditOptions, Face, Grid, GridId, GridKind,
    GridTransform, Orientation, RayHit, Schema, Snapshot, StorageKind, TieBreak, Volume,
    split_check,
};
use quasar_voxel_mesh::{
    CatalogFaces, CatalogLayers, CatalogModels, CulledInput, CulledModelExtractor, MeshBuffers,
    VertexShade,
};
use std::collections::BTreeSet;
use std::sync::Arc;

pub const SHIP_CELL: f64 = 2.5;

pub struct Ship {
    pub grid: Grid<Cube16>,
    pub colliders: VoxelColliders,
    meshed: BTreeSet<(i32, i32, i32)>,
    dirty: BTreeSet<(i32, i32, i32)>,
}

impl Ship {
    pub fn block_count(&self, block: ChannelId<u16>) -> usize {
        let mut n = 0;
        for c in self.grid.volume.chunk_coords() {
            if let Some(chunk) = self.grid.volume.chunk(c) {
                let storage = chunk.storage(block);
                if storage.uniform() == Some(AIR) {
                    continue;
                }
                for i in 0..4096 {
                    n += usize::from(chunk.get_index(block, i) != AIR);
                }
            }
        }
        n
    }
}

/// A ship-grid ray hit.
#[derive(Clone, Copy, Debug)]
pub struct ShipHit {
    pub ship: usize,
    pub hit: RayHit,
    /// World distance.
    pub distance: f64,
}

pub struct Fleet {
    pub ships: Vec<Ship>,
    pub ch: BlockChannels,
    pub content: Arc<ShipContent>,
    schema: Arc<Schema>,
    next_id: u64,
    pub splits: u64,
    extractor: CulledModelExtractor,
}

fn key(c: IVec3) -> (i32, i32, i32) {
    (c.x, c.y, c.z)
}

fn pose_of(transform: &GridTransform) -> RigidPose3 {
    let q = transform.rotation.as_quat();
    RigidPose3 {
        translation: transform.position.as_vec3().to_array(),
        rotation: [q.x, q.y, q.z, q.w],
    }
}

impl Fleet {
    pub fn new(content: Arc<ShipContent>) -> Self {
        let mut schema = Schema::new();
        let block = schema.channel("block", StorageKind::Paletted, AIR);
        let orientation = schema.channel("orientation", StorageKind::Paletted, 0u8);
        let part = schema.channel("part", StorageKind::Sparse, 0u32);
        Self {
            ships: Vec::new(),
            ch: BlockChannels {
                block,
                orientation,
                part,
            },
            content,
            schema: Arc::new(schema),
            next_id: 100,
            splits: 0,
            extractor: CulledModelExtractor::default(),
        }
    }

    fn allocate(&mut self) -> GridId {
        self.next_id += 1;
        GridId(self.next_id)
    }

    fn add_ship(
        &mut self,
        world: &mut RapierWorld,
        volume: Volume<Cube16>,
        transform: GridTransform,
        linvel: Vec3,
        angvel: Vec3,
    ) -> usize {
        let id = self.allocate();
        let mut colliders =
            VoxelColliders::dynamic_body(world, SHIP_CELL as f32, pose_of(&transform))
                .expect("ship body");
        if let Some(body) = colliders.body().and_then(|b| world.bodies.get_mut(b))
            as Option<&mut quasar_physics::rapier3d::dynamics::RigidBody>
        {
            body.set_linvel(linvel, true);
            body.set_angvel(angvel, true);
        }
        let dirty: BTreeSet<_> = volume.chunk_coords().into_iter().map(key).collect();
        let block = self.ch.block;
        for c in &dirty {
            colliders.rebuild_cubic(world, &volume, block, IVec3::new(c.0, c.1, c.2), |b| {
                b != AIR
            });
        }
        let grid = Grid::new(id, volume, SHIP_CELL)
            .with_transform(transform)
            .with_kind(GridKind::Dynamic);
        self.ships.push(Ship {
            grid,
            colliders,
            meshed: BTreeSet::new(),
            dirty,
        });
        self.ships.len() - 1
    }

    /// Builds the starter ship: a hull with a narrow neck, sloped nose,
    /// windows, a light and multi-cell thrusters.
    pub fn spawn_starter(&mut self, world: &mut RapierWorld, position: DVec3) -> usize {
        let b = self.content.blocks;
        let mut volume = Volume::<Cube16>::with_schema(self.schema.clone());
        let mut batch = EditBatch::new();
        let catalog = &self.content.catalog;
        let ch = self.ch;
        let place =
            |batch: &mut EditBatch, volume: &Volume<Cube16>, id: u16, o: Orientation, p: IVec3| {
                catalog
                    .place(batch, volume, ch, id, o, p)
                    .expect("starter ship placement");
            };
        // Rear hull: z 0..5, 3 wide, 2 tall.
        for z in 0..5 {
            for x in -1..=1 {
                for y in 0..2 {
                    let id = if y == 1 && x == 0 { b.dark } else { b.armor };
                    place(
                        &mut batch,
                        &volume,
                        id,
                        Orientation::IDENTITY,
                        IVec3::new(x, y, z),
                    );
                }
            }
        }
        // Neck: a single block at z = 5.
        place(
            &mut batch,
            &volume,
            b.dark,
            Orientation::IDENTITY,
            IVec3::new(0, 0, 5),
        );
        // Front hull: z 6..10 with windows on top and a sloped nose.
        for z in 6..10 {
            for x in -1..=1 {
                place(
                    &mut batch,
                    &volume,
                    b.armor,
                    Orientation::IDENTITY,
                    IVec3::new(x, 0, z),
                );
                let top = if z >= 7 && x == 0 { b.window } else { b.armor };
                place(
                    &mut batch,
                    &volume,
                    top,
                    Orientation::IDENTITY,
                    IVec3::new(x, 1, z),
                );
            }
        }
        // The slope's full back face (local -Z) mounts on the hull; the
        // wedge falls away towards the nose tip at +Z.
        let nose = Orientation::IDENTITY;
        for x in -1..=1 {
            place(&mut batch, &volume, b.slope, nose, IVec3::new(x, 0, 10));
        }
        place(
            &mut batch,
            &volume,
            b.light,
            Orientation::IDENTITY,
            IVec3::new(0, 2, 2),
        );
        // Thrusters (two cells long) at the back, pointing -Z.
        let back = Orientation::from_up_forward(Face::PosY, Face::NegZ).expect("perpendicular");
        for x in [-1, 1] {
            place(&mut batch, &volume, b.thruster, back, IVec3::new(x, 0, -1));
        }
        volume.apply(&batch, EditOptions::default());
        let transform = GridTransform::new(position, DQuat::from_rotation_y(0.4));
        self.add_ship(
            world,
            volume,
            transform,
            Vec3::ZERO,
            Vec3::new(0.0, 0.02, 0.0),
        )
    }

    /// Copies body poses into grid transforms.
    pub fn sync_poses(&mut self, world: &RapierWorld) {
        for ship in &mut self.ships {
            let pose = ship.colliders.grid_pose(world);
            let r = pose.rotation;
            ship.grid.transform = GridTransform::new(
                DVec3::from(pose.translation.map(f64::from)),
                DQuat::from_xyzw(
                    f64::from(r[0]),
                    f64::from(r[1]),
                    f64::from(r[2]),
                    f64::from(r[3]),
                )
                .normalize(),
            );
        }
    }

    pub fn raycast(&self, origin: DVec3, direction: DVec3, max_distance: f64) -> Option<ShipHit> {
        let mut best: Option<ShipHit> = None;
        for (i, ship) in self.ships.iter().enumerate() {
            let o = ship.grid.world_to_voxel(origin).as_vec3();
            let d = ship
                .grid
                .world_dir_to_voxel(direction)
                .normalize()
                .as_vec3();
            let block = self.ch.block;
            if let Some(hit) = ship.grid.volume.raycast(
                block,
                o,
                d,
                (max_distance / SHIP_CELL) as f32,
                TieBreak::default(),
                |_, b| b != AIR,
            ) {
                let distance = f64::from(hit.t) * SHIP_CELL;
                if best.is_none_or(|b| distance < b.distance) {
                    best = Some(ShipHit {
                        ship: i,
                        hit,
                        distance,
                    });
                }
            }
        }
        best
    }

    /// Places `block` against a hit face; the block's up faces away from the
    /// face it is mounted on, `turn` picks one of four headings.
    pub fn place(
        &mut self,
        ship: usize,
        cell: IVec3,
        mount: Face,
        block: u16,
        turn: u8,
        world: &mut RapierWorld,
    ) -> bool {
        let forwards: Vec<Face> = Face::ALL
            .into_iter()
            .filter(|f| f.axis() != mount.axis())
            .collect();
        let Some(orientation) =
            Orientation::from_up_forward(mount, forwards[turn as usize % forwards.len()])
        else {
            return false;
        };
        let Some(s) = self.ships.get_mut(ship) else {
            return false;
        };
        let mut batch = EditBatch::new();
        if self
            .content
            .catalog
            .place(
                &mut batch,
                &s.grid.volume,
                self.ch,
                block,
                orientation,
                cell,
            )
            .is_err()
        {
            return false;
        }
        let result = s.grid.volume.apply(&batch, EditOptions::default());
        for c in &result.dirty {
            s.dirty.insert(key(*c));
            s.colliders
                .rebuild_cubic(world, &s.grid.volume, self.ch.block, *c, |b| b != AIR);
        }
        true
    }

    /// Removes the block at `cell`; if the ship falls apart, every piece but
    /// the largest becomes a new ship drifting away from the cut.
    pub fn remove(&mut self, ship: usize, cell: IVec3, world: &mut RapierWorld) -> bool {
        let content = self.content.clone();
        let ch = self.ch;
        let Some(s) = self.ships.get_mut(ship) else {
            return false;
        };
        let Some(anchor) = content.catalog.anchor_of(&s.grid.volume, ch, cell) else {
            return false;
        };
        let id = s.grid.volume.get(ch.block, anchor);
        let orientation = Orientation(s.grid.volume.get(ch.orientation, anchor));
        let removed = content.catalog.footprint(id, orientation, anchor);
        let mut batch = EditBatch::new();
        content.catalog.remove(&mut batch, &s.grid.volume, ch, cell);
        let result = s.grid.volume.apply(&batch, EditOptions::default());
        for c in &result.dirty {
            s.dirty.insert(key(*c));
            s.colliders
                .rebuild_cubic(world, &s.grid.volume, ch.block, *c, |b| b != AIR);
        }
        let solid = |v: &Volume<Cube16>, p: IVec3| v.get(ch.block, p) != AIR;
        let connected =
            |v: &Volume<Cube16>, p: IVec3, f: Face| content.catalog.connected(v, ch, p, f);
        let Some(split) = split_check(&s.grid.volume, &removed, 1 << 20, solid, connected) else {
            return true;
        };
        let (linvel, angvel) = s.colliders.body().and_then(|b| world.bodies.get(b)).map_or(
            (Vec3::ZERO, Vec3::ZERO),
            |b: &quasar_physics::rapier3d::dynamics::RigidBody| (b.linvel(), b.angvel()),
        );
        let centroid = |cells: &[IVec3]| {
            cells.iter().map(|c| c.as_dvec3() + 0.5).sum::<DVec3>() / cells.len() as f64
        };
        let main_centre = centroid(&split.groups[0]);
        let transform = s.grid.transform;
        let mut pieces = Vec::new();
        for group in &split.groups[1..] {
            let ship = &mut self.ships[ship];
            let piece = ship.grid.volume.extract_cells(group);
            for c in group {
                ship.dirty
                    .insert(key(<Cube16 as quasar_voxel::ChunkLayout>::chunk_of(*c)));
            }
            // Push the piece away from the main body along the cut.
            let away = transform.rotation * (centroid(group) - main_centre).normalize_or_zero();
            pieces.push((
                piece,
                linvel + away.as_vec3() * 1.2,
                angvel + Vec3::new(0.05, 0.11, -0.04),
            ));
        }
        let ship_index = ship;
        {
            let ship = &mut self.ships[ship_index];
            let dirty: Vec<IVec3> = ship
                .dirty
                .iter()
                .map(|c| IVec3::new(c.0, c.1, c.2))
                .collect();
            for c in dirty {
                ship.colliders
                    .rebuild_cubic(world, &ship.grid.volume, ch.block, c, |b| b != AIR);
            }
        }
        for (piece, linvel, angvel) in pieces {
            self.add_ship(world, piece, transform, linvel, angvel);
            self.splits += 1;
        }
        true
    }

    /// Meshes dirty chunks of every ship and reports grid transforms.
    pub fn update_meshes(&mut self, out: &mut Vec<GeometryUpdate>) {
        let content = self.content.clone();
        for ship in &mut self.ships {
            out.push(GeometryUpdate::Grid {
                grid: ship.grid.id,
                transform: ship.grid.transform,
                cell_size: SHIP_CELL,
            });
            let dirty: Vec<_> = std::mem::take(&mut ship.dirty).into_iter().collect();
            for c in dirty {
                let coord = IVec3::new(c.0, c.1, c.2);
                let mut blocks = Snapshot::new();
                let mut orientation = Snapshot::new();
                ship.grid
                    .volume
                    .snapshot(self.ch.block, coord, 1, &mut blocks);
                ship.grid
                    .volume
                    .snapshot(self.ch.orientation, coord, 1, &mut orientation);
                let mut mesh = MeshBuffers::new();
                self.extractor.extract(
                    CulledInput {
                        blocks: &blocks,
                        orientation: Some(&orientation),
                        block_light: None,
                        sky_light: None,
                        opaque: &content.opaque,
                    },
                    &CatalogModels {
                        catalog: &content.catalog,
                        library: &content.library,
                    },
                    &CatalogFaces(&content.catalog),
                    &VertexShade {
                        ao: true,
                        smooth_light: false,
                    },
                    &CatalogLayers(&content.catalog),
                    |_| true,
                    &mut mesh,
                );
                if mesh.is_empty() {
                    if ship.meshed.remove(&c) {
                        out.push(GeometryUpdate::Remove {
                            grid: ship.grid.id,
                            chunk: coord,
                            level: 0,
                        });
                    }
                } else {
                    ship.meshed.insert(c);
                    out.push(GeometryUpdate::Upload {
                        grid: ship.grid.id,
                        chunk: coord,
                        level: 0,
                        origin: coord * 16,
                        scale: 1.0,
                        mesh,
                    });
                }
            }
        }
    }

    /// Re-meshes everything (after the renderer was recreated).
    pub fn mark_all_dirty(&mut self) {
        for ship in &mut self.ships {
            ship.meshed.clear();
            ship.dirty = ship
                .grid
                .volume
                .chunk_coords()
                .into_iter()
                .map(key)
                .collect();
        }
    }
}
