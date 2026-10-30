//! Game rules only. No renderer, window, ECS, scene document, clock or global state.
use crate::{
    collision::Scene,
    props::{self, Prop, SKIN},
};
use quasar_nav::{GridCell, SearchOptions, SearchOutcome};
use quasar_physics::{
    dim3::{CollisionWorld, RayHit, ShapeError3},
    rapier3d::pipeline::QueryFilter,
};
pub const DT: f32 = 1.0 / 60.0;
pub const EYE_HEIGHT: f32 = 1.65;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weapon {
    Pistol,
    Rifle,
}
impl Weapon {
    pub fn capacity(self) -> u32 {
        if self == Self::Pistol { 12 } else { 30 }
    }
    pub fn name(self) -> &'static str {
        if self == Self::Pistol {
            "K9 // SERVICE PISTOL"
        } else {
            "AR4 // ASSAULT RIFLE"
        }
    }
    fn cooldown(self) -> u32 {
        if self == Self::Pistol { 15 } else { 6 }
    }
    fn damage(self) -> f32 {
        if self == Self::Pistol { 48. } else { 30. }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    pub forward: f32,
    pub strafe: f32,
    pub look: [f32; 2],
    pub fire: bool,
    pub reload: bool,
    pub jump: bool,
    pub sprint: bool,
    pub weapon: Option<Weapon>,
    pub flashlight: bool,
    pub restart: bool,
}
#[derive(Clone, Debug)]
pub struct Zombie {
    pub position: [f32; 2],
    pub feet_y: f32,
    pub yaw: f32,
    pub health: f32,
    pub gait: f32,
    pub hit_ticks: u32,
    pub dead_ticks: u32,
    path: Vec<GridCell<2>>,
    waypoint: usize,
    attack_cooldown: u32,
}
impl Zombie {
    pub fn alive(&self) -> bool {
        self.health > 0.
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Shot {
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub ticks: u32,
}
pub struct World {
    pub tick: u64,
    pub player: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub health: f32,
    pub weapon: Weapon,
    pub ammo: [u32; 2],
    pub reload_ticks: u32,
    pub recoil: f32,
    pub muzzle_ticks: u32,
    pub hurt_ticks: u32,
    pub hit_marker: u32,
    pub kills: u32,
    pub shots_fired: u32,
    pub wave: u32,
    pub flashlight: bool,
    pub zombies: Vec<Zombie>,
    scene: Scene,
    grounded: bool,
    pub shot: Option<Shot>,
    pub distance_walked: f32,
    pub velocity_y: f32,
    seed: u64,
    rng: u64,
    cooldown: u32,
    next_wave: u32,
}
impl World {
    pub fn new(seed: u64) -> Self {
        let scene = Scene::new(props::courtyard()).expect("valid courtyard collision definitions");
        let mut w = Self {
            tick: 0,
            player: [0., EYE_HEIGHT + SKIN * 2., 10.],
            yaw: 0.,
            pitch: 0.,
            health: 100.,
            weapon: Weapon::Pistol,
            ammo: [12, 30],
            reload_ticks: 0,
            recoil: 0.,
            muzzle_ticks: 0,
            hurt_ticks: 0,
            hit_marker: 0,
            kills: 0,
            shots_fired: 0,
            wave: 1,
            flashlight: true,
            zombies: vec![],
            scene,
            grounded: true,
            shot: None,
            distance_walked: 0.,
            velocity_y: 0.,
            seed,
            rng: seed.max(1),
            cooldown: 0,
            next_wave: 0,
        };
        for position in [
            [-6., -8.],
            [-2., -10.],
            [3., -8.],
            [7., -4.],
            [-8., 2.],
            [3., 5.],
        ] {
            w.spawn(position);
        }
        w
    }
    pub fn props(&self) -> &[Prop] {
        &self.scene.props
    }
    pub fn collision(&self) -> &CollisionWorld {
        &self.scene.world
    }
    pub fn navigation(&self) -> &quasar_nav::physics3d::CollisionGrid3 {
        &self.scene.nav
    }
    /// Stages all geometry and navigation before replacing the active set.
    pub fn set_props(&mut self, props: Vec<Prop>) -> Result<(), ShapeError3> {
        let scene = Scene::new(props)?;
        self.scene = scene;
        for z in &mut self.zombies {
            z.path.clear();
            z.waypoint = 0;
        }
        Ok(())
    }
    pub fn raycast_world(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        distance: f32,
    ) -> Option<RayHit> {
        self.scene.world.raycast(origin, direction, distance, true)
    }
    pub fn forward(&self) -> [f32; 3] {
        let c = self.pitch.cos();
        [self.yaw.sin() * c, self.pitch.sin(), -self.yaw.cos() * c]
    }
    pub fn look(&mut self, delta: [f32; 2]) {
        let clean = |v: f32| if v.is_finite() { v } else { 0. };
        self.yaw = (self.yaw + clean(delta[0]) * 0.0025).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch - clean(delta[1]) * 0.0025).clamp(-1.35, 1.35);
    }
    pub fn ammo(&self) -> u32 {
        self.ammo[self.weapon as usize]
    }
    pub fn game_over(&self) -> bool {
        self.health <= 0.
    }
    pub fn living(&self) -> usize {
        self.zombies.iter().filter(|z| z.alive()).count()
    }
    pub fn update(&mut self, input: Input) {
        if input.restart {
            *self = Self::new(self.seed);
            return;
        }
        self.tick += 1;
        self.recoil *= 0.78;
        self.muzzle_ticks = self.muzzle_ticks.saturating_sub(1);
        self.hurt_ticks = self.hurt_ticks.saturating_sub(1);
        self.hit_marker = self.hit_marker.saturating_sub(1);
        self.cooldown = self.cooldown.saturating_sub(1);
        if let Some(shot) = &mut self.shot {
            shot.ticks = shot.ticks.saturating_sub(1);
            if shot.ticks == 0 {
                self.shot = None;
            }
        }
        if self.game_over() {
            return;
        }
        let clean = |v: f32| if v.is_finite() { v } else { 0. };
        self.look(input.look);
        if input.flashlight {
            self.flashlight = !self.flashlight;
        }
        if let Some(weapon) = input.weapon
            && weapon != self.weapon
        {
            self.weapon = weapon;
            self.reload_ticks = 0;
            self.cooldown = 12;
            self.recoil = 0.4;
        }
        if self.reload_ticks > 0 {
            self.reload_ticks -= 1;
            if self.reload_ticks == 0 {
                self.ammo[self.weapon as usize] = self.weapon.capacity();
            }
        } else if input.reload && self.ammo() < self.weapon.capacity() {
            self.reload_ticks = if self.weapon == Weapon::Pistol {
                75
            } else {
                110
            };
        }
        if input.fire && self.cooldown == 0 && self.reload_ticks == 0 {
            if self.ammo() == 0 {
                self.reload_ticks = 90;
            } else {
                self.fire();
            }
        }
        let f = clean(input.forward).clamp(-1., 1.);
        let s = clean(input.strafe).clamp(-1., 1.);
        let length = (f * f + s * s).sqrt().max(1.);
        let speed = if input.sprint { 5.5 } else { 3.5 };
        let delta = [
            (self.yaw.sin() * f + self.yaw.cos() * s) / length * speed * DT,
            (-self.yaw.cos() * f + self.yaw.sin() * s) / length * speed * DT,
        ];
        let before = [self.player[0], self.player[2]];
        if input.jump && self.grounded {
            self.velocity_y = 4.2;
        }
        self.velocity_y -= 10. * DT;
        let desired = [delta[0], self.velocity_y * DT, delta[1]];
        let motion = self
            .scene
            .player
            .move_feet(
                &self.scene.world,
                [self.player[0], self.player[1] - EYE_HEIGHT, self.player[2]],
                desired,
                DT,
                QueryFilter::default(),
            )
            .expect("finite player motion");
        self.player = [motion.feet[0], motion.feet[1] + EYE_HEIGHT, motion.feet[2]];
        self.grounded = motion.grounded;
        if (self.velocity_y < 0. && motion.grounded)
            || (self.velocity_y > 0. && motion.translation[1] + 1e-4 < desired[1])
        {
            self.velocity_y = 0.;
        }
        let moved = [self.player[0], self.player[2]];
        self.distance_walked += distance(before, moved);
        self.update_zombies();
        self.zombies.retain(|z| z.alive() || z.dead_ticks < 110);
        if self.living() == 0 {
            self.next_wave += 1;
            if self.next_wave >= 150 {
                self.wave += 1;
                self.next_wave = 0;
                self.health = (self.health + 25.).min(100.);
                for _ in 0..(self.wave as usize * 2 + 4).min(24) {
                    let angle = self.random() * std::f32::consts::TAU;
                    let p = [angle.sin() * 12.5, angle.cos() * 12.5];
                    if let Some(cell) = self.scene.nav.nearest_free(self.scene.nav.cell(p)) {
                        let point = self.scene.nav.point(cell);
                        self.spawn([point[0], point[2]]);
                    }
                }
            }
        }
    }
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / 16777216.
    }
    fn spawn(&mut self, position: [f32; 2]) {
        let position = if self.scene.zombie.overlaps(
            &self.scene.world,
            [position[0], SKIN * 2., position[1]],
            QueryFilter::default(),
        ) {
            let Some(cell) = self.scene.nav.nearest_free(self.scene.nav.cell(position)) else {
                return;
            };
            let p = self.scene.nav.point(cell);
            [p[0], p[2]]
        } else {
            position
        };
        self.zombies.push(Zombie {
            position,
            feet_y: SKIN * 2.,
            yaw: 0.,
            health: 100.,
            gait: 0.,
            hit_ticks: 0,
            dead_ticks: 0,
            path: vec![],
            waypoint: 0,
            attack_cooldown: 0,
        });
    }
    fn fire(&mut self) {
        self.ammo[self.weapon as usize] -= 1;
        self.cooldown = self.weapon.cooldown();
        self.recoil = 1.;
        self.muzzle_ticks = 4;
        self.shots_fired += 1;
        let direction = self.forward();
        let mut best = self
            .raycast_world(self.player, direction, 60.)
            .map_or(60., |hit| hit.distance);
        let mut hit = None;
        for (i, z) in self.zombies.iter().enumerate().filter(|(_, z)| z.alive()) {
            let body = self.scene.zombie.ray_distance(
                [z.position[0], z.feet_y, z.position[1]],
                self.player,
                direction,
                best,
            );
            if let Some(t) = body
                && t < best
            {
                best = t;
                hit = Some((i, self.player[1] + direction[1] * t > z.feet_y + 1.4));
            }
        }
        if let Some((i, head)) = hit {
            let z = &mut self.zombies[i];
            z.health -= self.weapon.damage() * if head { 2.5 } else { 1. };
            z.hit_ticks = 9;
            self.hit_marker = 9;
            if !z.alive() {
                self.kills += 1;
            }
        }
        self.shot = Some(Shot {
            from: self.player,
            to: std::array::from_fn(|i| self.player[i] + direction[i] * best),
            ticks: 3,
        });
    }
    fn update_zombies(&mut self) {
        let player = [self.player[0], self.player[2]];
        let Some(goal) = self.scene.nav.nearest_free(self.scene.nav.cell(player)) else {
            return;
        };
        for (i, z) in self.zombies.iter_mut().enumerate() {
            z.hit_ticks = z.hit_ticks.saturating_sub(1);
            z.attack_cooldown = z.attack_cooldown.saturating_sub(1);
            if !z.alive() {
                z.dead_ticks += 1;
                continue;
            }
            let gap = distance(z.position, player);
            if (self.tick + i as u64 * 7).is_multiple_of(40) || z.path.is_empty() {
                let Some(start) = self.scene.nav.nearest_reachable(
                    &self.scene.world,
                    &self.scene.zombie,
                    [z.position[0], z.feet_y, z.position[1]],
                    QueryFilter::default(),
                ) else {
                    continue;
                };
                if let SearchOutcome::Found(path) = self.scene.nav.find_path(
                    start,
                    goal,
                    SearchOptions {
                        max_visited: 8192,
                        ..SearchOptions::default()
                    },
                ) {
                    z.path = path.nodes;
                    z.waypoint = 1;
                }
            }
            let feet = [z.position[0], z.feet_y, z.position[1]];
            let direct = self.scene.zombie.clear_segment(
                &self.scene.world,
                feet,
                [player[0], z.feet_y, player[1]],
                QueryFilter::default(),
            );
            while z.waypoint + 1 < z.path.len() {
                let point = self.scene.nav.point(z.path[z.waypoint + 1]);
                if !self.scene.zombie.clear_segment(
                    &self.scene.world,
                    feet,
                    point,
                    QueryFilter::default(),
                ) {
                    break;
                }
                z.waypoint += 1;
            }
            let target = if direct {
                player
            } else {
                z.path
                    .get(z.waypoint)
                    .copied()
                    .map(|cell| {
                        let p = self.scene.nav.point(cell);
                        [p[0], p[2]]
                    })
                    .unwrap_or(z.position)
            };
            let delta = [target[0] - z.position[0], target[1] - z.position[1]];
            let len = distance(z.position, target);
            if len < 0.2 {
                z.waypoint = (z.waypoint + 1).min(z.path.len());
            }
            if len > 0.05 && gap > 0.85 {
                let speed = (0.75 + self.wave as f32 * 0.08).min(1.9);
                let motion = self
                    .scene
                    .zombie
                    .move_feet(
                        &self.scene.world,
                        feet,
                        [
                            delta[0] / len * speed * DT,
                            -0.02,
                            delta[1] / len * speed * DT,
                        ],
                        DT,
                        QueryFilter::default(),
                    )
                    .expect("finite zombie motion");
                z.position = [motion.feet[0], motion.feet[2]];
                z.feet_y = motion.feet[1];
                z.yaw = delta[0].atan2(delta[1]);
                z.gait += speed * DT * 7.;
            }
            if gap < 1.05 && direct && z.attack_cooldown == 0 {
                z.attack_cooldown = 55;
                self.health = (self.health - 10.).max(0.);
                self.hurt_ticks = 20;
            }
        }
    }
    /// Deterministic within the same floating-point platform; covers gameplay state.
    pub fn state_hash(&self) -> u64 {
        let mut h = 0xcbf29ce484222325u64;
        let mut write = |v: u64| {
            for b in v.to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x100000001b3);
            }
        };
        for n in [
            self.tick,
            self.rng,
            self.kills as u64,
            self.wave as u64,
            self.shots_fired as u64,
            self.cooldown as u64,
            self.reload_ticks as u64,
            self.next_wave as u64,
            self.flashlight as u64,
            self.seed,
            self.scene.fingerprint,
            self.grounded as u64,
            self.muzzle_ticks as u64,
            self.hurt_ticks as u64,
            self.hit_marker as u64,
            self.weapon as u64,
            self.ammo[0] as u64,
            self.ammo[1] as u64,
        ] {
            write(n);
        }
        for f in self.player.into_iter().chain([
            self.yaw,
            self.pitch,
            self.health,
            self.velocity_y,
            self.recoil,
            self.distance_walked,
        ]) {
            write(f.to_bits() as u64);
        }
        for z in &self.zombies {
            for f in z.position.into_iter().chain([z.feet_y, z.health, z.gait]) {
                write(f.to_bits() as u64);
            }
            write(z.yaw.to_bits() as u64);
            write(z.hit_ticks as u64);
            write(z.dead_ticks as u64);
            write(z.waypoint as u64);
            write(z.attack_cooldown as u64);
            for cell in &z.path {
                write(cell.coords[0] as u64);
                write(cell.coords[1] as u64);
            }
        }
        if let Some(shot) = self.shot {
            write(1);
            write(shot.ticks as u64);
            for f in shot.from.into_iter().chain(shot.to) {
                write(f.to_bits() as u64);
            }
        } else {
            write(0);
        }
        h
    }
}
fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}
pub fn autoplay(w: &World) -> Input {
    let target = w.zombies.iter().filter(|z| z.alive()).min_by(|a, b| {
        distance(a.position, [w.player[0], w.player[2]])
            .total_cmp(&distance(b.position, [w.player[0], w.player[2]]))
    });
    let mut look = [0.; 2];
    if let Some(z) = target {
        let desired = (z.position[0] - w.player[0]).atan2(w.player[2] - z.position[1]);
        let delta = (desired - w.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        look = [
            delta.clamp(-0.08, 0.08) / 0.0025,
            (w.pitch - (-0.02)) / 0.0025,
        ];
    }
    Input {
        look,
        fire: true,
        reload: w.ammo() == 0,
        weapon: (w.tick == 120).then_some(Weapon::Rifle),
        ..Input::default()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shots_hit_heads_and_obstacles_block_them() {
        let mut w = World::new(1);
        w.set_props(vec![]).unwrap();
        w.zombies.clear();
        w.spawn([0., 5.]);
        w.update(Input {
            fire: true,
            ..Input::default()
        });
        assert_eq!(w.kills, 1);
        assert_eq!(w.ammo(), 11);
        assert!(w.muzzle_ticks > 0);
        let mut w = World::new(1);
        w.zombies.clear();
        w.spawn([0., 5.]);
        let mut props = w.props().to_vec();
        props.push(Prop::cuboid(
            999,
            "test blocker",
            [0., 1., 7.],
            [1., 1., 0.5],
            [1.; 3],
        ));
        w.set_props(props).unwrap();
        w.update(Input {
            fire: true,
            ..Input::default()
        });
        assert_eq!(w.kills, 0);
        assert_eq!(w.zombies[0].health, 100.);
    }
    #[test]
    fn ammo_reload_switching_and_cooldown_are_real_game_rules() {
        let mut w = World::new(1);
        for _ in 0..20 {
            w.update(Input {
                fire: true,
                ..Input::default()
            });
        }
        assert_eq!(w.shots_fired, 2);
        w.update(Input {
            reload: true,
            ..Input::default()
        });
        assert!(w.reload_ticks > 0);
        let before = w.shots_fired;
        for _ in 0..60 {
            w.update(Input {
                fire: true,
                ..Input::default()
            });
        }
        assert_eq!(w.shots_fired, before);
        for _ in 0..20 {
            w.update(Input::default());
        }
        assert_eq!(w.ammo(), 12);
        w.update(Input {
            weapon: Some(Weapon::Rifle),
            ..Input::default()
        });
        assert_eq!(w.ammo(), 30);
    }
    #[test]
    fn collision_and_navigation_keep_actors_out_of_obstacles() {
        let mut w = World::new(1);
        w.player = [-5., EYE_HEIGHT, 2.];
        for _ in 0..120 {
            w.update(Input {
                forward: 1.,
                ..Input::default()
            });
        }
        assert!(w.player[2] >= 0.3);
        for z in w.zombies.iter().filter(|z| z.alive()) {
            assert!(!w.scene.zombie.overlaps(
                &w.scene.world,
                [z.position[0], z.feet_y, z.position[1]],
                QueryFilter::default()
            ));
        }
    }
    #[test]
    fn scripted_runs_repeat_and_enemies_approach() {
        let (mut a, mut b) = (World::new(42), World::new(42));
        let old = a.zombies[0].position;
        for _ in 0..600 {
            let input = autoplay(&a);
            a.update(input);
            b.update(input);
            assert_eq!(a.state_hash(), b.state_hash());
        }
        assert!(a.shots_fired > 10);
        assert!(a.kills > 0);
        assert_ne!(a.zombies.first().map(|z| z.position), Some(old));
    }
    #[test]
    fn wave_progression_damage_death_and_restart_are_bounded() {
        let mut w = World::new(7);
        w.zombies.clear();
        w.health = 70.;
        for _ in 0..150 {
            w.update(Input::default());
        }
        assert_eq!(w.wave, 2);
        assert_eq!(w.living(), 8);
        assert_eq!(w.health, 95.);
        w.zombies[0].position = [w.player[0], w.player[2]];
        w.update(Input::default());
        assert_eq!(w.health, 85.);
        w.health = 0.;
        let shots = w.shots_fired;
        w.update(Input {
            fire: true,
            ..Input::default()
        });
        assert_eq!(w.shots_fired, shots);
        w.update(Input {
            restart: true,
            ..Input::default()
        });
        assert_eq!(w.state_hash(), World::new(7).state_hash());
    }
}
