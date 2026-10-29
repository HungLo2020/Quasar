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
    assert!(graphics.work_stats().visible > 100);
    assert!(graphics.stats().draws < 100);
    assert!(graphics.work_stats().culled > 0);
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
        graphics.stats().draws < shadow_draws,
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

#[test]
#[ignore = "requires GPU"]
fn optimized_frames_match_scalar_reference_with_independent_shadow_culling() {
    use nightwatch::render::GraphicsOptions;
    use quasar_render_3d::SubmissionConfig;
    let mut gal =
        VulkanicGal::create(BackendChoice::Vulkan, "nightwatch optimization parity").unwrap();
    let mut reference = Graphics::with_submission(
        &mut gal,
        GraphicsOptions {
            shadow_resolution: 128,
            ..GraphicsOptions::default()
        },
        SubmissionConfig::default(),
    )
    .unwrap();
    let mut batched = Graphics::with_submission(
        &mut gal,
        GraphicsOptions {
            shadow_resolution: 128,
            ..GraphicsOptions::default()
        },
        SubmissionConfig {
            instancing: true,
            ..SubmissionConfig::default()
        },
    )
    .unwrap();
    let mut culled = Graphics::with_submission(
        &mut gal,
        GraphicsOptions {
            shadow_resolution: 128,
            ..GraphicsOptions::default()
        },
        SubmissionConfig {
            instancing: true,
            frustum_culling: true,
            ..SubmissionConfig::default()
        },
    )
    .unwrap();
    let mut target = OffscreenTarget::new(&mut gal, 640, 360).unwrap();
    let mut w = World::new(42);
    for frame in 0..6 {
        if frame > 0 {
            for _ in 0..20 {
                w.update(nightwatch::world::autoplay(&w));
            }
        }
        if frame == 3 {
            w.flashlight = false;
        }
        let hash = w.state_hash();
        let a = target
            .render(&mut gal, |gal, t| reference.commands(gal, t, &w, false))
            .unwrap();
        let b = target
            .render(&mut gal, |gal, t| batched.commands(gal, t, &w, false))
            .unwrap();
        let c = target
            .render(&mut gal, |gal, t| culled.commands(gal, t, &w, false))
            .unwrap();
        assert_eq!(a.rgba, b.rgba, "batching parity frame {frame}");
        assert_eq!(a.rgba, c.rgba, "camera/light culling parity frame {frame}");
        assert_eq!(hash, w.state_hash());
        assert!(batched.stats().draws < reference.stats().draws / 4);
        assert!(culled.work_stats().culled > 0);
    }
    reference.release(&mut gal).unwrap();
    batched.release(&mut gal).unwrap();
    culled.release(&mut gal).unwrap();
    target.release(&mut gal).unwrap();
    gal.retire_through(gal.latest_submission_id()).unwrap();
    assert_eq!(
        gal.metrics().resource_creates,
        gal.metrics().resource_destroys
    );
}
