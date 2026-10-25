//! Deterministic cellular sand plus Rapier obstacles and one dynamic ball.
use quasar_physics::dim2::{PhysicsWorld, StepConfig, WorldConfig};
use quasar_physics::rapier2d::math::Vector;
use quasar_physics::rapier2d::prelude::{ColliderBuilder, RigidBodyBuilder};
use quasar_sim::StateHasher;
use quasar_spatial::{Aabb, GridConfig, SpatialGrid};

pub const GRID_WIDTH: usize = 240;
pub const GRID_HEIGHT: usize = 135;
pub const BALL_KEY: u32 = u32::MAX;
pub const GRAVITY: f32 = 90.0;
pub const BALL_RADIUS: f32 = 3.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Cell {
    Empty = 0,
    Sand = 1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tool {
    Sand,
    Rock,
    Erase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObstacleKind {
    Bound,
    Rock,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SandInput {
    pub pointer: [f32; 2],
    pub pointer_valid: bool,
    pub primary_held: bool,
    pub primary_pressed: bool,
    pub secondary_held: bool,
    pub reset: bool,
    pub tool: Tool,
    pub brush_radius: f32,
}

impl Default for SandInput {
    fn default() -> Self {
        Self {
            pointer: [0.0, 0.0],
            pointer_valid: false,
            primary_held: false,
            primary_pressed: false,
            secondary_held: false,
            reset: false,
            tool: Tool::Sand,
            brush_radius: 4.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SandStats {
    pub sand: usize,
    pub obstacles: usize,
    pub ball: [f32; 2],
    pub ball_velocity: [f32; 2],
    pub nearby_obstacles: usize,
    pub physics_steps: u64,
    pub collision_events: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct Obstacle {
    pub bounds: Aabb<2>,
    pub kind: ObstacleKind,
    pub collider: quasar_physics::rapier2d::geometry::ColliderHandle,
}

pub struct SandWorld {
    seed: u64,
    grid: Vec<Cell>,
    solid: Vec<bool>,
    obstacles: std::collections::BTreeMap<u32, Obstacle>,
    obstacle_index: SpatialGrid<2, u32>,
    physics: PhysicsWorld,
    ball_body: quasar_physics::rapier2d::dynamics::RigidBodyHandle,
    ball_spawn: [f32; 2],
    next_obstacle: u32,
    tick: u64,
    collision_events: u64,
}

impl SandWorld {
    pub fn new(seed: u64) -> Self {
        let physics = PhysicsWorld::new(
            WorldConfig::new(
                [0.0, GRAVITY],
                StepConfig::new(1.0 / 60.0, 0.25, 8).expect("sand step configuration"),
            )
            .expect("sand gravity"),
        )
        .expect("sand physics world");
        let mut world = Self {
            seed,
            grid: vec![Cell::Empty; GRID_WIDTH * GRID_HEIGHT],
            solid: vec![false; GRID_WIDTH * GRID_HEIGHT],
            obstacles: std::collections::BTreeMap::new(),
            obstacle_index: SpatialGrid::new(
                GridConfig::new(8.0, 256, 100_000).expect("spatial grid configuration"),
            )
            .expect("sand spatial configuration"),
            physics,
            ball_body: quasar_physics::rapier2d::dynamics::RigidBodyHandle::invalid(),
            ball_spawn: [GRID_WIDTH as f32 * 0.5, 20.0],
            next_obstacle: 1,
            tick: 0,
            collision_events: 0,
        };
        world.insert_obstacle(
            Aabb::new(
                [0.0, GRID_HEIGHT as f64 - 3.0],
                [GRID_WIDTH as f64, GRID_HEIGHT as f64],
            )
            .expect("ground bounds"),
            ObstacleKind::Bound,
        );
        world.insert_obstacle(
            Aabb::new([0.0, 0.0], [3.0, GRID_HEIGHT as f64]).expect("left wall"),
            ObstacleKind::Bound,
        );
        world.insert_obstacle(
            Aabb::new(
                [GRID_WIDTH as f64 - 3.0, 0.0],
                [GRID_WIDTH as f64, GRID_HEIGHT as f64],
            )
            .expect("right wall"),
            ObstacleKind::Bound,
        );
        world.insert_obstacle(
            Aabb::new([36.0, 102.0], [70.0, 109.0]).expect("ramp"),
            ObstacleKind::Rock,
        );
        world.insert_obstacle(
            Aabb::new([150.0, 86.0], [194.0, 93.0]).expect("high ramp"),
            ObstacleKind::Rock,
        );
        world.insert_obstacle(
            Aabb::new([102.0, 116.0], [136.0, 123.0]).expect("low ramp"),
            ObstacleKind::Rock,
        );

        let (body, _) = world.physics.insert(
            RigidBodyBuilder::dynamic()
                .translation(Vector::new(world.ball_spawn[0], world.ball_spawn[1]))
                .linvel(Vector::new(24.0, 0.0)),
            ColliderBuilder::ball(BALL_RADIUS)
                .restitution(0.72)
                .friction(0.8),
        );
        world.ball_body = body;
        world.update_ball_index();

        let mut rng = seed | 1;
        for _ in 0..900 {
            rng = rng
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let x = ((rng >> 20) as usize % 170) + 28;
            let y = ((rng >> 44) as usize % 20) + 4;
            if !world.solid[y * GRID_WIDTH + x] {
                world.grid[y * GRID_WIDTH + x] = Cell::Sand;
            }
        }
        world
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn cell(&self, x: usize, y: usize) -> Option<Cell> {
        (x < GRID_WIDTH && y < GRID_HEIGHT).then(|| self.grid[y * GRID_WIDTH + x])
    }

    pub fn clear_sand(&mut self) {
        self.grid.fill(Cell::Empty);
    }

    pub fn is_solid(&self, x: usize, y: usize) -> bool {
        x < GRID_WIDTH && y < GRID_HEIGHT && self.solid[y * GRID_WIDTH + x]
    }

    pub fn obstacles(&self) -> impl Iterator<Item = (&u32, &Obstacle)> {
        self.obstacles.iter()
    }

    pub fn ball_position(&self) -> [f32; 2] {
        let position = self.physics.bodies[self.ball_body].translation();
        [position.x, position.y]
    }

    pub fn ball_velocity(&self) -> [f32; 2] {
        let velocity = self.physics.bodies[self.ball_body].linvel();
        [velocity.x, velocity.y]
    }

    pub fn stats(&self) -> SandStats {
        let sand = self.grid.iter().filter(|cell| **cell == Cell::Sand).count();
        let ball = self.ball_position();
        let nearby_obstacles = self
            .obstacle_index
            .query_radius([f64::from(ball[0]), f64::from(ball[1])], 30.0)
            .into_iter()
            .filter(|key| *key != BALL_KEY)
            .count();
        SandStats {
            sand,
            obstacles: self.obstacles.len(),
            ball,
            ball_velocity: self.ball_velocity(),
            nearby_obstacles,
            physics_steps: self.physics.fixed_steps(),
            collision_events: self.collision_events,
        }
    }

    pub fn state_hash(&self) -> u64 {
        let mut hasher = StateHasher::new();
        hasher.write_u64(self.tick);
        hasher.write_u64(self.seed);
        for cell in &self.grid {
            hasher.write_u8(*cell as u8);
        }
        let ball = self.ball_position();
        let velocity = self.ball_velocity();
        hasher.write_f32_bits(ball[0]);
        hasher.write_f32_bits(ball[1]);
        hasher.write_f32_bits(velocity[0]);
        hasher.write_f32_bits(velocity[1]);
        for (id, obstacle) in &self.obstacles {
            hasher.write_u32(*id);
            for value in obstacle.bounds.min.iter().chain(&obstacle.bounds.max) {
                hasher.write_f64_bits(*value);
            }
        }
        hasher.finish().value()
    }

    pub fn update(&mut self, input: SandInput) {
        if input.reset {
            *self = Self::new(self.seed);
            return;
        }
        if input.pointer_valid {
            match input.tool {
                Tool::Sand => {
                    if input.primary_held {
                        self.paint_sand(input.pointer, input.brush_radius);
                    }
                }
                Tool::Rock => {
                    if input.primary_pressed {
                        self.place_rock_at(input.pointer);
                    }
                }
                Tool::Erase => {
                    if input.primary_pressed {
                        self.erase(input.pointer, 6.0);
                    }
                }
            }
            if input.secondary_held {
                self.paint_erase(input.pointer, input.brush_radius);
            }
        }
        self.physics.step_fixed();
        self.collision_events += self.physics.drain_events().collisions.len() as u64;
        self.update_ball_index();
        self.simulate_sand();
        self.tick = self.tick.saturating_add(1);
    }

    pub fn paint_sand(&mut self, center: [f32; 2], radius: f32) {
        let cx = center[0].round() as isize;
        let cy = center[1].round() as isize;
        let radius = radius.max(0.0);
        let limit = radius.ceil() as isize;
        for dy in -limit..=limit {
            for dx in -limit..=limit {
                if (dx * dx + dy * dy) as f32 > radius * radius {
                    continue;
                }
                let x = cx + dx;
                let y = cy + dy;
                if x <= 0 || y <= 0 || x >= GRID_WIDTH as isize - 1 || y >= GRID_HEIGHT as isize - 1
                {
                    continue;
                }
                let index = y as usize * GRID_WIDTH + x as usize;
                if !self.solid[index] {
                    self.grid[index] = Cell::Sand;
                }
            }
        }
    }

    pub fn paint_erase(&mut self, center: [f32; 2], radius: f32) {
        let index = self
            .obstacle_index
            .query_point([f64::from(center[0]), f64::from(center[1])]);
        for id in index {
            if id != BALL_KEY {
                self.remove_obstacle(id);
            }
        }
        let cx = center[0].round() as isize;
        let cy = center[1].round() as isize;
        let limit = radius.ceil() as isize;
        for dy in -limit..=limit {
            for dx in -limit..=limit {
                if (dx * dx + dy * dy) as f32 > radius * radius {
                    continue;
                }
                let x = cx + dx;
                let y = cy + dy;
                if x <= 0 || y <= 0 || x >= GRID_WIDTH as isize - 1 || y >= GRID_HEIGHT as isize - 1
                {
                    continue;
                }
                self.grid[y as usize * GRID_WIDTH + x as usize] = Cell::Empty;
            }
        }
    }

    pub fn erase(&mut self, center: [f32; 2], radius: f32) {
        self.paint_erase(center, radius);
    }

    pub fn place_rock_at(&mut self, center: [f32; 2]) -> Option<u32> {
        let half_width = 6.0;
        let half_height = 2.5;
        let bounds = Aabb::new(
            [
                f64::from(center[0] - half_width),
                f64::from(center[1] - half_height),
            ],
            [
                f64::from(center[0] + half_width),
                f64::from(center[1] + half_height),
            ],
        )
        .ok()?;
        self.place_rock(bounds)
    }

    pub fn place_rock(&mut self, bounds: Aabb<2>) -> Option<u32> {
        if bounds.min[0] < 3.0
            || bounds.min[1] < 3.0
            || bounds.max[0] > f64::from(GRID_WIDTH as u32 - 3)
            || bounds.max[1] > f64::from(GRID_HEIGHT as u32 - 3)
        {
            return None;
        }
        let overlaps = self
            .obstacle_index
            .query(bounds)
            .into_iter()
            .any(|id| id != BALL_KEY);
        if overlaps {
            return None;
        }
        Some(self.insert_obstacle(bounds, ObstacleKind::Rock))
    }

    fn insert_obstacle(&mut self, bounds: Aabb<2>, kind: ObstacleKind) -> u32 {
        let id = self.next_obstacle;
        self.next_obstacle = self.next_obstacle.saturating_add(1);
        let extents = bounds.extents();
        let center = bounds.center();
        let collider =
            ColliderBuilder::cuboid((extents[0] * 0.5) as f32, (extents[1] * 0.5) as f32)
                .translation(Vector::new(center[0] as f32, center[1] as f32))
                .build();
        let collider = self.physics.insert_collider(collider, None);
        self.set_solid(&bounds, true);
        self.obstacle_index.insert(id, bounds);
        self.obstacles.insert(
            id,
            Obstacle {
                bounds,
                kind,
                collider,
            },
        );
        id
    }

    pub fn remove_obstacle(&mut self, id: u32) -> bool {
        let Some(obstacle) = self.obstacles.remove(&id) else {
            return false;
        };
        self.set_solid(&obstacle.bounds, false);
        self.obstacle_index.remove(id);
        self.physics.remove_collider(obstacle.collider);
        true
    }

    fn set_solid(&mut self, bounds: &Aabb<2>, value: bool) {
        let x0 = bounds.min[0].floor().max(0.0) as usize;
        let y0 = bounds.min[1].floor().max(0.0) as usize;
        let x1 = (bounds.max[0].ceil() as usize).min(GRID_WIDTH);
        let y1 = (bounds.max[1].ceil() as usize).min(GRID_HEIGHT);
        for y in y0..y1 {
            for x in x0..x1 {
                let index = y * GRID_WIDTH + x;
                self.solid[index] = value;
                if value {
                    self.grid[index] = Cell::Empty;
                }
            }
        }
    }

    fn update_ball_index(&mut self) {
        let ball = self.ball_position();
        if let Ok(bounds) = Aabb::from_center_half_extents(
            [f64::from(ball[0]), f64::from(ball[1])],
            [f64::from(BALL_RADIUS), f64::from(BALL_RADIUS)],
        ) {
            self.obstacle_index.insert(BALL_KEY, bounds);
        }
    }

    fn simulate_sand(&mut self) {
        let solid = &self.solid;
        let grid = &mut self.grid;
        for y in (1..GRID_HEIGHT - 1).rev() {
            let right_first = (y as u64 + self.tick).is_multiple_of(2);
            for step in 0..GRID_WIDTH {
                let x = if right_first {
                    step
                } else {
                    GRID_WIDTH - 1 - step
                };
                if x == 0 || x >= GRID_WIDTH - 1 {
                    continue;
                }
                let index = y * GRID_WIDTH + x;
                if grid[index] != Cell::Sand {
                    continue;
                }
                let below = index + GRID_WIDTH;
                if !solid[below] && grid[below] == Cell::Empty {
                    grid[below] = Cell::Sand;
                    grid[index] = Cell::Empty;
                    continue;
                }
                let direction = if right_first { 1 } else { -1 };
                for dx in [direction, -direction] {
                    let target_x = x as isize + dx;
                    if target_x <= 0 || target_x >= GRID_WIDTH as isize - 1 {
                        continue;
                    }
                    let target = (y + 1) * GRID_WIDTH + target_x as usize;
                    if !solid[target] && grid[target] == Cell::Empty {
                        grid[target] = Cell::Sand;
                        grid[index] = Cell::Empty;
                        break;
                    }
                }
            }
        }
    }
}

/// Deterministic scripted input used by `--autoplay` and headless tests.
pub fn autoplay_input(world: &SandWorld) -> SandInput {
    let tick = world.tick();
    let tool = match (tick / 90) % 3 {
        0 => Tool::Sand,
        1 => Tool::Rock,
        _ => Tool::Erase,
    };
    let pointer = [
        (tick as f32 * 0.11).sin() * 70.0 + GRID_WIDTH as f32 * 0.5,
        (tick as f32 * 0.07).cos() * 28.0 + 52.0,
    ];
    SandInput {
        pointer,
        pointer_valid: true,
        primary_held: tick % 120 < 80,
        primary_pressed: tick.is_multiple_of(30),
        secondary_held: tick % 240 > 220,
        reset: false,
        tool,
        brush_radius: 4.0,
    }
}
