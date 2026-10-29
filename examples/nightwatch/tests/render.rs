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
    let shadow_draws = graphics.stats().draws;
    let world_difference = |a: &[u8], b: &[u8]| {
        (100..290)
            .flat_map(|y| (40..600).map(move |x| (y * 640 + x) * 4))
            .filter(|i| a[*i..*i + 3] != b[*i..*i + 3])
            .count()
    };
    graphics.set_effects(false, true);
    let no_shadows = target
        .render(&mut gal, |gal, t| graphics.commands(gal, t, &w, false))
        .unwrap();
    assert!(
        world_difference(&normal.rgba, &no_shadows.rgba) > 100,
        "directional/flashlight shadows must affect world pixels"
    );
    assert!(
        graphics.stats().draws < shadow_draws / 2,
        "disabled shadows omit caster passes"
    );
    graphics.set_effects(false, false);
    let plain = target
        .render(&mut gal, |gal, t| graphics.commands(gal, t, &w, false))
        .unwrap();
    assert!(
        world_difference(&plain.rgba, &no_shadows.rgba) > 100,
        "normal maps must affect world pixels"
    );
    graphics.set_effects(true, true);
    let restored = target
        .render(&mut gal, |gal, t| graphics.commands(gal, t, &w, false))
        .unwrap();
    assert_eq!(
        normal.rgba, restored.rgba,
        "runtime toggles must be reversible"
    );
    graphics.resize_shadow_maps(&mut gal, 256).unwrap();
    let low_resolution = target
        .render(&mut gal, |gal, t| graphics.commands(gal, t, &w, false))
        .unwrap();
    assert_ne!(
        normal.rgba, low_resolution.rgba,
        "shadow resolution must change filtering"
    );
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
