//! Game rules and deterministic world state. No renderer or window dependency.

use quasar_sim::{Simulation, StateHash, StateHasher, Tick};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, error::Error, fmt};

pub const ARENA_RADIUS: f32 = 11.5;
pub const MAX_SHOTS: usize = 16;
pub const MAX_HEALTH: i32 = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}
impl Vec3 {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
    pub fn length(self) -> f32 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }
    pub fn length_xz(self) -> f32 {
        (self.x * self.x + self.z * self.z).sqrt()
    }
    pub fn distance_xz(self, other: Self) -> f32 {
        (self - other).length_xz()
    }
    pub fn normalized_xz(self) -> Self {
        let length = self.length_xz();
        if length > 1e-6 {
            Self::new(self.x / length, 0., self.z / length)
        } else {
            Self::new(0., 0., 0.)
        }
    }
    pub fn clamp_arena(self, radius: f32) -> Self {
        let length = self.length_xz();
        if length > radius {
            let scale = radius / length;
            Self::new(self.x * scale, self.y, self.z * scale)
        } else {
            self
        }
    }
}
impl std::ops::Add for Vec3 {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }
}
impl std::ops::Sub for Vec3 {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }
}
impl std::ops::Mul<f32> for Vec3 {
    type Output = Self;
    fn mul(self, value: f32) -> Self {
        Self::new(self.x * value, self.y * value, self.z * value)
    }
}

/// One tick of player intent. Values are normally -1/0/1 but analog gamepad
/// values are preserved in journals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GameInput {
    pub forward: f32,
    pub strafe: f32,
    pub turn: f32,
    /// Direct yaw delta from captured mouse motion, in radians.
    #[serde(default)]
    pub look: f32,
    pub shoot: bool,
    pub restart: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DroneSpec {
    pub radius: f32,
    pub height: f32,
    pub speed: f32,
    pub phase: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Level {
    pub duration: f32,
    pub crystals: Vec<[f32; 3]>,
    pub drones: Vec<DroneSpec>,
    pub portal: [f32; 3],
}
impl Level {
    pub fn validate(&self) -> Result<(), GameError> {
        if !self.duration.is_finite() || !(10.0..=600.0).contains(&self.duration) {
            return Err(GameError::invalid(
                "level duration must be 10..=600 seconds",
            ));
        }
        if self.crystals.is_empty() || self.crystals.len() > 32 {
            return Err(GameError::invalid(
                "level must contain 1..=32 crystal positions",
            ));
        }
        for position in &self.crystals {
            if !position.iter().all(|value| value.is_finite())
                || (position[0] * position[0] + position[2] * position[2]).sqrt() > ARENA_RADIUS
            {
                return Err(GameError::invalid(
                    "crystal positions must be finite and inside the arena",
                ));
            }
        }
        if self.drones.len() > 16 {
            return Err(GameError::invalid("level supports at most 16 drones"));
        }
        for drone in &self.drones {
            if ![drone.radius, drone.height, drone.speed, drone.phase]
                .iter()
                .all(|value| value.is_finite())
                || drone.radius <= 0.
                || drone.radius > ARENA_RADIUS
                || !(0.5..=8.).contains(&drone.height)
                || drone.speed.abs() > 3.
            {
                return Err(GameError::invalid(
                    "drone radius/height/speed/phase are out of range",
                ));
            }
        }
        if !self.portal.iter().all(|value| value.is_finite())
            || (self.portal[0] * self.portal[0] + self.portal[2] * self.portal[2]).sqrt()
                > ARENA_RADIUS
        {
            return Err(GameError::invalid(
                "portal must be finite and inside the arena",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GameRules {
    pub hz: f32,
    pub time_limit: f32,
    pub player_speed: f32,
    pub turn_speed: f32,
    pub camera_distance: f32,
    pub camera_height: f32,
}
impl Default for GameRules {
    fn default() -> Self {
        Self {
            hz: 60.0,
            time_limit: 90.0,
            player_speed: 5.0,
            turn_speed: 1.8,
            camera_distance: 9.0,
            camera_height: 5.2,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Crystal {
    pub position: Vec3,
    pub active: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Drone {
    pub spec: DroneSpec,
    pub angle: f32,
    pub position: Vec3,
    pub active: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    pub position: Vec3,
    pub velocity: Vec3,
    pub age: f32,
}
impl Shot {
    /// Yaw that aligns the long local Z axis with the shot velocity.
    pub fn yaw(&self) -> f32 {
        self.velocity.x.atan2(self.velocity.z)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GameEvent {
    Restart,
    Crystal,
    Shot,
    DroneDestroyed,
    Damage,
    PortalReady,
    Won,
    Lost,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GameWorld {
    pub rules: GameRules,
    pub level: Level,
    pub tick: u64,
    pub player: Vec3,
    pub yaw: f32,
    pub health: i32,
    pub crystals: Vec<Crystal>,
    pub drones: Vec<Drone>,
    pub shots: Vec<Shot>,
    pub portal: Vec3,
    pub score: u32,
    pub time_left: f32,
    pub shot_cooldown: f32,
    pub hit_cooldown: f32,
    pub portal_ready: bool,
    pub won: bool,
    pub lost: bool,
    pub events: Vec<GameEvent>,
}
impl GameWorld {
    pub fn new(level: Level, rules: GameRules) -> Result<Self, GameError> {
        level.validate()?;
        let mut drones = Vec::with_capacity(level.drones.len());
        for spec in &level.drones {
            drones.push(Drone {
                spec: *spec,
                angle: spec.phase,
                position: drone_position(spec, spec.phase),
                active: true,
            });
        }
        Ok(Self {
            tick: 0,
            player: Vec3::new(0., 0., 7.5),
            yaw: std::f32::consts::PI,
            health: MAX_HEALTH,
            crystals: level
                .crystals
                .iter()
                .map(|position| Crystal {
                    position: Vec3::new(position[0], position[1], position[2]),
                    active: true,
                })
                .collect(),
            drones,
            shots: Vec::new(),
            portal: Vec3::new(level.portal[0], level.portal[1], level.portal[2]),
            score: 0,
            time_left: rules.time_limit.min(level.duration),
            shot_cooldown: 0.,
            hit_cooldown: 0.,
            portal_ready: false,
            won: false,
            lost: false,
            events: Vec::new(),
            rules,
            level,
        })
    }
    pub fn crystals_remaining(&self) -> usize {
        self.crystals
            .iter()
            .filter(|crystal| crystal.active)
            .count()
    }
    pub fn drones_remaining(&self) -> usize {
        self.drones.iter().filter(|drone| drone.active).count()
    }
    pub fn finished(&self) -> bool {
        self.won || self.lost
    }
    pub fn forward(&self) -> Vec3 {
        Vec3::new(self.yaw.sin(), 0., self.yaw.cos())
    }
    pub fn right(&self) -> Vec3 {
        // forward x up; for forward=(sin yaw,0,cos yaw) this is the camera's
        // screen-right direction.
        Vec3::new(-self.yaw.cos(), 0., self.yaw.sin())
    }
    pub fn camera_eye(&self) -> Vec3 {
        self.player - self.forward() * self.rules.camera_distance
            + Vec3::new(0., self.rules.camera_height, 0.)
    }
    pub fn update(&mut self, input: GameInput) {
        self.events.clear();
        if input.restart {
            if let Ok(world) = Self::new(self.level.clone(), self.rules) {
                *self = world;
                self.events.push(GameEvent::Restart);
            }
            return;
        }
        if self.finished() {
            return;
        }
        let dt = 1.0 / self.rules.hz.max(1.0);
        self.tick = self.tick.saturating_add(1);
        self.yaw = wrap_angle(self.yaw + input.turn * self.rules.turn_speed * dt + input.look);
        let movement = self.forward() * input.forward + self.right() * input.strafe;
        self.player =
            (self.player + movement * self.rules.player_speed * dt).clamp_arena(ARENA_RADIUS);
        self.shot_cooldown = (self.shot_cooldown - dt).max(0.);
        self.hit_cooldown = (self.hit_cooldown - dt).max(0.);
        if input.shoot && self.shot_cooldown <= 0. && self.shots.len() < MAX_SHOTS {
            self.shots.push(Shot {
                position: self.player + self.forward() * 0.85 + Vec3::new(0., 0.8, 0.),
                velocity: self.forward() * 12.0,
                age: 0.,
            });
            self.shot_cooldown = 0.22;
            self.events.push(GameEvent::Shot);
        }
        for shot in &mut self.shots {
            shot.position = shot.position + shot.velocity * dt;
            shot.age += dt;
        }
        self.shots
            .retain(|shot| shot.age < 2.0 && shot.position.length_xz() <= ARENA_RADIUS + 1.0);
        for drone in &mut self.drones {
            if !drone.active {
                continue;
            }
            drone.angle = wrap_angle(drone.angle + drone.spec.speed * dt);
            drone.position = drone_position(&drone.spec, drone.angle);
        }
        for shot in &mut self.shots {
            for drone in &mut self.drones {
                if drone.active && shot.position.distance_xz(drone.position) < 0.8 {
                    drone.active = false;
                    shot.age = 10.;
                    self.events.push(GameEvent::DroneDestroyed);
                    break;
                }
            }
        }
        self.shots.retain(|shot| shot.age < 2.0);
        for crystal in &mut self.crystals {
            if crystal.active && self.player.distance_xz(crystal.position) < 1.0 {
                crystal.active = false;
                self.score = self.score.saturating_add(1);
                self.events.push(GameEvent::Crystal);
            }
        }
        if !self.portal_ready && self.crystals.iter().all(|crystal| !crystal.active) {
            self.portal_ready = true;
            self.events.push(GameEvent::PortalReady);
        }
        if self.portal_ready && self.player.distance_xz(self.portal) < 1.3 {
            self.won = true;
            self.events.push(GameEvent::Won);
            return;
        }
        if self.hit_cooldown <= 0. {
            for drone in &self.drones {
                if drone.active && self.player.distance_xz(drone.position) < 0.9 {
                    self.health -= 1;
                    self.hit_cooldown = 1.0;
                    self.events.push(GameEvent::Damage);
                    if self.health <= 0 {
                        self.health = 0;
                        self.lost = true;
                        self.events.push(GameEvent::Lost);
                    }
                    break;
                }
            }
        }
        self.time_left = (self.time_left - dt).max(0.);
        if self.time_left <= 0. && !self.won {
            self.lost = true;
            self.events.push(GameEvent::Lost);
        }
    }
    pub fn state_hash(&self) -> StateHash {
        let mut hash = StateHasher::new();
        hash.write_u64(self.tick);
        hash.write_f32_bits(self.player.x);
        hash.write_f32_bits(self.player.z);
        hash.write_f32_bits(self.yaw);
        hash.write_i64(i64::from(self.health));
        hash.write_u64(u64::from(self.score));
        hash.write_f32_bits(self.time_left);
        hash.write_f32_bits(self.shot_cooldown);
        hash.write_f32_bits(self.hit_cooldown);
        hash.write_u8(u8::from(self.portal_ready));
        hash.write_u8(u8::from(self.won));
        hash.write_u8(u8::from(self.lost));
        for crystal in &self.crystals {
            hash.write_u8(u8::from(crystal.active));
        }
        for drone in &self.drones {
            hash.write_u8(u8::from(drone.active));
            hash.write_f32_bits(drone.position.x);
            hash.write_f32_bits(drone.position.y);
            hash.write_f32_bits(drone.position.z);
        }
        for shot in &self.shots {
            hash.write_f32_bits(shot.position.x);
            hash.write_f32_bits(shot.position.y);
            hash.write_f32_bits(shot.position.z);
        }
        hash.finish()
    }
}
impl Simulation for GameWorld {
    type Input = GameInput;
    type Snapshot = GameWorld;
    type Error = Infallible;

    fn update(&mut self, _: Tick, input: &GameInput) -> Result<(), Self::Error> {
        self.update(*input);
        Ok(())
    }
    fn state_hash(&self) -> StateHash {
        GameWorld::state_hash(self)
    }
    fn snapshot(&self) -> GameWorld {
        self.clone()
    }
    fn restore(&mut self, snapshot: &GameWorld) -> Result<(), Self::Error> {
        *self = snapshot.clone();
        Ok(())
    }
}

/// Deterministic bot used by headless record/replay and automated rendering.
pub fn autoplay_input(world: &GameWorld) -> GameInput {
    let target = if world.portal_ready {
        Some(world.portal)
    } else {
        world
            .crystals
            .iter()
            .filter(|crystal| crystal.active)
            .min_by(|left, right| {
                world
                    .player
                    .distance_xz(left.position)
                    .total_cmp(&world.player.distance_xz(right.position))
            })
            .map(|crystal| crystal.position)
    };
    let (forward, turn) = target.map_or((0.0, 0.0), |target| {
        let delta = target - world.player;
        let desired = delta.x.atan2(delta.z);
        let difference = wrap_angle(desired - world.yaw);
        let turn = (difference * 2.5).clamp(-1.0, 1.0);
        let forward = if world.player.distance_xz(target) > 0.55 && difference.abs() < 0.35 {
            1.0
        } else {
            0.0
        };
        (forward, turn)
    });
    let shoot = world.shot_cooldown <= 0.
        && world
            .drones
            .iter()
            .filter(|drone| drone.active)
            .any(|drone| drone.position.distance_xz(world.player) < 8.0);
    GameInput {
        forward,
        strafe: 0.,
        turn,
        look: 0.,
        shoot,
        restart: false,
    }
}

fn drone_position(spec: &DroneSpec, angle: f32) -> Vec3 {
    Vec3::new(
        angle.cos() * spec.radius,
        spec.height + (angle * 2.0).sin() * 0.25,
        angle.sin() * spec.radius,
    )
}
fn wrap_angle(angle: f32) -> f32 {
    let two_pi = std::f32::consts::TAU;
    let mut angle = angle % two_pi;
    if angle > std::f32::consts::PI {
        angle -= two_pi;
    } else if angle < -std::f32::consts::PI {
        angle += two_pi;
    }
    angle
}

#[derive(Debug)]
pub struct GameError {
    message: String,
}
impl GameError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
    pub fn message(&self) -> &str {
        &self.message
    }
}
impl fmt::Display for GameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl Error for GameError {}
impl From<String> for GameError {
    fn from(value: String) -> Self {
        Self::invalid(value)
    }
}
impl From<&str> for GameError {
    fn from(value: &str) -> Self {
        Self::invalid(value)
    }
}
