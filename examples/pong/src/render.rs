//! Ordered 2D drawing shared by GPU presentation and headless reference snapshots.
use crate::*;
const BACK: [u8; 4] = [12, 18, 28, 255];
const PANEL: [u8; 4] = [16, 25, 37, 255];
const LINE: [u8; 4] = [37, 53, 68, 255];
const WHITE: [u8; 4] = [228, 237, 239, 255];
const MUTED: [u8; 4] = [123, 148, 161, 255];
const CYAN: [u8; 4] = [92, 230, 204, 255];
const CORAL: [u8; 4] = [255, 150, 117, 255];
/// Presentation labels supplied by the application, independent of input devices.
pub struct ControlHints {
    pub left: String,
    pub right: String,
    pub serve: String,
    pub pause: String,
    pub restart: String,
    pub fullscreen: String,
    pub filter: String,
    pub scale: String,
}
impl Default for ControlHints {
    fn default() -> Self {
        Self {
            left: "W/S".into(),
            right: "UP/DOWN".into(),
            serve: "SPACE".into(),
            pause: "P".into(),
            restart: "R".into(),
            fullscreen: "F11".into(),
            filter: "F2".into(),
            scale: "F3".into(),
        }
    }
}
pub fn frame(game: &Game) -> Vec<u8> {
    frame_with_hints(game, &ControlHints::default())
}
pub fn frame_with_hints(game: &Game, hints: &ControlHints) -> Vec<u8> {
    quasar_render_2d::reference::rasterize(
        &scene_with_hints(game, hints),
        &quasar_render_2d::Textures::default(),
    )
    .expect("valid game draw list")
}
pub fn scene_with_hints(game: &Game, hints: &ControlHints) -> quasar_render_2d::Canvas {
    let mut c =
        Canvas(quasar_render_2d::Canvas::new(WIDTH, HEIGHT).expect("valid game dimensions"));
    c.rect(0, 0, WIDTH as i32, HEIGHT as i32, BACK);
    c.text(24, 15, "PONG", 3, WHITE);
    c.text(25, 43, "QUASAR EXAMPLE 01", 1, MUTED);
    c.text(283, 18, &game.score[0].to_string(), 4, CYAN);
    c.text(316, 25, ":", 2, MUTED);
    c.text(337, 18, &game.score[1].to_string(), 4, CORAL);
    c.text(
        494,
        20,
        if game.mode == Mode::Solo {
            "SINGLE PLAYER"
        } else {
            "LOCAL / TWO"
        },
        1,
        WHITE,
    );
    c.text(494, 37, "FIRST TO SEVEN", 1, MUTED);
    c.rect(16, 64, 608, 284, LINE);
    c.rect(17, 65, 606, 282, PANEL);
    for y in (77..338).step_by(15) {
        c.rect(319, y, 2, 7, LINE);
    }
    for (i, x) in [(0, 28), (1, 604)] {
        let y = (game.paddles[i] - PADDLE_HEIGHT / 2.0).round() as i32;
        let color = if i == 0 { CYAN } else { CORAL };
        c.rect(
            x - 3,
            y - 3,
            14,
            PADDLE_HEIGHT as i32 + 6,
            [24, 45, 54, 255],
        );
        c.rect(x, y, 8, PADDLE_HEIGHT as i32, color);
        c.rect(x + 2, y + 3, 2, PADDLE_HEIGHT as i32 - 6, WHITE);
    }
    let (x, y) = (game.ball[0].round() as i32, game.ball[1].round() as i32);
    c.rect(x - 6, y - 6, 12, 12, [28, 57, 62, 255]);
    c.rect(x - 4, y - 4, 8, 8, WHITE);
    let overlay = match game.phase {
        Phase::Ready => Some(("READY?", format!("{} TO SERVE", hints.serve))),
        Phase::Paused => Some(("PAUSED", format!("{} TO RESUME", hints.pause))),
        Phase::Finished => Some((
            if game.score[0] >= WIN_SCORE {
                "LEFT WINS"
            } else {
                "RIGHT WINS"
            },
            format!("{} TO PLAY AGAIN", hints.restart),
        )),
        Phase::Playing => None,
    };
    if let Some((title, subtitle)) = overlay {
        c.rect(170, 143, 300, 100, LINE);
        c.rect(171, 144, 298, 98, BACK);
        c.rect(188, 157, 24, 2, CYAN);
        c.centered(171, title, 3, WHITE);
        c.centered(213, &subtitle, 1, MUTED);
    }
    c.label(24, 363, 140, &format!("{}  MOVE", hints.left), CYAN);
    c.label(
        171,
        363,
        170,
        &if game.mode == Mode::Solo {
            "VS COMPUTER".into()
        } else {
            format!("{}  MOVE", hints.right)
        },
        CORAL,
    );
    c.label(
        354,
        363,
        270,
        &format!("{} SERVE   {} PAUSE", hints.serve, hints.pause),
        MUTED,
    );
    c.label(
        24,
        381,
        380,
        &format!(
            "{} FULLSCREEN   {} FILTER   {} SCALE",
            hints.fullscreen, hints.filter, hints.scale
        ),
        MUTED,
    );
    c.label(
        411,
        381,
        213,
        &format!("{} RESET  RALLY {:02}", hints.restart, game.rally),
        MUTED,
    );
    c.0
}
struct Canvas(quasar_render_2d::Canvas);
impl Canvas {
    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: [u8; 4]) {
        self.0.rect(
            quasar_render_2d::Rect::new(x as f32, y as f32, w as f32, h as f32),
            color,
        );
    }
    fn label(&mut self, x: i32, y: i32, width: usize, text: &str, color: [u8; 4]) {
        let text: String = text.chars().take(width / 6).collect();
        self.text(x, y, &text, 1, color);
    }
    fn centered(&mut self, y: i32, text: &str, scale: i32, color: [u8; 4]) {
        let text: String = text.chars().take((290 / (6 * scale)) as usize).collect();
        self.text(
            (WIDTH as i32 - (text.len() as i32 * 6 - 1) * scale) / 2,
            y,
            &text,
            scale,
            color,
        );
    }
    fn text(&mut self, x: i32, y: i32, text: &str, scale: i32, color: [u8; 4]) {
        self.0.text([x as f32, y as f32], text, scale as f32, color);
    }
}

#[cfg(all(test, feature = "desktop"))]
#[test]
#[ignore = "requires Vulkan; run cargo test -- --include-ignored"]
fn gpu_game_scenes_match_headless_snapshots() {
    let textures = quasar_render_2d::Textures::default();
    let mut renderer = quasar_render_2d::Renderer2D::headless().unwrap();
    let mut game = Game::new(Mode::Solo);
    for phase in [Phase::Ready, Phase::Playing, Phase::Paused, Phase::Finished] {
        game.phase = phase;
        let canvas = scene_with_hints(&game, &ControlHints::default());
        let expected = quasar_render_2d::reference::rasterize(&canvas, &textures).unwrap();
        let actual = renderer.render_to_image(&canvas, &textures).unwrap();
        let differences = actual
            .iter()
            .zip(&expected)
            .filter(|(a, b)| a.abs_diff(**b) > 1)
            .count();
        assert_eq!(differences, 0, "GPU snapshot mismatch in {phase:?}");
        let stats = renderer.stats();
        assert!(stats.quads > 100);
        assert!(stats.batches < 16);
        assert!(stats.geometry_upload_bytes < expected.len() / 4);
    }
}
