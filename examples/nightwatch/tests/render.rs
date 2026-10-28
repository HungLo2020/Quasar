#![cfg(feature = "graphics")]
use nightwatch::{
    render::Graphics,
    world::{Input, Weapon, World},
};
use quasar_frame::OffscreenTarget;
use vulkanic_gal::{BackendChoice, VulkanicGal};
#[test]
#[ignore = "requires Vulkan"]
fn world_weapon_flash_hud_resize_and_release_compose() {
    let mut gal = VulkanicGal::create(BackendChoice::Vulkan, "nightwatch test").unwrap();
    let mut graphics = Graphics::new(&mut gal).unwrap();
    let mut target = OffscreenTarget::new(&mut gal, 640, 360).unwrap();
    let mut w = World::new(42);
    let normal = target
        .render(&mut gal, |gal, t| graphics.commands(gal, t, &w, false))
        .unwrap();
    assert!(graphics.stats().draws > 100);
    let unique: std::collections::BTreeSet<_> =
        normal.rgba.as_chunks::<4>().0.iter().copied().collect();
    assert!(unique.len() > 100);
    w.update(Input {
        fire: true,
        ..Input::default()
    });
    let flash = target
        .render(&mut gal, |gal, t| graphics.commands(gal, t, &w, false))
        .unwrap();
    assert_ne!(normal.rgba, flash.rgba);
    // Lamp/flashlight effects must alter world pixels, not just the HUD.
    w.muzzle_ticks = 0;
    w.flashlight = false;
    let dark = target
        .render(&mut gal, |gal, t| graphics.commands(gal, t, &w, false))
        .unwrap();
    let sample = |rgba: &[u8]| {
        rgba[(180 * 640 + 320) * 4..(300 * 640 + 320) * 4]
            .iter()
            .map(|v| u64::from(*v))
            .sum::<u64>()
    };
    assert_ne!(sample(&normal.rgba), sample(&dark.rgba));
    w.weapon = Weapon::Rifle;
    let rifle = target
        .render(&mut gal, |gal, t| graphics.commands(gal, t, &w, false))
        .unwrap();
    assert_ne!(dark.rgba, rifle.rgba);
    graphics.release_targets(&mut gal).unwrap();
    target.release(&mut gal).unwrap();
    let mut target = OffscreenTarget::new(&mut gal, 320, 240).unwrap();
    let resized = target
        .render(&mut gal, |gal, t| graphics.commands(gal, t, &w, true))
        .unwrap();
    assert_eq!(resized.rgba.len(), 320 * 240 * 4);
    graphics.release(&mut gal).unwrap();
    target.release(&mut gal).unwrap();
    gal.retire_through(gal.latest_submission_id()).unwrap();
    assert_eq!(
        gal.metrics().resource_creates,
        gal.metrics().resource_destroys
    );
}
