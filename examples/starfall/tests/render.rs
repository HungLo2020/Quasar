use quasar_diagnostics::Diagnostics;
use quasar_frame::Gpu;
use quasar_frame::OffscreenTarget;
use starfall::render::Graphics;
use starfall::settings::{GameCli, GameSettings};
use starfall::world::{GameRules, GameWorld};

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
    let gal = Gpu::headless().unwrap();
    let mut graphics = Graphics::new(&gal, &content.texture).unwrap();
    let target = OffscreenTarget::new(&gal, 320, 180).unwrap();
    let report = target
        .render(&gal, |gal, target| {
            graphics.commands(gal, target, &world, None)
        })
        .unwrap();
    assert_eq!(report.rgba.len(), 320 * 180 * 4);
    let colors: std::collections::BTreeSet<_> =
        report.rgba.as_chunks::<4>().0.iter().copied().collect();
    assert!(colors.len() > 16, "offscreen frame is unexpectedly uniform");
    assert!(graphics.last_stats().draws > 10);
    target.release(&gal).unwrap();
    graphics.release(&gal).unwrap();
}

#[test]
#[ignore = "requires a Vulkan-capable graphics device"]
fn offscreen_dimensions_are_validated() {
    assert!(quasar_frame::OffscreenTarget::new(&Gpu::headless().unwrap(), 0, 10).is_err());
}
