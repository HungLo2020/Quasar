//! Game feel on top of the deterministic simulation: hit-stop, trauma
//! camera shake and render interpolation. None of it touches the simulated
//! state, so recordings and replays are unaffected.
use crate::world::{GameEvent, GameWorld, Vec3};
use quasar_core::feel::{HitStop, Trauma, TraumaSettings};
use quasar_sim::interp::{Interpolation, blend_angle};

/// Keys in the interpolation snapshot.
const PLAYER: u32 = 0;
const DRONES: u32 = 1;

pub struct Feel {
    pub hit_stop: HitStop,
    pub shake: Trauma,
    motion: Interpolation<u32, [f32; 3]>,
    yaw: Option<(u64, f32)>,
}

impl Default for Feel {
    fn default() -> Self {
        Self::new()
    }
}

fn arr(v: Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

impl Feel {
    pub fn new() -> Self {
        Self {
            hit_stop: HitStop::new(0.25),
            shake: Trauma::new(
                TraumaSettings {
                    amplitude: [0.35, 0.2, 0.35],
                    ..TraumaSettings::default()
                },
                0x5ea1,
            ),
            // Restarts teleport the craft: draw them at once.
            motion: Interpolation::new().with_snap(|a: &[f32; 3], b: &[f32; 3]| {
                let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
                d.iter().map(|v| v * v).sum::<f32>() > 9.0
            }),
            yaw: None,
        }
    }

    /// Advances the timers by one frame of real time.
    pub fn frame(&mut self, dt: f32) {
        self.hit_stop.scale_dt(dt);
        self.shake.update(dt);
    }

    /// Whether simulation ticks should wait (a hit-stop is running).
    pub fn frozen(&self) -> bool {
        self.hit_stop.frozen()
    }

    /// Big moments jolt the camera and briefly freeze the action.
    pub fn on_events(&mut self, events: &[GameEvent]) {
        for event in events {
            match event {
                GameEvent::Damage => {
                    self.shake.add(0.6);
                    self.hit_stop.trigger(0.07);
                }
                GameEvent::DroneDestroyed => {
                    self.shake.add(0.3);
                    self.hit_stop.trigger(0.045);
                }
                GameEvent::Lost | GameEvent::Won => self.shake.add(0.8),
                GameEvent::Restart => {
                    self.shake.clear();
                    self.hit_stop.clear();
                    self.motion.clear();
                }
                _ => {}
            }
        }
    }

    /// Records the pre-tick positions; call just before each step.
    pub fn record(&mut self, world: &GameWorld) {
        let drones = world
            .drones
            .iter()
            .enumerate()
            .map(|(i, d)| (DRONES + i as u32, arr(d.position)));
        self.motion.record(
            world.tick,
            std::iter::once((PLAYER, arr(world.player))).chain(drones),
        );
        self.yaw = Some((world.tick, world.yaw));
    }

    /// The world as drawn: positions `alpha` of the way from the previous
    /// tick to the current one.
    pub fn shown(&self, world: &GameWorld, alpha: f32) -> GameWorld {
        let mut shown = world.clone();
        let at = |v: [f32; 3]| Vec3::new(v[0], v[1], v[2]);
        shown.player = at(self
            .motion
            .blend(world.tick, &PLAYER, &arr(world.player), alpha));
        for (i, drone) in shown.drones.iter_mut().enumerate() {
            drone.position = at(self.motion.blend(
                world.tick,
                &(DRONES + i as u32),
                &arr(drone.position),
                alpha,
            ));
        }
        if let Some((tick, yaw)) = self.yaw
            && tick + 1 == world.tick
        {
            shown.yaw = blend_angle(yaw, world.yaw, alpha.clamp(0.0, 1.0));
        }
        shown
    }

    /// Camera offset for this frame.
    pub fn camera_shake(&self) -> [f32; 3] {
        self.shake.offset()
    }
}
