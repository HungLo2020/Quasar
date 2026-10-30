//! Deterministic 3D physics playground: kinematic player, dynamic props,
//! trigger sensor and spatial queries.
use quasar_animation::{
    Clip, Condition, Interpolation, Keyframe, LoopMode, ParameterValue, StateMachine, Track, Value,
};
use quasar_nav::{Grid, GridCell, SearchOptions, SearchOutcome};
use quasar_physics::dim3::{ActiveEvents, PhysicsWorld, StepConfig, WorldConfig};
use quasar_physics::rapier3d::control::{CharacterAutostep, KinematicCharacterController};
use quasar_physics::rapier3d::geometry::{
    ColliderHandle, Group, InteractionGroups, InteractionTestMode,
};
use quasar_physics::rapier3d::math::{Pose, Vector};
use quasar_physics::rapier3d::pipeline::QueryFilter;
use quasar_physics::rapier3d::prelude::{
    ColliderBuilder, RigidBodyBuilder, RigidBodyHandle, SharedShape,
};
use quasar_sim::StateHasher;
use quasar_spatial::{Aabb, Frustum, GridConfig, SpatialGrid};
use std::sync::Arc;

pub const PLAYER_KEY: u64 = u64::MAX;
pub const MAX_PROPS: usize = 64;
pub const PLAYER_RADIUS: f32 = 0.35;
pub const PLAYER_HALF_HEIGHT: f32 = 0.5;
pub const EYE_HEIGHT: f32 = 0.65;
pub const MOVE_SPEED: f32 = 5.0;
pub const JUMP_SPEED: f32 = 5.6;
pub const SENSITIVITY: f32 = 0.0022;
pub const NAV_EXTENT: f32 = 16.0;
pub const NAV_DIMENSION: u32 = 32;
pub const AGENT_COUNT: usize = 2;
pub const AGENT_RADIUS: f32 = 0.4;
pub const AGENT_SPEED: f32 = 2.4;

const GROUP_WORLD: Group = Group::GROUP_1;
const GROUP_PLAYER: Group = Group::GROUP_2;
const GROUP_PROP: Group = Group::GROUP_3;
const GROUP_TRIGGER: Group = Group::GROUP_4;

fn groups(memberships: Group, filter: Group) -> InteractionGroups {
    InteractionGroups::new(memberships, filter, InteractionTestMode::And)
}

#[derive(Clone, Copy, Debug)]
pub enum PropKind {
    Block { half: [f32; 3] },
    Ball { radius: f32 },
}

impl PropKind {
    pub fn half_extents(self) -> [f32; 3] {
        match self {
            Self::Block { half } => half,
            Self::Ball { radius } => [radius; 3],
        }
    }
    pub fn tint(self) -> [f32; 4] {
        match self {
            Self::Block { .. } => [0.55, 0.62, 0.78, 1.0],
            Self::Ball { .. } => [1.0, 0.56, 0.24, 1.0],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Prop {
    pub body: RigidBodyHandle,
    pub collider: ColliderHandle,
    pub kind: PropKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaygroundInput {
    pub forward: f32,
    pub strafe: f32,
    pub look: [f32; 2],
    pub jump: bool,
    pub kick: bool,
    pub spawn: bool,
    pub reset: bool,
    pub toggle_debug: bool,
}

impl Default for PlaygroundInput {
    fn default() -> Self {
        Self {
            forward: 0.0,
            strafe: 0.0,
            look: [0.0, 0.0],
            jump: false,
            kick: false,
            spawn: false,
            reset: false,
            toggle_debug: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LookHit {
    pub collider: ColliderHandle,
    pub prop: Option<usize>,
    pub distance: f32,
    pub point: [f32; 3],
    pub normal: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct StaticBox {
    pub bounds: Aabb<3>,
    pub tint: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct PlaygroundStats {
    pub props: usize,
    pub balls: usize,
    pub blocks: usize,
    pub player: [f32; 3],
    pub velocity_y: f32,
    pub grounded: bool,
    pub awake: usize,
    pub nearby_props: usize,
    pub look_hit: Option<LookHit>,
    pub zone_active: bool,
    pub collision_events: u64,
    pub agents: usize,
    pub agent_path_nodes: usize,
    pub nav_generation: u64,
    pub debug_nav: bool,
}

pub struct Agent {
    pub body: RigidBodyHandle,
    pub collider: ColliderHandle,
    pub position: [f32; 2],
    path: Vec<GridCell<2>>,
    path_index: usize,
    goal: GridCell<2>,
    nav_generation: u64,
    machine: StateMachine,
}

pub struct Playground {
    seed: u64,
    tick: u64,
    physics: PhysicsWorld,
    controller: KinematicCharacterController,
    player_body: RigidBodyHandle,
    player_collider: ColliderHandle,
    player_position: Vector,
    vertical_velocity: f32,
    grounded: bool,
    yaw: f32,
    pitch: f32,
    props: Vec<Prop>,
    static_boxes: Vec<StaticBox>,
    sensor: ColliderHandle,
    sensor_bounds: Aabb<3>,
    zone_active: bool,
    spatial: SpatialGrid<3, u64>,
    look_hit: Option<LookHit>,
    collision_events: u64,
    agents: Vec<Agent>,
    nav: Grid<2>,
    nav_generation: u64,
    nav_positions: Vec<([f32; 3], [f32; 4])>,
    nav_last_tick: u64,
    debug_nav: bool,
}

impl Playground {
    pub fn new(seed: u64) -> Self {
        let physics = PhysicsWorld::new(
            WorldConfig::new(
                [0.0, -9.81, 0.0],
                StepConfig::new(1.0 / 60.0, 0.25, 8).expect("playground step configuration"),
            )
            .expect("playground gravity"),
        )
        .expect("playground physics world");
        let mut world = Self {
            seed,
            tick: 0,
            physics,
            controller: KinematicCharacterController {
                up: Vector::Y,
                offset: quasar_physics::rapier3d::control::CharacterLength::Absolute(0.01),
                autostep: Some(CharacterAutostep {
                    max_height: quasar_physics::rapier3d::control::CharacterLength::Absolute(0.4),
                    min_width: quasar_physics::rapier3d::control::CharacterLength::Absolute(0.2),
                    include_dynamic_bodies: false,
                }),
                snap_to_ground: Some(
                    quasar_physics::rapier3d::control::CharacterLength::Absolute(0.25),
                ),
                max_slope_climb_angle: 50.0_f32.to_radians(),
                min_slope_slide_angle: 40.0_f32.to_radians(),
                ..KinematicCharacterController::default()
            },
            player_body: RigidBodyHandle::invalid(),
            player_collider: ColliderHandle::invalid(),
            player_position: Vector::new(0.0, 1.2, 6.0),
            vertical_velocity: 0.0,
            grounded: false,
            yaw: 0.0,
            pitch: 0.0,
            props: Vec::new(),
            static_boxes: Vec::new(),
            sensor: ColliderHandle::invalid(),
            sensor_bounds: Aabb::new([-2.4, 0.3, -7.2], [2.4, 2.7, -4.8]).expect("sensor bounds"),
            zone_active: false,
            spatial: SpatialGrid::new(GridConfig::new(4.0, 256, 100_000).expect("spatial config"))
                .expect("spatial grid"),
            look_hit: None,
            collision_events: 0,
            agents: Vec::new(),
            nav: Grid::new([NAV_DIMENSION, NAV_DIMENSION]).expect("nav dimensions"),
            nav_generation: 0,
            nav_positions: Vec::new(),
            nav_last_tick: 0,
            debug_nav: false,
        };
        world.build();
        world.build_agents();
        world.rebuild_nav();
        world.rebuild_spatial();
        world.replan_agents();
        world
    }

    fn build_agents(&mut self) {
        let idle = Arc::new(pulse_clip("idle", 1.0, 0.6, 1.0));
        let moving = Arc::new(pulse_clip("move", 0.45, 0.85, 1.15));
        let alert = Arc::new(pulse_clip("alert", 0.3, 1.0, 1.3));
        for index in 0..AGENT_COUNT {
            let x = -3.5 + index as f32 * 7.0;
            let z = 2.0;
            let body = self.physics.insert_body(
                RigidBodyBuilder::kinematic_position_based().translation(Vector::new(
                    x,
                    AGENT_RADIUS,
                    z,
                )),
            );
            let collider = self.physics.insert_collider(
                ColliderBuilder::ball(AGENT_RADIUS)
                    .friction(0.2)
                    .collision_groups(groups(GROUP_PROP, GROUP_WORLD | GROUP_PLAYER | GROUP_PROP)),
                Some(body),
            );
            let mut machine = StateMachine::new();
            let layer = machine.add_layer("base");
            machine
                .add_state(layer, "idle", idle.clone(), LoopMode::Loop, 1.0)
                .expect("idle state");
            machine
                .add_state(layer, "move", moving.clone(), LoopMode::Loop, 1.2)
                .expect("move state");
            machine
                .add_state(layer, "alert", alert.clone(), LoopMode::Loop, 1.6)
                .expect("alert state");
            for (from, condition) in [
                (
                    "idle",
                    Condition::Float {
                        parameter: "distance".into(),
                        compare: quasar_animation::Compare::Less,
                        value: 3.0,
                    },
                ),
                (
                    "move",
                    Condition::Float {
                        parameter: "distance".into(),
                        compare: quasar_animation::Compare::Less,
                        value: 3.0,
                    },
                ),
            ] {
                machine
                    .add_transition(
                        layer,
                        quasar_animation::Transition::new(
                            Some(from.into()),
                            "alert",
                            0.18,
                            condition,
                        )
                        .expect("alert transition"),
                    )
                    .expect("alert transition target");
            }
            for (to, moving) in [("move", true), ("idle", false)] {
                machine
                    .add_transition(
                        layer,
                        quasar_animation::Transition::new(
                            Some("alert".into()),
                            to,
                            0.18,
                            Condition::All(vec![
                                Condition::Float {
                                    parameter: "distance".into(),
                                    compare: quasar_animation::Compare::Greater,
                                    value: 3.5,
                                },
                                Condition::Bool {
                                    parameter: "moving".into(),
                                    equals: moving,
                                },
                            ]),
                        )
                        .expect("exit alert transition"),
                    )
                    .expect("exit alert transition target");
            }
            machine
                .add_transition(
                    layer,
                    quasar_animation::Transition::new(
                        Some("idle".into()),
                        "move",
                        0.12,
                        Condition::Bool {
                            parameter: "moving".into(),
                            equals: true,
                        },
                    )
                    .expect("move transition"),
                )
                .expect("move transition target");
            machine
                .add_transition(
                    layer,
                    quasar_animation::Transition::new(
                        Some("move".into()),
                        "idle",
                        0.12,
                        Condition::Bool {
                            parameter: "moving".into(),
                            equals: false,
                        },
                    )
                    .expect("idle transition"),
                )
                .expect("idle transition target");
            let goal = if index == 0 {
                GridCell::new2(29, 3)
            } else {
                GridCell::new2(3, 4)
            };
            self.agents.push(Agent {
                body,
                collider,
                position: [x, z],
                path: Vec::new(),
                path_index: 0,
                goal,
                nav_generation: 0,
                machine,
            });
        }
    }

    fn build(&mut self) {
        self.add_static(
            Aabb::new([-18.0, -1.0, -18.0], [18.0, 0.0, 18.0]).unwrap(),
            [0.16, 0.20, 0.26, 1.0],
        );
        for (bounds, tint) in [
            (
                Aabb::new([-18.0, 0.0, -18.0], [-17.0, 3.0, 18.0]).unwrap(),
                [0.24, 0.30, 0.38, 1.0],
            ),
            (
                Aabb::new([17.0, 0.0, -18.0], [18.0, 3.0, 18.0]).unwrap(),
                [0.24, 0.30, 0.38, 1.0],
            ),
            (
                Aabb::new([-18.0, 0.0, -18.0], [18.0, 3.0, -17.0]).unwrap(),
                [0.24, 0.30, 0.38, 1.0],
            ),
            (
                Aabb::new([-18.0, 0.0, 17.0], [18.0, 3.0, 18.0]).unwrap(),
                [0.24, 0.30, 0.38, 1.0],
            ),
        ] {
            self.add_static(bounds, tint);
        }
        self.add_static(
            Aabb::new([-4.0, 0.0, -2.0], [4.0, 0.25, 0.0]).unwrap(),
            [0.30, 0.42, 0.34, 1.0],
        );
        self.add_static(
            Aabb::new([6.0, 0.0, -8.0], [9.0, 1.2, -5.0]).unwrap(),
            [0.36, 0.34, 0.30, 1.0],
        );

        let player_shared = SharedShape::capsule_y(PLAYER_HALF_HEIGHT, PLAYER_RADIUS);
        self.player_body = self.physics.insert_body(
            RigidBodyBuilder::kinematic_position_based().translation(Vector::new(0.0, 1.2, 6.0)),
        );
        self.player_collider = self.physics.insert_collider(
            ColliderBuilder::new(player_shared)
                .friction(0.0)
                .active_events(ActiveEvents::COLLISION_EVENTS)
                .collision_groups(groups(
                    GROUP_PLAYER,
                    GROUP_WORLD | GROUP_PROP | GROUP_TRIGGER,
                )),
            Some(self.player_body),
        );

        let sensor = ColliderBuilder::cuboid(2.4, 1.2, 1.2)
            .translation(Vector::new(0.0, 1.5, -6.0))
            .sensor(true)
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .collision_groups(groups(GROUP_TRIGGER, GROUP_PLAYER | GROUP_PROP));
        self.sensor = self.physics.insert_collider(sensor, None);

        for (index, position) in [
            [0.0, 0.6, 3.5],
            [0.0, 1.8, 3.5],
            [1.4, 0.6, 3.5],
            [-1.4, 0.6, 3.5],
            [5.0, 0.6, -2.0],
            [6.2, 0.6, -2.0],
            [5.0, 1.8, -2.0],
        ]
        .into_iter()
        .enumerate()
        {
            let half = if index == 3 {
                [0.5, 0.8, 0.5]
            } else {
                [0.6, 0.6, 0.6]
            };
            self.spawn_block(position, half);
        }
        for position in [[2.0, 5.0, 0.0], [-2.0, 4.0, -1.0], [0.0, 6.0, 2.0]] {
            self.spawn_ball_at(position, 0.45);
        }
    }

    fn add_static(&mut self, bounds: Aabb<3>, tint: [f32; 4]) {
        let center = bounds.center();
        let extents = bounds.extents();
        let collider = ColliderBuilder::cuboid(
            (extents[0] * 0.5) as f32,
            (extents[1] * 0.5) as f32,
            (extents[2] * 0.5) as f32,
        )
        .translation(Vector::new(
            center[0] as f32,
            center[1] as f32,
            center[2] as f32,
        ))
        .collision_groups(groups(GROUP_WORLD, GROUP_PLAYER | GROUP_PROP));
        self.physics.insert(RigidBodyBuilder::fixed(), collider);
        self.static_boxes.push(StaticBox { bounds, tint });
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn player_position(&self) -> [f32; 3] {
        [
            self.player_position.x,
            self.player_position.y,
            self.player_position.z,
        ]
    }

    pub fn camera_eye(&self) -> [f32; 3] {
        [
            self.player_position.x,
            self.player_position.y + EYE_HEIGHT,
            self.player_position.z,
        ]
    }

    pub fn yaw(&self) -> f32 {
        self.yaw
    }

    pub fn pitch(&self) -> f32 {
        self.pitch
    }

    pub fn forward(&self) -> [f32; 3] {
        let cp = self.pitch.cos();
        [-self.yaw.sin() * cp, self.pitch.sin(), -self.yaw.cos() * cp]
    }

    pub fn right(&self) -> [f32; 3] {
        [self.yaw.cos(), 0.0, -self.yaw.sin()]
    }

    pub fn props(&self) -> &[Prop] {
        &self.props
    }

    pub fn agents(&self) -> &[Agent] {
        &self.agents
    }

    pub fn agent_state(&self, index: usize) -> Option<&str> {
        self.agents.get(index)?.machine.current_state(0)
    }

    pub fn agent_path(&self, index: usize) -> Option<&[GridCell<2>]> {
        self.agents.get(index).map(|agent| agent.path.as_slice())
    }

    pub fn nav_generation(&self) -> u64 {
        self.nav_generation
    }

    pub fn debug_nav(&self) -> bool {
        self.debug_nav
    }

    pub fn set_debug_nav(&mut self, enabled: bool) {
        self.debug_nav = enabled;
    }

    /// Position and rotation for rendering one prop.
    pub fn prop_transform(&self, index: usize) -> Option<([f32; 3], [f32; 4])> {
        let prop = self.props.get(index)?;
        let body = &self.physics.bodies[prop.body];
        let position = body.translation();
        let rotation = body.rotation();
        Some((
            [position.x, position.y, position.z],
            [rotation.x, rotation.y, rotation.z, rotation.w],
        ))
    }

    pub fn static_boxes(&self) -> &[StaticBox] {
        &self.static_boxes
    }

    pub fn spatial(&self) -> &SpatialGrid<3, u64> {
        &self.spatial
    }

    pub fn sensor_bounds(&self) -> Aabb<3> {
        self.sensor_bounds
    }

    pub fn look_hit(&self) -> Option<LookHit> {
        self.look_hit
    }

    pub fn zone_active(&self) -> bool {
        self.zone_active
    }

    pub fn stats(&self) -> PlaygroundStats {
        let awake = self
            .physics
            .bodies
            .iter()
            .filter(|(_, body)| !body.is_sleeping())
            .count();
        let balls = self
            .props
            .iter()
            .filter(|prop| matches!(prop.kind, PropKind::Ball { .. }))
            .count();
        let nearby_props = self
            .spatial
            .query_radius(
                [
                    f64::from(self.player_position.x),
                    f64::from(self.player_position.y),
                    f64::from(self.player_position.z),
                ],
                8.0,
            )
            .into_iter()
            .filter(|key| *key != PLAYER_KEY)
            .count();
        PlaygroundStats {
            props: self.props.len(),
            balls,
            blocks: self.props.len() - balls,
            player: self.player_position(),
            velocity_y: self.vertical_velocity,
            grounded: self.grounded,
            awake,
            nearby_props,
            look_hit: self.look_hit,
            zone_active: self.zone_active,
            collision_events: self.collision_events,
            agents: self.agents.len(),
            agent_path_nodes: self.agents.iter().map(|agent| agent.path.len()).sum(),
            nav_generation: self.nav_generation,
            debug_nav: self.debug_nav,
        }
    }

    pub fn state_hash(&self) -> u64 {
        let mut hasher = StateHasher::new();
        hasher.write_u64(self.tick);
        hasher.write_u64(self.seed);
        for value in self.player_position() {
            hasher.write_f32_bits(value);
        }
        hasher.write_f32_bits(self.vertical_velocity);
        hasher.write_f32_bits(self.yaw);
        hasher.write_f32_bits(self.pitch);
        hasher.write_u8(self.grounded as u8);
        for prop in &self.props {
            let body = &self.physics.bodies[prop.body];
            let position = body.translation();
            let rotation = body.rotation();
            for value in [position.x, position.y, position.z] {
                hasher.write_f32_bits(value);
            }
            for value in [rotation.x, rotation.y, rotation.z, rotation.w] {
                hasher.write_f32_bits(value);
            }
        }
        for (index, agent) in self.agents.iter().enumerate() {
            hasher.write_u64(index as u64);
            hasher.write_f32_bits(agent.position[0]);
            hasher.write_f32_bits(agent.position[1]);
            hasher.write(agent.machine.current_state(0).unwrap_or("").as_bytes());
            hasher.write_u8(agent.path_index.min(255) as u8);
        }
        hasher.finish().value()
    }

    pub fn update(&mut self, input: PlaygroundInput) {
        if input.reset {
            *self = Self::new(self.seed);
            return;
        }
        if input.spawn {
            self.spawn_ball_from_eye();
        }
        if input.toggle_debug {
            self.debug_nav = !self.debug_nav;
        }
        self.yaw -= input.look[0] * SENSITIVITY;
        self.pitch = (self.pitch - input.look[1] * SENSITIVITY).clamp(-1.45, 1.45);

        let dt = self.physics.config().step.fixed_dt;
        let obstacles_changed = self.nav_obstacles_changed();
        if obstacles_changed && self.tick.saturating_sub(self.nav_last_tick) >= 5 {
            self.rebuild_nav();
        }
        self.update_agents(dt);
        let forward = self.forward();
        let right = self.right();
        let mut wish = [
            forward[0] * input.forward + right[0] * input.strafe,
            0.0,
            forward[2] * input.forward + right[2] * input.strafe,
        ];
        let length = (wish[0] * wish[0] + wish[2] * wish[2]).sqrt();
        if length > 1.0 {
            wish[0] /= length;
            wish[2] /= length;
        }
        if self.grounded && self.vertical_velocity <= 0.0 {
            self.vertical_velocity = 0.0;
        } else {
            self.vertical_velocity -= 9.81 * dt;
        }
        if input.jump && self.grounded {
            self.vertical_velocity = JUMP_SPEED;
        }
        let desired = Vector::new(
            wish[0] * MOVE_SPEED * dt,
            self.vertical_velocity * dt,
            wish[2] * MOVE_SPEED * dt,
        );
        let shape = self.physics.colliders[self.player_collider]
            .shared_shape()
            .clone();
        let filter = QueryFilter::default()
            .exclude_collider(self.player_collider)
            .exclude_rigid_body(self.player_body)
            .exclude_sensors();
        let mut collisions = Vec::new();
        let movement = {
            let queries = self.physics.query_pipeline_with_filter(filter);
            self.controller.move_shape(
                dt,
                &queries,
                &*shape,
                &Pose::from_translation(self.player_position),
                desired,
                |collision| collisions.push(collision),
            )
        };
        let mut new_position = self.player_position + movement.translation;
        if new_position.y < -8.0 {
            new_position = Vector::new(0.0, 1.2, 6.0);
            self.vertical_velocity = 0.0;
        }
        self.player_position = new_position;
        self.grounded = movement.grounded;
        if self.grounded && self.vertical_velocity < 0.0 {
            self.vertical_velocity = 0.0;
        }
        self.physics.bodies[self.player_body].set_next_kinematic_translation(new_position);

        if !collisions.is_empty() {
            let raw = self.physics.raw_mut();
            let dispatcher = raw.narrow_phase.query_dispatcher();
            let mut queries = raw.broad_phase.as_query_pipeline_mut(
                dispatcher,
                &mut raw.bodies,
                &mut raw.colliders,
                filter,
            );
            self.controller.solve_character_collision_impulses(
                dt,
                &mut queries,
                &*shape,
                80.0,
                &collisions,
            );
        }

        self.physics.step_fixed();
        self.player_position = self.physics.bodies[self.player_body].translation();
        if input.kick {
            self.kick();
        }
        self.rebuild_spatial();
        self.look_hit = self.current_look_hit();
        for event in self.physics.drain_events().collisions {
            self.collision_events += 1;
            let _ = event;
        }
        // Query the full player capsule: its center may remain outside a trigger
        // while its surface has entered it (e.g. when stopped by a nearby prop).
        // Events above still exercise the event channel and counters.
        let player = &self.physics.colliders[self.player_collider];
        self.zone_active = self
            .physics
            .intersect_shape(
                *player.position(),
                player.shape(),
                QueryFilter::default().exclude_collider(self.player_collider),
            )
            .any(|(handle, _)| handle == self.sensor);
        self.tick = self.tick.saturating_add(1);
    }

    pub fn agent_pulse(&self, index: usize) -> f32 {
        self.agents
            .get(index)
            .and_then(|agent| agent.machine.value("pulse"))
            .and_then(Value::as_f32)
            .unwrap_or(1.0)
    }

    fn world_cell(&self, position: [f32; 2]) -> GridCell<2> {
        GridCell::new2(
            ((position[0] + NAV_EXTENT).floor() as i32).clamp(0, NAV_DIMENSION as i32 - 1),
            ((position[1] + NAV_EXTENT).floor() as i32).clamp(0, NAV_DIMENSION as i32 - 1),
        )
    }

    fn cell_center(cell: GridCell<2>) -> [f32; 2] {
        [
            cell.coords[0] as f32 - NAV_EXTENT + 0.5,
            cell.coords[1] as f32 - NAV_EXTENT + 0.5,
        ]
    }

    fn rebuild_nav(&mut self) {
        let mut nav = Grid::<2>::new([NAV_DIMENSION, NAV_DIMENSION]).expect("nav dimensions");
        for static_box in &self.static_boxes {
            if static_box.bounds.max[1] <= 0.05 {
                continue;
            }
            self.block_nav_bounds(&mut nav, static_box.bounds, 0.05);
        }
        for prop in &self.props {
            let aabb = self.physics.colliders[prop.collider].compute_aabb();
            let bounds = Aabb::new(
                [
                    f64::from(aabb.mins.x) - 0.25,
                    f64::from(aabb.mins.y),
                    f64::from(aabb.mins.z) - 0.25,
                ],
                [
                    f64::from(aabb.maxs.x) + 0.25,
                    f64::from(aabb.maxs.y),
                    f64::from(aabb.maxs.z) + 0.25,
                ],
            );
            if let Ok(bounds) = bounds {
                self.block_nav_bounds(&mut nav, bounds, 0.0);
            }
        }
        self.nav = nav;
        self.nav_generation = self.nav_generation.wrapping_add(1);
        self.nav_positions = self
            .props
            .iter()
            .map(|prop| {
                let body = &self.physics.bodies[prop.body];
                let position = body.translation();
                let rotation = body.rotation();
                (
                    [position.x, position.y, position.z],
                    [rotation.x, rotation.y, rotation.z, rotation.w],
                )
            })
            .collect();
        self.nav_last_tick = self.tick;
    }

    fn nav_obstacles_changed(&self) -> bool {
        if self.nav_positions.len() != self.props.len() {
            return true;
        }
        for (prop, (position, rotation)) in self.props.iter().zip(&self.nav_positions) {
            let body = &self.physics.bodies[prop.body];
            let current = body.translation();
            let current_rotation = body.rotation();
            let moved = (current.x - position[0]).powi(2)
                + (current.y - position[1]).powi(2)
                + (current.z - position[2]).powi(2)
                > 0.05 * 0.05;
            let turned = (current_rotation.x - rotation[0]).abs()
                + (current_rotation.y - rotation[1]).abs()
                + (current_rotation.z - rotation[2]).abs()
                + (current_rotation.w - rotation[3]).abs()
                > 0.01;
            if moved || turned {
                return true;
            }
        }
        false
    }

    fn block_nav_bounds(&self, nav: &mut Grid<2>, bounds: Aabb<3>, margin: f64) {
        let planar = Aabb::new(
            [
                bounds.min[0] + f64::from(NAV_EXTENT) - margin,
                bounds.min[2] + f64::from(NAV_EXTENT) - margin,
            ],
            [
                bounds.max[0] + f64::from(NAV_EXTENT) + margin,
                bounds.max[2] + f64::from(NAV_EXTENT) + margin,
            ],
        );
        if let Ok(planar) = planar {
            let _ = nav.set_aabb_blocked(planar, true);
        }
    }

    fn nearest_free(&self, goal: GridCell<2>) -> Option<GridCell<2>> {
        if !self.nav.is_blocked(goal) {
            return Some(goal);
        }
        for radius in 1i32..=8 {
            for offset_y in -radius..=radius {
                for offset_x in -radius..=radius {
                    if offset_x.abs() != radius && offset_y.abs() != radius {
                        continue;
                    }
                    let candidate = GridCell::new2(
                        (goal.coords[0] + offset_x).clamp(0, NAV_DIMENSION as i32 - 1),
                        (goal.coords[1] + offset_y).clamp(0, NAV_DIMENSION as i32 - 1),
                    );
                    if !self.nav.is_blocked(candidate) {
                        return Some(candidate);
                    }
                }
            }
        }
        None
    }

    fn replan_agent(&mut self, index: usize) {
        let Some(agent) = self.agents.get(index) else {
            return;
        };
        let start = self.world_cell(agent.position);
        let goal = self.nearest_free(agent.goal);
        if let Some(goal) = goal
            && start != goal
            && !self.nav.is_blocked(start)
        {
            match self.nav.find_smoothed_path(
                start,
                goal,
                SearchOptions {
                    max_visited: 4096,
                    ..SearchOptions::default()
                },
            ) {
                SearchOutcome::Found(path) => {
                    self.agents[index].path = path.nodes;
                    self.agents[index].path_index = 1;
                }
                _ => {
                    self.agents[index].path.clear();
                    self.agents[index].path_index = 0;
                }
            }
        } else {
            self.agents[index].path.clear();
            self.agents[index].path_index = 0;
        }
        self.agents[index].nav_generation = self.nav_generation;
    }

    fn replan_agents(&mut self) {
        for index in 0..self.agents.len() {
            self.replan_agent(index);
        }
    }

    fn update_agents(&mut self, dt: f32) {
        for index in 0..self.agents.len() {
            if self.agents[index].nav_generation != self.nav_generation {
                self.replan_agent(index);
            }
            let [px, _, pz] = self.player_position();
            let distance = ((self.agents[index].position[0] - px).powi(2)
                + (self.agents[index].position[1] - pz).powi(2))
            .sqrt();
            let moving = self.agents[index].path_index < self.agents[index].path.len();
            {
                let agent = &mut self.agents[index];
                agent
                    .machine
                    .set_parameter("moving", ParameterValue::Bool(moving));
                agent
                    .machine
                    .set_parameter("distance", ParameterValue::Float(distance));
                let _ = agent.machine.update(dt);
            }
            if moving {
                let target_cell = self.agents[index].path[self.agents[index].path_index];
                let target = Self::cell_center(target_cell);
                let delta = [
                    target[0] - self.agents[index].position[0],
                    target[1] - self.agents[index].position[1],
                ];
                let distance = (delta[0] * delta[0] + delta[1] * delta[1]).sqrt();
                let step = AGENT_SPEED * dt;
                if distance <= step {
                    self.agents[index].position = target;
                    self.agents[index].path_index += 1;
                    if self.agents[index].path_index >= self.agents[index].path.len() {
                        self.agents[index].path.clear();
                        self.agents[index].path_index = 0;
                        self.agents[index].goal = GridCell::new2(
                            NAV_DIMENSION as i32 - 2 - self.agents[index].goal.coords[0],
                            NAV_DIMENSION as i32 - 2 - self.agents[index].goal.coords[1],
                        );
                        // Force a replan for the new goal without waiting for
                        // an obstacle-generation change.
                        self.agents[index].nav_generation = u64::MAX;
                    }
                } else if distance > f32::EPSILON {
                    self.agents[index].position[0] += delta[0] / distance * step;
                    self.agents[index].position[1] += delta[1] / distance * step;
                }
            }
            let position = self.agents[index].position;
            self.physics.bodies[self.agents[index].body].set_next_kinematic_translation(
                Vector::new(position[0], AGENT_RADIUS, position[1]),
            );
        }
    }

    fn current_look_hit(&self) -> Option<LookHit> {
        let eye = self.camera_eye();
        let direction = self.forward();
        let filter = QueryFilter::default()
            .exclude_collider(self.player_collider)
            .exclude_rigid_body(self.player_body);
        let hit = self
            .physics
            .raycast_filtered(eye, direction, 12.0, true, filter)?;
        let prop = self
            .props
            .iter()
            .position(|prop| prop.collider == hit.collider);
        Some(LookHit {
            collider: hit.collider,
            prop,
            distance: hit.distance,
            point: hit.point,
            normal: hit.normal,
        })
    }

    fn kick(&mut self) {
        let Some(hit) = self.look_hit else {
            return;
        };
        let Some(index) = hit.prop else {
            return;
        };
        let direction = self.forward();
        let impulse = Vector::new(
            direction[0] * 9.0,
            direction[1] * 9.0 + 2.5,
            direction[2] * 9.0,
        );
        self.physics.bodies[self.props[index].body].apply_impulse_at_point(
            impulse,
            Vector::new(hit.point[0], hit.point[1], hit.point[2]),
            true,
        );
    }

    pub fn spawn_block(&mut self, position: [f32; 3], half: [f32; 3]) {
        self.spawn_prop(PropKind::Block { half }, position, Vector::ZERO);
    }

    pub fn spawn_ball_at(&mut self, position: [f32; 3], radius: f32) {
        self.spawn_prop(
            PropKind::Ball { radius },
            position,
            Vector::new(0.0, -2.0, 0.0),
        );
    }

    pub fn spawn_ball_from_eye(&mut self) {
        if self.props.len() >= MAX_PROPS {
            return;
        }
        let eye = self.camera_eye();
        let forward = self.forward();
        let position = [
            eye[0] + forward[0] * 1.2,
            eye[1] + forward[1] * 1.2,
            eye[2] + forward[2] * 1.2,
        ];
        self.spawn_prop(
            PropKind::Ball { radius: 0.35 },
            position,
            Vector::new(forward[0] * 7.0, forward[1] * 7.0 + 1.0, forward[2] * 7.0),
        );
    }

    fn spawn_prop(&mut self, kind: PropKind, position: [f32; 3], velocity: Vector) {
        if self.props.len() >= MAX_PROPS {
            return;
        }
        let (collider, impulse) = match kind {
            PropKind::Block { half } => (
                ColliderBuilder::cuboid(half[0], half[1], half[2])
                    .friction(0.75)
                    .restitution(0.1),
                1.5,
            ),
            PropKind::Ball { radius } => (
                ColliderBuilder::ball(radius)
                    .friction(0.35)
                    .restitution(0.82),
                0.8,
            ),
        };
        let collider = collider
            .collision_groups(groups(GROUP_PROP, GROUP_WORLD | GROUP_PLAYER | GROUP_PROP))
            .mass(impulse);
        let (body, collider) = self.physics.insert(
            RigidBodyBuilder::dynamic()
                .translation(Vector::new(position[0], position[1], position[2]))
                .linvel(velocity)
                .linear_damping(0.05)
                .angular_damping(0.15),
            collider,
        );
        self.props.push(Prop {
            body,
            collider,
            kind,
        });
    }

    fn rebuild_spatial(&mut self) {
        self.spatial.clear();
        for (index, prop) in self.props.iter().enumerate() {
            let aabb = self.physics.colliders[prop.collider].compute_aabb();
            if let Ok(bounds) = quasar_spatial::Aabb::new(
                [
                    f64::from(aabb.mins.x),
                    f64::from(aabb.mins.y),
                    f64::from(aabb.mins.z),
                ],
                [
                    f64::from(aabb.maxs.x),
                    f64::from(aabb.maxs.y),
                    f64::from(aabb.maxs.z),
                ],
            ) {
                self.spatial.insert(index as u64, bounds);
            }
        }
        let aabb = self.physics.colliders[self.player_collider].compute_aabb();
        if let Ok(bounds) = quasar_spatial::Aabb::new(
            [
                f64::from(aabb.mins.x),
                f64::from(aabb.mins.y),
                f64::from(aabb.mins.z),
            ],
            [
                f64::from(aabb.maxs.x),
                f64::from(aabb.maxs.y),
                f64::from(aabb.maxs.z),
            ],
        ) {
            self.spatial.insert(PLAYER_KEY, bounds);
        }
    }

    /// Props whose AABBs intersect a camera frustum inside `region`.
    pub fn props_in_frustum(&self, frustum: &Frustum<3>, region: Aabb<3>) -> Vec<usize> {
        self.spatial
            .query_frustum(frustum, region)
            .into_iter()
            .filter(|key| *key != PLAYER_KEY)
            .filter_map(|key| usize::try_from(key).ok())
            .filter(|index| *index < self.props.len())
            .collect()
    }
}

/// Deterministic scripted play used by `--autoplay` and headless tests.
pub fn autoplay_input(world: &Playground) -> PlaygroundInput {
    let tick = world.tick();
    PlaygroundInput {
        forward: 1.0,
        strafe: (tick as f32 * 0.025).sin() * 0.5,
        look: [
            (tick as f32 * 0.013).sin() * 0.018,
            (tick as f32 * 0.009).cos() * 0.004,
        ],
        jump: tick.is_multiple_of(90),
        kick: tick.is_multiple_of(35),
        spawn: tick.is_multiple_of(150) && tick > 0,
        reset: false,
        toggle_debug: tick.is_multiple_of(300),
    }
}

fn pulse_clip(name: &str, duration: f32, low: f32, high: f32) -> Clip {
    Clip::new(
        name,
        duration,
        vec![
            Track::new(
                "pulse",
                Interpolation::Linear,
                vec![
                    Keyframe::new(0.0, Value::Float(low)),
                    Keyframe::new(duration * 0.5, Value::Float(high)),
                    Keyframe::new(duration, Value::Float(low)),
                ],
            )
            .expect("pulse track"),
        ],
        Vec::new(),
    )
    .expect("pulse clip")
}
