use falling_sand::render::Graphics;
use falling_sand::sand::{
    Cell, GRID_HEIGHT, GRID_WIDTH, SandInput, SandWorld, Tool, autoplay_input,
};
use quasar_frame::Gpu;
use quasar_frame::OffscreenTarget;

fn run_script(seed: u64, ticks: u64) -> SandWorld {
    let mut world = SandWorld::new(seed);
    for _ in 0..ticks {
        world.update(autoplay_input(&world));
    }
    world
}

#[test]
fn scripted_runs_are_reproducible() {
    let first = run_script(0x1234_5678, 240);
    let second = run_script(0x1234_5678, 240);
    assert_eq!(first.state_hash(), second.state_hash());
    assert_eq!(first.stats().sand, second.stats().sand);
    assert_eq!(first.tick(), 240);
}

#[test]
fn sand_falls_and_accumulates_on_the_ground() {
    let mut world = SandWorld::new(7);
    world.clear_sand();
    world.paint_sand([120.0, 10.0], 2.0);
    assert!(world.stats().sand > 0);
    for _ in 0..120 {
        world.update(SandInput::default());
    }
    let count = (0..GRID_HEIGHT)
        .flat_map(|y| (0..GRID_WIDTH).map(move |x| (x, y)))
        .filter(|(x, y)| world.cell(*x, *y) == Some(Cell::Sand))
        .count();
    assert_eq!(count, world.stats().sand);
    assert!(world.stats().sand >= 8);
    let lowest = (0..GRID_HEIGHT)
        .rev()
        .find(|y| (0..GRID_WIDTH).any(|x| world.cell(x, *y) == Some(Cell::Sand)))
        .unwrap();
    assert!(lowest > 100, "sand did not fall: lowest row {lowest}");
}

#[test]
fn a_rock_blocks_sand() {
    let mut world = SandWorld::new(11);
    world.clear_sand();
    let rock = world
        .place_rock(quasar_spatial::Aabb::new([100.0, 60.0], [140.0, 65.0]).unwrap())
        .unwrap();
    for x in 108..=132 {
        for y in 20..=30 {
            world.paint_sand([x as f32, y as f32], 0.6);
        }
    }
    for _ in 0..180 {
        world.update(SandInput::default());
    }
    for y in 60..65 {
        for x in 100..140 {
            assert!(world.is_solid(x, y));
            assert_ne!(world.cell(x, y), Some(Cell::Sand));
        }
    }
    assert!(world.remove_obstacle(rock));
}

#[test]
fn sand_and_rock_brushes_and_erase_are_interactive() {
    let mut world = SandWorld::new(13);
    world.clear_sand();
    world.paint_sand([80.0, 40.0], 5.0);
    assert!(world.stats().sand > 50);
    world.paint_erase([80.0, 40.0], 7.0);
    assert_eq!(world.stats().sand, 0);

    let rock = world.place_rock_at([80.0, 50.0]).unwrap();
    assert!(world.is_solid(80, 50));
    assert!(world.place_rock_at([81.0, 50.0]).is_none());
    world.erase([80.0, 50.0], 8.0);
    assert!(!world.is_solid(80, 50));
    assert!(world.obstacles().all(|(id, _)| *id != rock));
}

#[test]
fn tools_place_sand_and_erase_through_the_input_path() {
    let mut world = SandWorld::new(17);
    world.clear_sand();
    let mut input = SandInput {
        pointer: [60.0, 20.0],
        pointer_valid: true,
        primary_held: true,
        primary_pressed: true,
        tool: Tool::Sand,
        ..SandInput::default()
    };
    world.update(input);
    let after_sand = world.stats().sand;
    assert!(after_sand > 0);

    input.tool = Tool::Erase;
    input.primary_pressed = true;
    world.update(input);
    assert!(world.stats().sand < after_sand);

    input.tool = Tool::Rock;
    input.pointer = [120.0, 40.0];
    input.primary_held = false;
    input.primary_pressed = true;
    world.update(input);
    assert!(world.is_solid(120, 40));
}

#[test]
fn the_rapier_ball_bounces_and_stays_above_the_ground() {
    let mut world = SandWorld::new(19);
    for _ in 0..300 {
        world.update(SandInput::default());
    }
    let ball = world.ball_position();
    assert!(ball[0].is_finite() && ball[1].is_finite());
    assert!(ball[1] > 125.0, "ball escaped below the world: {ball:?}");
    assert!(ball[1] < 138.0);
    assert!(world.stats().physics_steps >= 300);
}

#[test]
fn state_hash_reacts_to_input_and_reset_is_deterministic() {
    let mut idle = SandWorld::new(23);
    let mut active = SandWorld::new(23);
    for _ in 0..60 {
        idle.update(SandInput::default());
    }
    for _ in 0..60 {
        active.update(autoplay_input(&active));
    }
    assert_ne!(idle.state_hash(), active.state_hash());
    let before = idle.state_hash();
    idle.update(SandInput {
        reset: true,
        ..SandInput::default()
    });
    assert_eq!(idle.tick(), 0);
    assert_ne!(idle.state_hash(), before);
    let fresh = SandWorld::new(23);
    assert_eq!(idle.state_hash(), fresh.state_hash());
}

#[test]
#[ignore = "requires a Vulkan-capable graphics device"]
fn offscreen_frame_is_non_uniform_and_resources_balance() {
    let gal = Gpu::headless().unwrap();
    let mut graphics = Graphics::new(&gal).unwrap();
    let target = OffscreenTarget::new(&gal, 480, 270).unwrap();
    let world = run_script(31, 40);
    let mut menu = falling_sand::menu::Menu::new(falling_sand::settings::UiOptions::default());
    menu.open_main();
    menu.update(
        &quasar_ui::UiInput::default(),
        quasar_ui::Rect::new(0.0, 0.0, 480.0, 270.0),
        1.0,
        1.0 / 60.0,
    )
    .unwrap();
    let report = target
        .render(&gal, |gal, target| {
            graphics.commands(
                gal,
                target,
                &world,
                Some([60.0, 30.0]),
                Tool::Sand,
                menu.ui(),
            )
        })
        .unwrap();
    assert_eq!(report.rgba.len(), 480 * 270 * 4);
    let colors: std::collections::BTreeSet<_> =
        report.rgba.as_chunks::<4>().0.iter().copied().collect();
    assert!(colors.len() > 4);
    target.release(&gal).unwrap();
    graphics.release(&gal).unwrap();

    drop(graphics);
    gal.wait().unwrap();
    let h = gal.instance.generate_report().unwrap().hub;
    assert_eq!(
        h.buffers.num_kept_from_user
            + h.textures.num_kept_from_user
            + h.texture_views.num_kept_from_user,
        0
    );
}
