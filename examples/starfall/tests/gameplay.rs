use quasar_diagnostics::Diagnostics;
use quasar_input::{InputState, Key, MouseButton};
use quasar_sim::{Recorder, Replay, Simulation};
use starfall::session::{GameSession, StepResult};
use starfall::settings::{GameCli, GameSettings};
use starfall::world::{
    DroneSpec, GameError, GameInput, GameRules, GameWorld, Level, Vec3, autoplay_input,
};
use std::path::PathBuf;

fn level() -> Level {
    Level {
        duration: 90.0,
        crystals: vec![
            [-5.0, 0.9, -2.0],
            [5.0, 0.9, -2.5],
            [-4.0, 0.9, 4.0],
            [4.0, 0.9, 4.5],
            [0.0, 0.9, -6.0],
        ],
        drones: vec![
            DroneSpec {
                radius: 6.0,
                height: 1.6,
                speed: 0.45,
                phase: 0.0,
            },
            DroneSpec {
                radius: 8.5,
                height: 2.2,
                speed: -0.32,
                phase: 1.8,
            },
        ],
        portal: [0.0, 0.5, 8.0],
    }
}

fn settings() -> GameSettings {
    GameSettings::load(
        &quasar_config::CommonArgs::default(),
        &GameCli {
            ticks: 600,
            ..GameCli::default()
        },
    )
    .unwrap()
}

#[test]
fn autoplay_collects_and_wins_within_the_level_budget() {
    let mut world = GameWorld::new(level(), GameRules::default()).unwrap();
    let mut collected = false;
    for _ in 0..1_200 {
        let input = autoplay_input(&world);
        world.update(input);
        collected |= world.score > 0;
        if world.finished() {
            break;
        }
    }
    assert!(collected, "autoplay never collected a crystal");
    assert!(
        world.won,
        "autoplay did not finish the objective: {world:?}"
    );
    assert_eq!(world.crystals_remaining(), 0);
}

#[test]
fn level_and_rule_inputs_are_validated() {
    let mut invalid = level();
    invalid.duration = 0.0;
    assert!(GameWorld::new(invalid, GameRules::default()).is_err());
    let mut invalid = level();
    invalid.crystals.clear();
    assert!(GameWorld::new(invalid, GameRules::default()).is_err());
    let mut invalid = level();
    invalid.portal = [99.0, 0.0, 0.0];
    assert!(GameWorld::new(invalid, GameRules::default()).is_err());
}

#[test]
fn snapshot_restore_returns_to_the_same_hash() {
    let mut world = GameWorld::new(level(), GameRules::default()).unwrap();
    for _ in 0..120 {
        world.update(autoplay_input(&world));
    }
    let snapshot = world.snapshot();
    let hash = world.state_hash();
    for _ in 0..120 {
        world.update(autoplay_input(&world));
    }
    assert_ne!(world.state_hash(), hash);
    world.restore(&snapshot).unwrap();
    assert_eq!(world.state_hash(), hash);
    assert_eq!(world.tick, snapshot.tick);
}

#[test]
fn live_recording_and_replay_produce_the_same_summary() {
    let settings = settings();
    let world = GameWorld::new(level(), settings.rules()).unwrap();
    let mut live = GameSession::live(world, &settings);
    for _ in 0..600 {
        let input = autoplay_input(live.world());
        live.step(input).unwrap();
    }
    let summary = live.summary();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("starfall-replay.json");
    live.write_record(&path).unwrap();

    let replay_world = GameWorld::new(level(), settings.rules()).unwrap();
    let mut replay = GameSession::from_file(replay_world, &path, &settings).unwrap();
    for _ in 0..600 {
        if replay.step(starfall::world::GameInput::default()).unwrap() == StepResult::Exhausted {
            break;
        }
    }
    assert_eq!(replay.summary(), summary);
    assert!(replay.world().score > 0);
}

#[test]
fn recorder_replay_reuses_quasar_sim_and_reports_divergence() {
    let settings = settings();
    let world = GameWorld::new(level(), settings.rules()).unwrap();
    let mut recorder = Recorder::new(world.clone(), settings.simulation_config());
    for _ in 0..240 {
        let input = autoplay_input(recorder.simulation());
        recorder.tick(input).unwrap();
    }
    let expected = recorder.current_hash();
    let mut replay = Replay::new(world, recorder.journal().clone()).unwrap();
    let summary = replay.run_to_end().unwrap();
    assert_eq!(summary.last_hash, expected);
    assert_eq!(summary.ticks, 240);
}

#[test]
fn content_loads_level_texture_and_generated_sounds() {
    let settings = settings();
    let diagnostics = Diagnostics::disabled();
    let content = starfall::load_content_blocking(&settings, &diagnostics).unwrap();
    assert_eq!(content.level.crystals.len(), 5);
    assert_eq!(content.level.drones.len(), 3);
    assert_eq!((content.texture.width, content.texture.height), (8, 8));
    assert_eq!(content.texture.rgba.len(), 8 * 8 * 4);
    assert!(content.sounds.win.samples().len() > 1_000);
    for event in [
        starfall::world::GameEvent::Crystal,
        starfall::world::GameEvent::Shot,
        starfall::world::GameEvent::DroneDestroyed,
        starfall::world::GameEvent::Damage,
        starfall::world::GameEvent::PortalReady,
        starfall::world::GameEvent::Won,
        starfall::world::GameEvent::Lost,
    ] {
        assert!(content.sounds.for_event(event).is_some(), "{event:?}");
    }
}

#[test]
fn game_cli_accepts_both_record_and_replay_but_session_configuration_rejects_them() {
    let cli = GameCli::parse(&[
        "--seed".into(),
        "7".into(),
        "--ticks".into(),
        "12".into(),
        "--autoplay".into(),
        "--record".into(),
        "a.json".into(),
    ])
    .unwrap();
    assert_eq!(cli.seed, Some(7));
    assert_eq!(cli.ticks, 12);
    assert!(cli.autoplay);
    assert!(cli.record.is_some());
    assert!(GameCli::parse(&["--ticks".into(), "0".into()]).is_err());
    assert!(GameCli::parse(&["--unknown".into()]).is_err());
    let mut common = quasar_config::CommonArgs::default();
    common.overrides.push(("game.seed".into(), "9".into()));
    common.headless = Some(true);
    let cli = GameCli {
        replay: Some(PathBuf::from("a.json")),
        record: Some(PathBuf::from("b.json")),
        ..GameCli::default()
    };
    assert!(GameSettings::load(&common, &cli).is_err());
}

#[test]
fn configured_controls_drive_named_actions() {
    let settings = settings();
    let mut input = InputState::default();
    input
        .set_bindings(settings.controls.bindings().unwrap())
        .unwrap();
    input.key(Key::KeyW, true);
    input.key(Key::ArrowLeft, true);
    input.key(Key::Space, true);
    input.key(Key::KeyR, true);
    let game_input = settings.controls.input(&input);
    assert_eq!(game_input.forward, 1.0);
    assert!(game_input.shoot);
    assert!(game_input.restart);
    input.key(Key::Space, false);
    assert!(!settings.controls.input(&input).shoot);
    input.key(Key::Escape, true);
    assert!(settings.controls.quit_requested(&input));
}

#[test]
fn mouse_motion_and_left_click_drive_aim_and_shoot() {
    let settings = settings();
    let mut input = InputState::default();
    input
        .set_bindings(settings.controls.bindings().unwrap())
        .unwrap();
    input.relative_motion = (120.0, 0.0);
    input.mouse_button(MouseButton::Left, true);
    let game_input = settings.input(&input);
    assert!((game_input.look + 120.0 * settings.mouse_sensitivity).abs() < 1e-6);
    assert!(game_input.shoot);
}

#[test]
fn strafe_right_is_screen_right() {
    let level = Level {
        duration: 90.0,
        crystals: vec![[0.0, 0.9, -10.0]],
        drones: vec![],
        portal: [0.0, 0.5, 8.0],
    };
    let mut world = GameWorld::new(level, GameRules::default()).unwrap();
    assert!((world.yaw - std::f32::consts::PI).abs() < 1e-6);
    let start = world.player;
    world.update(GameInput {
        strafe: 1.0,
        ..GameInput::default()
    });
    assert!(world.player.x > start.x);
    let right = world.player.x;
    world.update(GameInput {
        strafe: -1.0,
        ..GameInput::default()
    });
    assert!(world.player.x < right);
}

#[test]
fn projectile_orientation_follows_player_facing() {
    for yaw in [0.0, 1.1, std::f32::consts::PI, -2.2] {
        let level = Level {
            duration: 90.0,
            crystals: vec![[0.0, 0.9, -10.0]],
            drones: vec![],
            portal: [0.0, 0.5, 8.0],
        };
        let mut world = GameWorld::new(level, GameRules::default()).unwrap();
        world.yaw = yaw;
        world.update(GameInput {
            shoot: true,
            ..GameInput::default()
        });
        let shot = world.shots.first().expect("shot spawned");
        let difference = (shot.yaw() - yaw).rem_euclid(std::f32::consts::TAU);
        let difference = difference.min(std::f32::consts::TAU - difference);
        assert!(
            difference < 1e-5,
            "shot yaw {} != player yaw {yaw}",
            shot.yaw()
        );
    }
}

#[test]
fn mouse_look_changes_aim_direction() {
    let level = Level {
        duration: 90.0,
        crystals: vec![[0.0, 0.9, -10.0]],
        drones: vec![],
        portal: [0.0, 0.5, 8.0],
    };
    let mut world = GameWorld::new(level, GameRules::default()).unwrap();
    let before = world.forward();
    world.update(GameInput {
        look: 0.5,
        ..GameInput::default()
    });
    assert!((world.forward().x - before.x).abs() > 1e-3);
    assert!((world.forward().z - before.z).abs() > 1e-3);
}

#[test]
fn missing_assets_fail_cleanly() {
    let directory = tempfile::tempdir().unwrap();
    let error = starfall::content::load_content(
        directory.path().to_path_buf(),
        "missing-level.json".into(),
        44_100,
        Diagnostics::disabled(),
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("level"));
}

#[test]
fn projectile_collision_destroys_a_stationary_drone() {
    let level = Level {
        duration: 90.0,
        crystals: vec![[0.0, 0.9, -8.0]],
        drones: vec![DroneSpec {
            radius: 2.0,
            height: 0.9,
            speed: 0.0,
            phase: -std::f32::consts::FRAC_PI_2,
        }],
        portal: [0.0, 0.5, 8.0],
    };
    let mut world = GameWorld::new(level, GameRules::default()).unwrap();
    world.player = Vec3::new(0.0, 0.0, 0.0);
    world.yaw = std::f32::consts::PI;
    world.update(GameInput {
        shoot: true,
        ..GameInput::default()
    });
    assert_eq!(world.drones.len(), 1);
    for _ in 0..10 {
        world.update(GameInput::default());
        if !world.drones[0].active {
            break;
        }
    }
    assert!(!world.drones[0].active);
}

#[test]
fn restart_resets_objective_state() {
    let mut world = GameWorld::new(level(), GameRules::default()).unwrap();
    world.score = 3;
    world.health = 1;
    world.time_left = 2.0;
    world.won = true;
    world.update(GameInput {
        restart: true,
        ..GameInput::default()
    });
    assert_eq!(world.score, 0);
    assert_eq!(world.health, 3);
    assert_eq!(world.crystals_remaining(), 5);
    assert!(!world.won && !world.lost);
    assert_eq!(world.events, vec![starfall::world::GameEvent::Restart]);
}

#[test]
fn mouse_aim_is_preserved_by_replay() {
    let settings = settings();
    let world = GameWorld::new(level(), settings.rules()).unwrap();
    let mut recorder = Recorder::new(world.clone(), settings.simulation_config());
    for index in 0..120 {
        recorder
            .tick(GameInput {
                forward: 1.0,
                look: if index % 7 == 0 { 0.03 } else { 0.0 },
                shoot: index % 11 == 0,
                ..GameInput::default()
            })
            .unwrap();
    }
    let expected = recorder.current_hash();
    let mut replay = Replay::new(world, recorder.journal().clone()).unwrap();
    assert_eq!(replay.run_to_end().unwrap().last_hash, expected);
}

#[test]
fn timeout_sets_loss_and_emits_the_loss_event() {
    let mut world = GameWorld::new(level(), GameRules::default()).unwrap();
    world.time_left = 0.001;
    world.update(GameInput::default());
    assert!(world.lost);
    assert!(world.events.contains(&starfall::world::GameEvent::Lost));
    assert!(world.finished());
}

#[test]
fn game_error_is_a_regular_error() {
    let error = GameError::invalid("test");
    assert_eq!(error.to_string(), "test");
}
