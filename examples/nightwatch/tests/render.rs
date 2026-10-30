#![cfg(feature = "graphics")]
use nightwatch::{
    render::Graphics,
    world::{Input, Weapon, World},
};
use quasar_frame::Gpu;
use quasar_frame::OffscreenTarget;
#[test]
#[ignore = "requires Vulkan"]
fn world_weapon_flash_hud_resize_and_release_compose() {
    let gal = Gpu::headless().unwrap();
    let mut graphics = Graphics::new(&gal).unwrap();
    let target = OffscreenTarget::new(&gal, 640, 360).unwrap();
    let mut w = World::new(42);
    let normal = target
        .render(&gal, |gal, t| graphics.commands(gal, t, &w, false))
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
        .render(&gal, |gal, t| graphics.commands(gal, t, &w, false))
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
        .render(&gal, |gal, t| graphics.commands(gal, t, &w, false))
        .unwrap();
    assert!(
        world_difference(&plain.rgba, &no_shadows.rgba) > 100,
        "normal maps must affect world pixels"
    );
    graphics.set_effects(true, true);
    let restored = target
        .render(&gal, |gal, t| graphics.commands(gal, t, &w, false))
        .unwrap();
    assert_eq!(
        normal.rgba, restored.rgba,
        "runtime toggles must be reversible"
    );
    graphics.resize_shadow_maps(&gal, 256).unwrap();
    let low_resolution = target
        .render(&gal, |gal, t| graphics.commands(gal, t, &w, false))
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
        .render(&gal, |gal, t| graphics.commands(gal, t, &w, false))
        .unwrap();
    assert_ne!(normal.rgba, flash.rgba);
    // Lamp/flashlight effects must alter world pixels, not just the HUD.
    w.muzzle_ticks = 0;
    w.flashlight = false;
    let dark = target
        .render(&gal, |gal, t| graphics.commands(gal, t, &w, false))
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
        .render(&gal, |gal, t| graphics.commands(gal, t, &w, false))
        .unwrap();
    assert_ne!(dark.rgba, rifle.rgba);
    graphics.release_targets(&gal).unwrap();
    target.release(&gal).unwrap();
    let target = OffscreenTarget::new(&gal, 320, 240).unwrap();
    let resized = target
        .render(&gal, |gal, t| graphics.commands(gal, t, &w, true))
        .unwrap();
    assert_eq!(resized.rgba.len(), 320 * 240 * 4);
    graphics.release(&gal).unwrap();
    target.release(&gal).unwrap();
    drop(graphics);
    assert_no_owned_images(&gal);
}

#[test]
#[ignore = "requires GPU"]
fn optimized_frames_match_scalar_reference_with_independent_shadow_culling() {
    use nightwatch::render::GraphicsOptions;
    use quasar_render_3d::SubmissionConfig;
    let gal = Gpu::headless().unwrap();
    let mut reference = Graphics::with_submission(
        &gal,
        GraphicsOptions {
            shadow_resolution: 128,
            ..GraphicsOptions::default()
        },
        SubmissionConfig::default(),
    )
    .unwrap();
    let mut batched = Graphics::with_submission(
        &gal,
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
        &gal,
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
    let target = OffscreenTarget::new(&gal, 640, 360).unwrap();
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
            .render(&gal, |gal, t| reference.commands(gal, t, &w, false))
            .unwrap();
        let b = target
            .render(&gal, |gal, t| batched.commands(gal, t, &w, false))
            .unwrap();
        let c = target
            .render(&gal, |gal, t| culled.commands(gal, t, &w, false))
            .unwrap();
        assert_eq!(a.rgba, b.rgba, "batching parity frame {frame}");
        assert_eq!(a.rgba, c.rgba, "camera/light culling parity frame {frame}");
        assert_eq!(hash, w.state_hash());
        assert!(batched.stats().draws < reference.stats().draws / 4);
        assert!(culled.work_stats().culled > 0);
    }
    reference.release(&gal).unwrap();
    batched.release(&gal).unwrap();
    culled.release(&gal).unwrap();
    target.release(&gal).unwrap();
    drop(reference);
    drop(batched);
    drop(culled);
    assert_no_owned_images(&gal);
}

#[test]
#[ignore = "requires GPU"]
fn authored_asset_reload_is_transactional_and_keeps_the_game_world_unchanged() {
    use nightwatch::render::GraphicsOptions;
    use quasar_render_3d::SubmissionConfig;
    fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let destination = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &destination);
            } else {
                std::fs::copy(entry.path(), destination).unwrap();
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&Graphics::default_asset_root(), dir.path());
    let gal = Gpu::headless().unwrap();
    let mut graphics = Graphics::with_asset_root(
        &gal,
        GraphicsOptions {
            shadow_resolution: 128,
            ..GraphicsOptions::default()
        },
        SubmissionConfig {
            instancing: true,
            frustum_culling: true,
            ..SubmissionConfig::default()
        },
        dir.path(),
    )
    .unwrap();
    let target = OffscreenTarget::new(&gal, 640, 360).unwrap();
    let world = World::new(42);
    let hash = world.state_hash();
    let before = target
        .render(&gal, |gal, t| graphics.commands(gal, t, &world, false))
        .unwrap();
    let pending = graphics
        .commands(&gal, target.target(), &world, false)
        .unwrap();
    // Native command buffers retain the previous GPU generation during reload.
    graphics.reload_assets(&gal).unwrap();
    gal.queue.submit(pending);
    gal.wait().unwrap();
    assert_eq!(
        quasar_frame::read_rgba(&gal, &target.texture).unwrap(),
        before.rgba
    );
    graphics.reload_assets(&gal).unwrap();
    let same = target
        .render(&gal, |gal, t| graphics.commands(gal, t, &world, false))
        .unwrap();
    assert_eq!(before.rgba, same.rgba);
    let model_path = dir.path().join("survival-kit/workbench.glb");
    let original = std::fs::read(&model_path).unwrap();
    std::fs::write(&model_path, b"not glTF").unwrap();
    assert!(graphics.reload_assets(&gal).is_err());
    let fallback = target
        .render(&gal, |gal, t| graphics.commands(gal, t, &world, false))
        .unwrap();
    assert_eq!(
        before.rgba, fallback.rgba,
        "failed reload keeps old GPU generations"
    );
    std::fs::write(&model_path, original).unwrap();
    let palette = dir.path().join("blaster-kit/Textures/colormap.png");
    quasar_render_2d::write_png(&palette, 1, 1, &[0, 255, 0, 255]).unwrap();
    graphics.reload_assets(&gal).unwrap();
    let changed = target
        .render(&gal, |gal, t| graphics.commands(gal, t, &world, false))
        .unwrap();
    assert_ne!(before.rgba, changed.rgba);
    assert_eq!(hash, world.state_hash());
    graphics.release(&gal).unwrap();
    target.release(&gal).unwrap();
    drop(graphics);
    assert_no_owned_images(&gal);
}

fn assert_no_owned_images(gpu: &Gpu) {
    gpu.wait().unwrap();
    let report = gpu.instance.generate_report().expect("native wgpu report");
    assert_eq!(report.hub.textures.num_kept_from_user, 0, "{report:?}");
    assert_eq!(report.hub.texture_views.num_kept_from_user, 0, "{report:?}");
    assert_eq!(report.hub.buffers.num_kept_from_user, 0, "{report:?}");
}
