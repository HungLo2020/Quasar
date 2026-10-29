//! Optional runner adapter. The game, renderer and GAL remain independent owners.
use nightwatch::{
    Result,
    render::Graphics,
    settings::Settings,
    world::{Input, Weapon, World, autoplay},
};
use quasar_app::{
    AppConfig, AppContext, Application, DesktopApplication, Flow, RenderPolicy, TimingConfig,
};
use quasar_core::FixedStepConfig;
use quasar_frame::{CompletionPolicy, FrameSurface};
use quasar_input::{Key, MouseButton};
use quasar_profile::{FrameProfiler, FrameSample};
use quasar_window::{DesktopConfig, WindowConfig, winit::event::WindowEvent};
use std::time::Duration;
use vulkanic_gal::{resources::*, *};

pub fn run(settings: Settings) -> Result<()> {
    let config = AppConfig {
        window: WindowConfig {
            title: "Nightwatch - Quasar".into(),
            application_id: "quasar-nightwatch".into(),
            size: (settings.width, settings.height),
            minimum_size: (640, 360),
            fullscreen: settings.fullscreen,
            resizable: true,
        },
        desktop: DesktopConfig::for_application("quasar-nightwatch"),
        timing: TimingConfig::Fixed(FixedStepConfig::for_rate(60., 8)),
        render: RenderPolicy::EveryFrame,
        max_frame_wait: Duration::from_millis(16),
        frame_interval: None,
    };
    let world = World::new(settings.seed);
    quasar_app::run(
        App {
            settings,
            world,
            paused: false,
            graphics: None,
            surface: None,
            gal: None,
            frames: 0,
            done: false,
            profile: FrameProfiler::new(600),
            audio: None,
        },
        config,
    )
    .map_err(|e| e.to_string().into())
}
struct App {
    settings: Settings,
    world: World,
    paused: bool,
    graphics: Option<Graphics>,
    surface: Option<FrameSurface>,
    gal: Option<VulkanicGal>,
    frames: u64,
    done: bool,
    profile: FrameProfiler,
    audio: Option<crate::sound::Audio>,
}
impl App {
    fn initialize(&mut self, c: &AppContext<'_>) -> Result<()> {
        if self.gal.is_some() {
            return Ok(());
        }
        let window = c.window.ok_or("missing desktop window")?;
        let size = window.native().inner_size();
        set_gpu_timestamps_requested(self.settings.benchmark);
        let mut gal = VulkanicGal::create_windowed_vulkan(
            "Nightwatch",
            window.shared_native(),
            FrameSurfaceDesc {
                label: "nightwatch-surface".into(),
                extent: Extent3d {
                    width: size.width.max(1),
                    height: size.height.max(1),
                    depth: 1,
                },
                color_format: TextureFormat::Bgra8Unorm,
                present_mode: if self.settings.benchmark {
                    PresentMode::AutoNoVsync
                } else {
                    PresentMode::AutoVsync
                },
                max_frames_in_flight: 3,
            },
        )?;
        self.graphics = Some(Graphics::with_options(
            &mut gal,
            nightwatch::render::GraphicsOptions {
                shadows: self.settings.shadows,
                normal_maps: self.settings.normal_maps,
                shadow_resolution: self.settings.shadow_resolution,
            },
        )?);
        self.surface = Some(FrameSurface::new(size.width.max(1), size.height.max(1)));
        self.gal = Some(gal);
        if self.settings.audio {
            self.audio = crate::sound::Audio::open(self.settings.audio_device.as_deref());
        }
        Ok(())
    }
    fn release(&mut self) -> Result<()> {
        self.audio.take();
        if let Some(mut gal) = self.gal.take() {
            if let Some(mut surface) = self.surface.take() {
                surface.finish(&mut gal)?;
                surface.release_targets(&mut gal)?;
            }
            if let Some(mut graphics) = self.graphics.take() {
                graphics.release(&mut gal)?;
            }
            gal.shutdown_frame_surface()?;
            gal.retire_through(gal.latest_submission_id())?;
            let metrics = gal.metrics();
            println!(
                "nightwatch resources created={} destroyed={}",
                metrics.resource_creates, metrics.resource_destroys
            );
            if metrics.resource_creates != metrics.resource_destroys {
                return Err("GPU resource imbalance at teardown".into());
            }
        }
        Ok(())
    }
    fn capture(&self, c: &AppContext<'_>) {
        if let Some(window) = c.window
            && let Err(e) = window.capture_cursor(!self.paused && !self.world.game_over())
        {
            eprintln!("nightwatch cursor: {e}");
        }
    }
}
impl Application for App {
    type Error = nightwatch::GameError;
    fn setup(&mut self, c: &mut AppContext<'_>) -> Result<()> {
        self.initialize(c)
    }
    fn frame_update(&mut self, c: &mut AppContext<'_>) -> Result<Flow> {
        if self.done {
            return Ok(Flow::Exit);
        }
        if let Some(graphics) = &mut self.graphics {
            let (mut shadows, mut normals) = graphics.effects();
            if c.input.pressed(Key::KeyH) {
                shadows = !shadows;
                c.input.consume_pressed(Key::KeyH);
            }
            if c.input.pressed(Key::KeyN) {
                normals = !normals;
                c.input.consume_pressed(Key::KeyN);
            }
            graphics.set_effects(shadows, normals);
            self.settings.shadows = shadows;
            self.settings.normal_maps = normals;
        }
        if c.input.pressed(Key::F11) {
            if let Some(window) = c.window {
                window.toggle_fullscreen();
            }
            c.input.consume_pressed(Key::F11);
        }
        if c.focus_changed == Some(false) && !self.settings.autoplay {
            self.paused = true;
            self.capture(c);
            c.input.consume_all();
            return Ok(Flow::Continue);
        }
        if c.input.pressed(Key::Escape) {
            if self.world.game_over() {
                return Ok(Flow::Exit);
            }
            self.paused = !self.paused;
            self.capture(c);
            c.input.consume_all();
            return Ok(Flow::Continue);
        }
        if self.world.game_over() && c.input.pressed(Key::Enter) {
            self.world = World::new(self.settings.seed);
            self.paused = false;
            self.capture(c);
            c.input.consume_all();
            return Ok(Flow::Continue);
        }
        if self.paused {
            if c.input.pressed(Key::KeyQ) {
                return Ok(Flow::Exit);
            }
            if c.input.mouse_pressed(MouseButton::Left) {
                self.paused = false;
                self.capture(c);
            }
            c.input.consume_all();
            return Ok(Flow::Continue);
        }
        // A benchmark renders the same scripted tick sequence on every machine,
        // independently of host frame rate. Interactive play keeps fixed ticks.
        if self.settings.benchmark && self.settings.autoplay {
            let input = autoplay(&self.world);
            self.world.update(input);
        }
        if !self.world.game_over() && !self.settings.autoplay {
            let (x, y) = c.input.relative_motion();
            self.world.look([x as f32, y as f32]);
            c.input.consume_relative_motion();
        }
        Ok(Flow::Continue)
    }
    fn tick(&mut self, c: &mut AppContext<'_>) -> Result<Flow> {
        if self.settings.benchmark && self.settings.autoplay {
            return Ok(Flow::Continue);
        }
        if self.paused {
            return Ok(Flow::Deferred);
        }
        let input = if self.settings.autoplay {
            autoplay(&self.world)
        } else {
            Input {
                forward: f32::from(c.input.held(Key::KeyW)) - f32::from(c.input.held(Key::KeyS)),
                strafe: f32::from(c.input.held(Key::KeyD)) - f32::from(c.input.held(Key::KeyA)),
                fire: c.input.mouse_held(MouseButton::Left),
                reload: c.input.pressed(Key::KeyR),
                jump: c.input.held(Key::Space),
                sprint: c.input.held(Key::ShiftLeft),
                flashlight: c.input.pressed(Key::KeyF),
                weapon: if c.input.pressed(Key::Digit1) {
                    Some(Weapon::Pistol)
                } else if c.input.pressed(Key::Digit2) {
                    Some(Weapon::Rifle)
                } else {
                    None
                },
                ..Input::default()
            }
        };
        let alive = !self.world.game_over();
        let shots = self.world.shots_fired;
        let health = self.world.health;
        let reloading = self.world.reload_ticks;
        self.world.update(input);
        if let Some(audio) = &self.audio {
            if self.world.shots_fired > shots {
                audio.fire(self.world.weapon);
            }
            if self.world.health < health {
                audio.hurt();
            }
            if self.world.reload_ticks > reloading {
                audio.reload();
            }
        }
        if alive && self.world.game_over() {
            self.capture(c);
        }
        Ok(Flow::Continue)
    }
    fn render(&mut self, _: &mut AppContext<'_>) -> Result<Flow> {
        if self.done {
            return Ok(Flow::Continue);
        }
        if let (Some(gal), Some(surface), Some(graphics)) =
            (&mut self.gal, &mut self.surface, &mut self.graphics)
            && let Some(report) = surface.render(gal, CompletionPolicy::Poll, |gal, target| {
                graphics.commands(gal, target, &self.world, self.paused)
            })?
        {
            self.frames += 1;
            if self.settings.benchmark && self.frames > 8 {
                let mut sample = FrameSample::from_frame_report(self.frames, &report);
                sample.with_renderer_stats(graphics.stats());
                self.profile.record(sample);
            }
            if self
                .settings
                .frames
                .is_some_and(|limit| self.frames >= limit)
            {
                self.done = true;
            }
        }
        Ok(Flow::Continue)
    }
    fn suspended(&mut self, _: &mut AppContext<'_>) -> Result<()> {
        self.paused = true;
        self.release()
    }
    fn shutdown(&mut self, _: &mut AppContext<'_>) -> Result<()> {
        if let Some(path) = &self.settings.benchmark_file {
            self.profile.write_json(path)?;
        }
        self.release()?;
        println!(
            "nightwatch frames={} ticks={} shots={} kills={} wave={} weapon={:?} hash={:016x}",
            self.frames,
            self.world.tick,
            self.world.shots_fired,
            self.world.kills,
            self.world.wave,
            self.world.weapon,
            self.world.state_hash()
        );
        if let Some(path) = &self.settings.screenshot {
            nightwatch::render::screenshot_with_options(
                &self.world,
                self.settings.width,
                self.settings.height,
                path,
                nightwatch::render::GraphicsOptions {
                    shadows: self.settings.shadows,
                    normal_maps: self.settings.normal_maps,
                    shadow_resolution: self.settings.shadow_resolution,
                },
            )?;
        }
        Ok(())
    }
}
impl DesktopApplication for App {
    fn resumed(&mut self, c: &mut AppContext<'_>) -> Result<()> {
        self.initialize(c)
    }
    fn configure_window(&mut self, window: &mut quasar_window::GameWindow) -> Result<()> {
        if let Err(e) = window.capture_cursor(!self.paused) {
            eprintln!("nightwatch cursor: {e}");
        }
        Ok(())
    }
    fn window_event(&mut self, c: &mut AppContext<'_>, event: &WindowEvent) -> Result<()> {
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
        ) && let (Some(gal), Some(surface), Some(graphics)) =
            (&mut self.gal, &mut self.surface, &mut self.graphics)
        {
            surface.finish(gal)?;
            graphics.release_targets(gal)?;
            if let Some(window) = c.window {
                let size = window.native().inner_size();
                surface.resize(size.width, size.height);
            }
        }
        Ok(())
    }
}
