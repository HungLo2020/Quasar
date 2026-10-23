//! Pong's simulation and software renderer. No window, Vulkan, Qt, or device I/O.
pub mod render;
use serde::{Deserialize, Serialize};
pub const WIDTH: u32 = 640;
pub const HEIGHT: u32 = 400;
pub const TICK_HZ: f64 = 120.0;
pub const PADDLE_HEIGHT: f64 = 52.0;
pub const PADDLE_SPEED: f64 = 290.0;
pub const BALL_RADIUS: f64 = 4.0;
pub const TOP: f64 = 68.0;
pub const BOTTOM: f64 = 344.0;
pub const WIN_SCORE: u32 = 7;
const LEFT_FACE: f64 = 40.0;
const RIGHT_FACE: f64 = 600.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Mode {
    #[default]
    Solo,
    Local,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Ready,
    Playing,
    Paused,
    Finished,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Input {
    pub left: i8,
    pub right: i8,
    pub serve: bool,
    pub pause: bool,
    pub restart: bool,
}
impl Input {
    pub fn valid(self) -> bool {
        (-1..=1).contains(&self.left) && (-1..=1).contains(&self.right)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Game {
    pub mode: Mode,
    pub phase: Phase,
    pub paddles: [f64; 2],
    pub ball: [f64; 2],
    pub velocity: [f64; 2],
    pub score: [u32; 2],
    pub rally: u32,
    pub longest_rally: u32,
    pub ticks: u64,
    serve_direction: f64,
}
impl Game {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            phase: Phase::Ready,
            paddles: [(TOP + BOTTOM) / 2.0; 2],
            ball: [WIDTH as f64 / 2.0, (TOP + BOTTOM) / 2.0],
            velocity: [0.0; 2],
            score: [0; 2],
            rally: 0,
            longest_rally: 0,
            ticks: 0,
            serve_direction: 1.0,
        }
    }
    pub fn tick(&mut self, input: Input) {
        self.ticks += 1;
        if input.restart {
            let ticks = self.ticks;
            *self = Self::new(self.mode);
            self.ticks = ticks;
            return;
        }
        if input.pause {
            self.phase = match self.phase {
                Phase::Playing => Phase::Paused,
                Phase::Paused => Phase::Playing,
                other => other,
            };
            return;
        }
        if matches!(self.phase, Phase::Paused | Phase::Finished) {
            return;
        }
        let dt = 1.0 / TICK_HZ;
        let right = if self.mode == Mode::Solo {
            let target = if self.velocity[0] > 0.0 {
                self.ball[1]
            } else {
                (TOP + BOTTOM) / 2.0
            };
            direction(target - self.paddles[1], 9.0)
        } else {
            input.right.clamp(-1, 1)
        };
        for (i, axis) in [input.left.clamp(-1, 1), right].into_iter().enumerate() {
            let speed = if i == 1 && self.mode == Mode::Solo {
                215.0
            } else {
                PADDLE_SPEED
            };
            self.paddles[i] = (self.paddles[i] + axis as f64 * speed * dt)
                .clamp(TOP + PADDLE_HEIGHT / 2.0, BOTTOM - PADDLE_HEIGHT / 2.0);
        }
        if self.phase == Phase::Ready {
            if input.serve {
                self.velocity = [
                    self.serve_direction * 230.0,
                    if self.score.iter().sum::<u32>().is_multiple_of(2) {
                        85.0
                    } else {
                        -85.0
                    },
                ];
                self.phase = Phase::Playing;
            }
            return;
        }
        let previous = self.ball;
        self.ball[0] += self.velocity[0] * dt;
        self.ball[1] += self.velocity[1] * dt;
        if self.ball[1] < TOP + BALL_RADIUS {
            self.ball[1] = 2.0 * (TOP + BALL_RADIUS) - self.ball[1];
            self.velocity[1] = self.velocity[1].abs();
        } else if self.ball[1] > BOTTOM - BALL_RADIUS {
            self.ball[1] = 2.0 * (BOTTOM - BALL_RADIUS) - self.ball[1];
            self.velocity[1] = -self.velocity[1].abs();
        }
        for (i, face, sign) in [(0, LEFT_FACE, 1.0), (1, RIGHT_FACE, -1.0)] {
            let crossing = if i == 0 {
                self.velocity[0] < 0.0 && previous[0] >= face && self.ball[0] <= face
            } else {
                self.velocity[0] > 0.0 && previous[0] <= face && self.ball[0] >= face
            };
            if !crossing {
                continue;
            }
            let fraction = (face - previous[0]) / (self.ball[0] - previous[0]);
            let y = previous[1] + fraction * (self.ball[1] - previous[1]);
            let offset = y - self.paddles[i];
            if offset.abs() <= PADDLE_HEIGHT / 2.0 + BALL_RADIUS {
                let angle = (offset / (PADDLE_HEIGHT / 2.0)).clamp(-1.0, 1.0) * 0.95;
                let speed = (self.velocity[0].hypot(self.velocity[1]) + 18.0).min(510.0);
                self.velocity = [sign * speed * angle.cos(), speed * angle.sin()];
                self.ball[0] = face + sign * (self.ball[0] - face).abs();
                self.rally += 1;
                self.longest_rally = self.longest_rally.max(self.rally);
            }
        }
        if self.ball[0] < -BALL_RADIUS {
            self.point(1);
        } else if self.ball[0] > WIDTH as f64 + BALL_RADIUS {
            self.point(0);
        }
    }
    fn point(&mut self, player: usize) {
        self.score[player] += 1;
        self.rally = 0;
        self.ball = [WIDTH as f64 / 2.0, (TOP + BOTTOM) / 2.0];
        self.velocity = [0.0; 2];
        self.serve_direction = if player == 0 { -1.0 } else { 1.0 };
        self.phase = if self.score[player] >= WIN_SCORE {
            Phase::Finished
        } else {
            Phase::Ready
        };
    }
    /// Deterministic exercise driver, shared by headless and native smoke runs.
    pub fn autoplay(&self) -> Input {
        Input {
            left: direction(self.ball[1] - self.paddles[0], 5.0),
            right: direction(self.ball[1] - self.paddles[1], 5.0),
            serve: self.phase == Phase::Ready,
            restart: self.phase == Phase::Finished,
            ..Default::default()
        }
    }
}
fn direction(delta: f64, deadzone: f64) -> i8 {
    if delta > deadzone {
        1
    } else if delta < -deadzone {
        -1
    } else {
        0
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Replay {
    pub version: u32,
    pub mode: Mode,
    pub tick_hz: f64,
    pub inputs: Vec<Input>,
}
impl Replay {
    pub fn new(mode: Mode) -> Self {
        Self {
            version: 1,
            mode,
            tick_hz: TICK_HZ,
            inputs: Vec::new(),
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != 1 || self.tick_hz != TICK_HZ {
            return Err("unsupported replay version or tick rate");
        }
        if self.inputs.iter().any(|input| !input.valid()) {
            return Err("invalid replay input axis");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wall_and_paddle_collisions_score_and_match_end() {
        let mut game = Game::new(Mode::Local);
        game.phase = Phase::Playing;
        game.ball = [100.0, TOP + 4.1];
        game.velocity = [-230.0, -85.0];
        game.tick(Input::default());
        assert!(game.velocity[1] > 0.0);
        assert!(game.ball[1] >= TOP + BALL_RADIUS);
        game.ball = [LEFT_FACE + 0.1, game.paddles[0]];
        game.velocity = [-300.0, 0.0];
        game.tick(Input::default());
        assert!(game.velocity[0] > 0.0);
        assert_eq!(game.rally, 1);
        game.score[0] = WIN_SCORE - 1;
        game.ball = [WIDTH as f64 + 3.9, 100.0];
        game.velocity = [300.0, 0.0];
        game.tick(Input::default());
        assert_eq!(game.score[0], WIN_SCORE);
        assert_eq!(game.phase, Phase::Finished);
        let state = game.ball;
        game.tick(Input {
            serve: true,
            ..Default::default()
        });
        assert_eq!(game.ball, state);
    }
    #[test]
    fn pause_freezes_simulation_and_paddles_are_bounded() {
        let mut game = Game::new(Mode::Local);
        game.tick(Input {
            serve: true,
            ..Default::default()
        });
        game.tick(Input {
            pause: true,
            ..Default::default()
        });
        let ball = game.ball;
        let paddles = game.paddles;
        game.tick(Input {
            left: 1,
            right: -1,
            ..Default::default()
        });
        assert_eq!(game.ball, ball);
        assert_eq!(game.paddles, paddles);
        game.tick(Input {
            pause: true,
            ..Default::default()
        });
        for _ in 0..1000 {
            game.tick(Input {
                left: 1,
                right: -1,
                ..Default::default()
            });
        }
        assert!(game.paddles[0] <= BOTTOM - PADDLE_HEIGHT / 2.0);
        assert!(game.paddles[1] >= TOP + PADDLE_HEIGHT / 2.0);
    }
    #[test]
    fn replay_reproduces_state_and_pixels() {
        let mut original = Game::new(Mode::Solo);
        let mut replay = Replay::new(Mode::Solo);
        for _ in 0..12000 {
            let input = original.autoplay();
            original.tick(input);
            replay.inputs.push(input);
        }
        assert!(original.longest_rally > 0);
        let json = serde_json::to_vec(&replay).unwrap();
        let replay: Replay = serde_json::from_slice(&json).unwrap();
        replay.validate().unwrap();
        let mut played = Game::new(replay.mode);
        for input in replay.inputs {
            played.tick(input);
        }
        assert_eq!(original, played);
        assert_eq!(render::frame(&original), render::frame(&played));
    }
}
