//! The walkable graveyard: player movement, collision and layout.
use quasar_core::noise::{hash2, unit_f32};

pub const TICK_RATE: f32 = 60.0;
/// Half the walled area, in metres.
pub const HALF: f32 = 22.0;
pub const EYE: f32 = 1.65;
const RADIUS: f32 = 0.3;

/// One frame of player intent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    pub forward: f32,
    pub strafe: f32,
    /// Yaw and pitch deltas in radians.
    pub look: [f32; 2],
    pub sprint: bool,
    pub toggle_flashlight: bool,
}

/// What a scene piece is, for drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Headstone,
    Cross,
    Slab,
    Obelisk,
    Tree,
    Lantern,
    Mausoleum,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    pub kind: Kind,
    /// Ground position (x, z).
    pub at: [f32; 2],
    pub yaw: f32,
    /// Lean in radians (old, sinking stones).
    pub tilt: f32,
    pub scale: f32,
    /// Collision half extents (x, z) around `at`, unrotated.
    pub half: [f32; 2],
}

pub struct World {
    pub tick: u64,
    /// Feet position.
    pub position: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub flashlight: bool,
    pub pieces: Vec<Piece>,
}

fn rand(x: i32, z: i32, salt: u32) -> f32 {
    unit_f32(hash2(0x9a7e ^ salt, x, z))
}

/// The deterministic graveyard layout.
pub fn layout() -> Vec<Piece> {
    let mut pieces = Vec::new();
    // Rows of graves either side of a central path.
    for row in 0..9 {
        for column in -6..=6 {
            if column == 0 {
                continue; // the path
            }
            let (gx, gz) = (column, row);
            if rand(gx, gz, 1) < 0.18 {
                continue; // an empty plot
            }
            let x = column as f32 * 2.7 + (rand(gx, gz, 2) - 0.5) * 0.6;
            let z = 8.0 - row as f32 * 3.1 + (rand(gx, gz, 3) - 0.5) * 0.5;
            let pick = rand(gx, gz, 4);
            let kind = if pick < 0.5 {
                Kind::Headstone
            } else if pick < 0.75 {
                Kind::Cross
            } else if pick < 0.92 {
                Kind::Slab
            } else {
                Kind::Obelisk
            };
            let half = match kind {
                Kind::Slab => [0.55, 1.1],
                Kind::Obelisk => [0.35, 0.35],
                _ => [0.45, 0.2],
            };
            pieces.push(Piece {
                kind,
                at: [x, z],
                yaw: (rand(gx, gz, 5) - 0.5) * 0.3,
                tilt: (rand(gx, gz, 6) - 0.5) * 0.35,
                scale: 0.85 + rand(gx, gz, 7) * 0.35,
                half,
            });
        }
    }
    // Dead trees around the edge.
    for i in 0..9 {
        let angle = i as f32 / 9.0 * std::f32::consts::TAU + rand(i, 0, 8);
        let r = 16.5 + rand(i, 1, 9) * 3.0;
        pieces.push(Piece {
            kind: Kind::Tree,
            at: [angle.cos() * r, angle.sin() * r - 2.0],
            yaw: rand(i, 2, 10) * std::f32::consts::TAU,
            tilt: (rand(i, 3, 11) - 0.5) * 0.15,
            scale: 0.9 + rand(i, 4, 12) * 0.5,
            half: [0.35, 0.35],
        });
    }
    // Lanterns along the path and by the mausoleum.
    for (i, z) in [9.0f32, 2.5, -4.0, -10.5].into_iter().enumerate() {
        let side = if i % 2 == 0 { 1.2 } else { -1.2 };
        pieces.push(Piece {
            kind: Kind::Lantern,
            at: [side, z],
            yaw: 0.0,
            tilt: 0.0,
            scale: 1.0,
            half: [0.15, 0.15],
        });
    }
    pieces.push(Piece {
        kind: Kind::Mausoleum,
        at: [0.0, -17.0],
        yaw: 0.0,
        tilt: 0.0,
        scale: 1.0,
        half: [2.6, 2.2],
    });
    pieces
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    pub fn new() -> Self {
        Self {
            tick: 0,
            position: [0.0, 0.0, 16.0],
            yaw: 0.0,
            pitch: -0.06,
            flashlight: true,
            pieces: layout(),
        }
    }

    /// Eye position.
    pub fn eye(&self) -> [f32; 3] {
        [self.position[0], self.position[1] + EYE, self.position[2]]
    }

    /// Unit view direction (yaw 0 looks down -Z).
    pub fn forward(&self) -> [f32; 3] {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        [-sy * cp, sp, -cy * cp]
    }

    fn blocked(&self, x: f32, z: f32) -> bool {
        if x.abs() > HALF - RADIUS || z.abs() > HALF - RADIUS {
            return true;
        }
        self.pieces.iter().any(|p| {
            (x - p.at[0]).abs() < p.half[0] + RADIUS && (z - p.at[1]).abs() < p.half[1] + RADIUS
        })
    }

    pub fn update(&mut self, input: Input) {
        self.tick += 1;
        if input.toggle_flashlight {
            self.flashlight = !self.flashlight;
        }
        self.yaw = (self.yaw + input.look[0]).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + input.look[1]).clamp(-1.4, 1.4);
        let speed = if input.sprint { 4.2 } else { 2.2 } / TICK_RATE;
        let (sy, cy) = self.yaw.sin_cos();
        let mut dx = -sy * input.forward + cy * input.strafe;
        let mut dz = -cy * input.forward - sy * input.strafe;
        let length = (dx * dx + dz * dz).sqrt();
        if length > 1.0 {
            dx /= length;
            dz /= length;
        }
        // Slide along obstacles one axis at a time.
        let (x, z) = (self.position[0], self.position[2]);
        if !self.blocked(x + dx * speed, z) {
            self.position[0] += dx * speed;
        }
        if !self.blocked(self.position[0], z + dz * speed) {
            self.position[2] += dz * speed;
        }
    }
}

/// A slow scripted walk up the path (headless screenshots and tests).
pub fn autoplay(world: &World) -> Input {
    let t = world.tick as f32 / TICK_RATE;
    Input {
        forward: if t < 3.0 { 0.6 } else { 0.0 },
        look: [(t * 0.7).sin() * 0.002, 0.0],
        ..Input::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_is_deterministic_and_leaves_the_path_open() {
        assert_eq!(layout(), layout());
        let world = World::new();
        assert!(world.pieces.len() > 80);
        // Walk the whole path to the mausoleum steps.
        for z in -14..16 {
            assert!(!world.blocked(0.0, z as f32), "path blocked at {z}");
        }
    }

    #[test]
    fn players_walk_and_collide() {
        let mut world = World::new();
        let start = world.position;
        for _ in 0..120 {
            world.update(Input {
                forward: 1.0,
                ..Input::default()
            });
        }
        assert!(world.position[2] < start[2] - 3.0, "walked forward");
        // Walking into a grave stops at its edge.
        let grave = world
            .pieces
            .iter()
            .find(|p| p.kind == Kind::Headstone)
            .copied()
            .unwrap();
        world.position = [grave.at[0], 0.0, grave.at[1] + grave.half[1] + 1.0];
        world.yaw = 0.0;
        for _ in 0..600 {
            world.update(Input {
                forward: 1.0,
                ..Input::default()
            });
        }
        assert!(world.position[2] > grave.at[1] + grave.half[1]);
        // The walls hold.
        world.position = [0.0, 0.0, 0.0];
        world.yaw = std::f32::consts::FRAC_PI_2;
        for _ in 0..5000 {
            world.update(Input {
                forward: 1.0,
                sprint: true,
                ..Input::default()
            });
        }
        assert!(world.position[0].abs() < HALF);
        world.update(Input {
            toggle_flashlight: true,
            ..Input::default()
        });
        assert!(!world.flashlight);
    }
}
