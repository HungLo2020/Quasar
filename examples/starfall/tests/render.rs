use quasar_diagnostics::Diagnostics;
use quasar_frame::OffscreenTarget;
use starfall::render::Graphics;
use starfall::settings::{GameCli, GameSettings};
use starfall::world::{GameRules, GameWorld};
use vulkanic_gal::{BackendChoice, VulkanicGal};

#[test]
#[ignore = "requires a Vulkan-capable graphics device"]
fn world_and_hud_render_to_a_non_uniform_offscreen_frame() {
    let settings = GameSettings::load(
        &quasar_config::CommonArgs::default(),
        &GameCli {
            ticks: 30,
            ..GameCli::default()
        },
    )
    .unwrap();
    let content = starfall::load_content_blocking(&settings, &Diagnostics::disabled()).unwrap();
    let world = GameWorld::new(content.level, GameRules::default()).unwrap();
    let mut gal = VulkanicGal::create(BackendChoice::Vulkan, "Starfall render test").unwrap();
    let mut graphics = Graphics::new(&mut gal, &content.texture).unwrap();
    let mut target = OffscreenTarget::new(&mut gal, 320, 180).unwrap();
    let report = target
        .render(&mut gal, |gal, target| {
            graphics.commands(gal, target, &world)
        })
        .unwrap();
    assert_eq!(report.rgba.len(), 320 * 180 * 4);
    let colors: std::collections::BTreeSet<_> =
        report.rgba.as_chunks::<4>().0.iter().copied().collect();
    assert!(colors.len() > 16, "offscreen frame is unexpectedly uniform");
    assert!(graphics.last_stats().draws > 10);
    target.release(&mut gal).unwrap();
    graphics.release(&mut gal).unwrap();
    gal.retire_through(gal.latest_submission_id()).unwrap();
}

#[test]
#[ignore = "requires a Vulkan-capable graphics device"]
fn offscreen_dimensions_are_validated() {
    assert!(
        quasar_frame::OffscreenTarget::new(
            &mut VulkanicGal::create(BackendChoice::Vulkan, "unused").unwrap(),
            0,
            10
        )
        .is_err()
    );
}
