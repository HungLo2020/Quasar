use super::*;
fn world() -> World {
    World::load(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/island.tmj")).unwrap()
}
#[test]
fn autoplay_completes_quest_and_replays_identically() {
    let world = world();
    let mut state = world.initial();
    let mut autoplay = Autoplay::default();
    let mut inputs = Vec::new();
    for _ in 0..6000 {
        let input = autoplay.input(&world, &state).unwrap();
        world.tick(&mut state, input).unwrap();
        inputs.push(input);
    }
    assert_eq!(state.phase, Phase::Won);
    assert_eq!(state.collected.len(), 3);
    let replay = Replay {
        version: 1,
        map_hash: world.hash,
        inputs,
    };
    replay.validate(&world).unwrap();
    let mut played = world.initial();
    for input in replay.inputs {
        world.tick(&mut played, input).unwrap();
    }
    assert_eq!(state, played);
    assert!(world.camera(&state).x > 1000.);
}
#[test]
fn pause_collision_and_restart_preserve_game_rules() {
    let world = world();
    let mut state = world.initial();
    world
        .tick(
            &mut state,
            Input {
                pause: true,
                ..Default::default()
            },
        )
        .unwrap();
    let paused = state.clone();
    world
        .tick(
            &mut state,
            Input {
                x: 1,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(state, paused);
    world
        .tick(
            &mut state,
            Input {
                restart: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(state, world.initial());
    state.position = [3 * 4096 + 5 * 256, 16 * 4096 + 2048];
    let old = state.position;
    world
        .tick(
            &mut state,
            Input {
                x: -1,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(state.position, old);
}
#[test]
fn map_scene_culls_and_camera_is_restored() {
    let world = world();
    let art = Art::load(&world).unwrap();
    let state = world.initial();
    let (canvas, stats) = scene(&world, &state, &art, Filter::Nearest).unwrap();
    assert!(stats.visited_cells < 3000);
    assert_eq!(world.runtime.stored_cells(), 49152);
    assert!(stats.emitted_tiles > 500);
    assert_eq!(canvas.camera(), Camera2D::top_left(WIDTH, HEIGHT));
    canvas.validate(&art.textures).unwrap();
}
#[cfg(feature = "desktop")]
#[test]
#[ignore = "requires Vulkan"]
fn gpu_game_pixels_match_headless_scene() {
    let world = world();
    let art = Art::load(&world).unwrap();
    let mut state = world.initial();
    let mut auto = Autoplay::default();
    let mut renderer = quasar_render_2d::Renderer2D::headless().unwrap();
    for count in [0, 300, 700] {
        for _ in 0..count {
            let input = auto.input(&world, &state).unwrap();
            world.tick(&mut state, input).unwrap();
        }
        let (canvas, _) = scene(&world, &state, &art, Filter::Nearest).unwrap();
        let expected = quasar_render_2d::reference::rasterize(&canvas, &art.textures).unwrap();
        let actual = renderer.render_to_image(&canvas, &art.textures).unwrap();
        assert!(
            actual
                .iter()
                .zip(&expected)
                .all(|(a, b)| a.abs_diff(*b) <= 1)
        );
        assert!(renderer.stats().batches < 12);
    }
}

#[test]
fn controls_are_neutral_until_named_actions_change() {
    let controls = controls::Controls::default();
    let mut input = quasar_input::InputState::default();
    input.set_bindings(controls.bindings.clone()).unwrap();
    assert_eq!(controls.input(&input), Input::default());
    input.key(quasar_input::Key::KeyA, true);
    assert_eq!(controls.input(&input).x, -1);
    input.release_all();
    assert_eq!(controls.input(&input), Input::default());
}
