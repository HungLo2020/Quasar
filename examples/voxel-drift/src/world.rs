//! The drift scene: an asteroid, a fleet of ship grids, physics, a free
//! flight camera and the tools.
use crate::GeometryUpdate;
use crate::asteroid::{ASTEROID, Asteroid};
use crate::content::{ShipContent, Tool, mat};
use crate::ship::{Fleet, SHIP_CELL, ShipHit};
use quasar_jobs::{JobPool, JobPoolConfig};
use quasar_math::{DVec3, IVec3, Vec3};
use quasar_physics::StepConfig;
use quasar_physics::dim3::{PhysicsWorld, WorldConfig3};
use quasar_voxel::{Face, GridTransform, codec};
use std::sync::Arc;

const FLY: f64 = 14.0;
const DT: f64 = 1.0 / 60.0;
pub const REACH: f64 = 60.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    pub forward: f32,
    pub strafe: f32,
    pub lift: f32,
    pub boost: bool,
    pub look: [f32; 2],
    pub primary: bool,
    pub secondary: bool,
    pub slot: Option<usize>,
    pub cycle: i32,
    pub turn: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub position: DVec3,
    pub yaw: f32,
    pub pitch: f32,
}

impl Camera {
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, -cy * cp)
    }
}

/// What the crosshair points at.
#[derive(Clone, Copy, Debug)]
pub enum Target {
    Asteroid {
        point: Vec3,
        normal: Vec3,
        distance: f64,
    },
    Ship(ShipHit),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DriftStats {
    pub ships: usize,
    pub ship_blocks: usize,
    pub splits: u64,
    pub digs: u64,
    pub fills: u64,
    pub placed: u64,
    pub removed: u64,
    pub nodes: usize,
    pub levels: [usize; 3],
    pub triangles: usize,
    pub collider_chunks: usize,
}

/// Visual-only events for the renderer (never part of the state hash).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    /// Asteroid dug (or filled) around a world point.
    Dig { point: Vec3, fill: bool },
    /// A ship block was cut out at a world point.
    BlockRemoved { point: Vec3 },
}

pub struct DriftWorld {
    pub asteroid: Asteroid,
    pub fleet: Fleet,
    pub physics: PhysicsWorld,
    pub content: Arc<ShipContent>,
    pub camera: Camera,
    pub selected: usize,
    pub turn: u8,
    pub target: Option<Target>,
    pub stats: DriftStats,
    pool: JobPool,
    updates: Vec<GeometryUpdate>,
    effects: Vec<Effect>,
    tick: u64,
}

impl DriftWorld {
    pub fn new(seed: u64, workers: usize) -> Self {
        let pool = if workers == 0 {
            JobPool::new(JobPoolConfig::default())
        } else {
            JobPool::new(JobPoolConfig::new(workers, 4096))
        }
        .expect("job pool");
        let mut physics = PhysicsWorld::new(WorldConfig3::default()).expect("physics");
        physics.set_gravity([0.0; 3]).expect("zero gravity");
        physics
            .set_step_config(StepConfig::default())
            .expect("step config");
        let mut asteroid = Asteroid::new(seed, &pool);
        let chunks = asteroid.all_chunks();
        asteroid.rebuild_colliders(physics.raw_mut(), &pool, &chunks);
        let content = Arc::new(ShipContent::new());
        let mut fleet = Fleet::new(content.clone());
        fleet.spawn_starter(physics.raw_mut(), DVec3::new(-26.0, 36.0, 66.0));
        let mut updates = vec![GeometryUpdate::Grid {
            grid: ASTEROID,
            transform: GridTransform::default(),
            cell_size: 1.0,
        }];
        fleet.update_meshes(&mut updates);
        let mut world = Self {
            asteroid,
            fleet,
            physics,
            content,
            camera: Camera {
                position: DVec3::new(-6.0, 40.0, 112.0),
                yaw: -0.12,
                pitch: -0.22,
            },
            selected: 0,
            turn: 0,
            target: None,
            stats: DriftStats::default(),
            pool,
            updates,
            effects: Vec::new(),
            tick: 0,
        };
        world.refresh();
        world
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }
    pub fn tool(&self) -> Tool {
        self.content.tools[self.selected].0
    }

    pub fn update(&mut self, input: Input) {
        self.tick += 1;
        if let Some(slot) = input.slot
            && slot < self.content.tools.len()
        {
            self.selected = slot;
        }
        if input.cycle != 0 {
            let n = self.content.tools.len() as i32;
            self.selected = (self.selected as i32 + input.cycle).rem_euclid(n) as usize;
        }
        if input.turn {
            self.turn = (self.turn + 1) % 4;
        }
        let c = &mut self.camera;
        c.yaw += input.look[0];
        c.pitch = (c.pitch + input.look[1]).clamp(-1.55, 1.55);
        let forward = c.forward().as_dvec3();
        let right = DVec3::new(f64::from(c.yaw.cos()), 0.0, f64::from(c.yaw.sin()));
        let speed = if input.boost { FLY * 3.0 } else { FLY };
        let wish = forward * f64::from(input.forward)
            + right * f64::from(input.strafe)
            + DVec3::Y * f64::from(input.lift);
        c.position += wish.clamp_length_max(1.0) * speed * DT;
        self.target = self.pick();
        if input.primary {
            self.primary();
        }
        if input.secondary {
            self.secondary();
        }
        self.physics.step_fixed();
        self.fleet.sync_poses(self.physics.raw());
        self.refresh();
    }

    fn refresh(&mut self) {
        let camera = self.camera.position.as_vec3();
        self.asteroid
            .update_lod(camera, &self.pool, &mut self.updates);
        self.fleet.update_meshes(&mut self.updates);
        let block = self.fleet.ch.block;
        self.stats.ships = self.fleet.ships.len();
        self.stats.ship_blocks = self.fleet.ships.iter().map(|s| s.block_count(block)).sum();
        self.stats.splits = self.fleet.splits;
        self.stats.nodes = self.asteroid.node_count();
        self.stats.levels = self.asteroid.level_counts();
        self.stats.triangles = self.asteroid.total_triangles();
        self.stats.collider_chunks = self.asteroid.collider_chunks;
    }

    fn pick(&self) -> Option<Target> {
        let origin = self.camera.position;
        let direction = self.camera.forward();
        let rock = self
            .asteroid
            .grid
            .volume
            .raycast_density(
                self.asteroid.density,
                origin.as_vec3(),
                direction,
                REACH as f32,
                0.25,
            )
            .map(|(hit, normal)| Target::Asteroid {
                point: hit.point,
                normal,
                distance: f64::from(hit.t),
            });
        let ship = self
            .fleet
            .raycast(origin, direction.as_dvec3(), REACH)
            .map(Target::Ship);
        match (rock, ship) {
            (Some(Target::Asteroid { distance: a, .. }), Some(Target::Ship(s)))
                if s.distance < a =>
            {
                Some(Target::Ship(s))
            }
            (Some(r), _) => Some(r),
            (None, s) => s,
        }
    }

    fn primary(&mut self) {
        match (self.tool(), self.target) {
            (Tool::Drill, Some(Target::Asteroid { point, .. })) => self.dig(point, false),
            (Tool::Filler, Some(Target::Asteroid { point, normal, .. })) => {
                self.dig(point + normal * 2.0, true)
            }
            (Tool::Block(block), Some(Target::Ship(hit))) => {
                if let Some(face) = hit.hit.face {
                    let cell = hit.hit.adjacent();
                    if self.fleet.place(
                        hit.ship,
                        cell,
                        face,
                        block,
                        self.turn,
                        self.physics.raw_mut(),
                    ) {
                        self.stats.placed += 1;
                    }
                }
            }
            _ => {}
        }
    }

    fn secondary(&mut self) {
        if let Some(Target::Ship(hit)) = self.target {
            let point = self.ship_cell_world(hit.ship, hit.hit.voxel).as_vec3();
            if self
                .fleet
                .remove(hit.ship, hit.hit.voxel, self.physics.raw_mut())
            {
                self.stats.removed += 1;
                self.effects.push(Effect::BlockRemoved { point });
            }
        }
    }

    /// Visual events since the last call.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// Digs (or fills) a 3.5 m sphere of asteroid at `point` (voxel space).
    pub fn dig(&mut self, point: Vec3, fill: bool) {
        let changed = self.asteroid.brush(
            self.physics.raw_mut(),
            &self.pool,
            point,
            3.5,
            fill,
            mat::ICE as u8,
        );
        if changed > 0 {
            self.effects.push(Effect::Dig { point, fill });
            if fill {
                self.stats.fills += 1;
            } else {
                self.stats.digs += 1;
            }
        }
    }

    /// Removes a ship block by grid cell (scripted runs and tests).
    pub fn remove_ship_block(&mut self, ship: usize, cell: IVec3) -> bool {
        let point = self.ship_cell_world(ship, cell).as_vec3();
        let done = self.fleet.remove(ship, cell, self.physics.raw_mut());
        if done {
            self.effects.push(Effect::BlockRemoved { point });
        }
        self.stats.removed += u64::from(done);
        done
    }

    /// Places a ship block by grid cell (scripted runs and tests).
    pub fn place_ship_block(
        &mut self,
        ship: usize,
        cell: IVec3,
        mount: Face,
        block: u16,
        turn: u8,
    ) -> bool {
        let done = self
            .fleet
            .place(ship, cell, mount, block, turn, self.physics.raw_mut());
        self.stats.placed += u64::from(done);
        done
    }

    pub fn take_updates(&mut self) -> Vec<GeometryUpdate> {
        std::mem::take(&mut self.updates)
    }

    /// Re-emits every grid and mesh (after the renderer was recreated).
    pub fn request_full_upload(&mut self) {
        self.updates.clear();
        self.updates.push(GeometryUpdate::Grid {
            grid: ASTEROID,
            transform: GridTransform::default(),
            cell_size: 1.0,
        });
        self.asteroid.forget_resident();
        self.fleet.mark_all_dirty();
        self.refresh();
    }

    /// Deterministic hash of terrain edits, ship contents and poses.
    pub fn state_hash(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut eat = |bytes: &[u8]| {
            for b in bytes {
                h = (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3);
            }
        };
        let volume = &self.asteroid.grid.volume;
        for c in volume.edited_chunks() {
            eat(&codec::encode_chunk(
                volume.schema(),
                volume.chunk(c).expect("edited chunk"),
                true,
            ));
        }
        for ship in &self.fleet.ships {
            let v = &ship.grid.volume;
            for c in v.chunk_coords() {
                eat(&codec::encode_chunk(
                    v.schema(),
                    v.chunk(c).expect("chunk"),
                    true,
                ));
            }
            let t = ship.grid.transform;
            for x in t
                .position
                .to_array()
                .iter()
                .chain(t.rotation.to_array().iter())
            {
                eat(&((x * 1e3).round() as i64).to_le_bytes());
            }
        }
        for x in self.camera.position.to_array() {
            eat(&x.to_bits().to_le_bytes());
        }
        h
    }

    /// Points the camera at a world position.
    pub fn look_at(&mut self, point: DVec3) {
        let d = (point - self.camera.position).normalize_or_zero();
        self.camera.yaw = d.x.atan2(-d.z) as f32;
        self.camera.pitch = d.y.clamp(-1.0, 1.0).asin() as f32;
    }

    /// World position of a ship grid cell's centre.
    pub fn ship_cell_world(&self, ship: usize, cell: IVec3) -> DVec3 {
        self.fleet.ships[ship]
            .grid
            .voxel_to_world(cell.as_dvec3() + 0.5)
    }
}

/// Scripted input: fly in, drill and fill the asteroid, build on the ship,
/// then cut its neck so the two halves drift apart.
pub fn autoplay(world: &mut DriftWorld) -> Input {
    let t = world.tick();
    let mut input = Input::default();
    match t {
        0..=89 => input.forward = 1.0,
        90..=239 => {
            input.slot = Some(0);
            world.look_at(DVec3::new(((t / 25) as f64 - 6.0) * 7.0, 10.0, 0.0));
            input.primary = t.is_multiple_of(25);
        }
        240..=299 => {
            input.slot = Some(1);
            world.look_at(DVec3::new(-12.0, -8.0, 0.0));
            input.primary = t.is_multiple_of(30);
        }
        300 => {
            // Extend the ship's rear hull and add a window.
            let armor = world.content.blocks.armor;
            let window = world.content.blocks.window;
            world.place_ship_block(0, IVec3::new(2, 0, 2), Face::PosX, armor, 0);
            world.place_ship_block(0, IVec3::new(2, 1, 2), Face::PosY, window, 0);
        }
        360 => {
            // Cut the neck: the front half separates and drifts away.
            world.remove_ship_block(0, IVec3::new(0, 0, 5));
        }
        _ => {
            // Back off and watch the halves drift apart.
            input.forward = -0.25;
            input.lift = 0.2;
            let centre = world
                .fleet
                .ships
                .iter()
                .map(|s| s.grid.voxel_to_world(DVec3::new(0.5, 1.0, 5.0)))
                .sum::<DVec3>()
                / world.fleet.ships.len().max(1) as f64;
            world.look_at(centre);
        }
    }
    input
}

impl DriftWorld {
    pub fn ship_cell_size() -> f64 {
        SHIP_CELL
    }
}
