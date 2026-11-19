//! The windowed game: fixed 60 Hz movement, mouse look and rendering.
use graveyard::render::Graphics;
use graveyard::world::{Input, TICK_RATE, World};
use graveyard::{GraveError, Result};
use quasar_app::{
    AppConfig, AppContext, Application, DesktopApplication, Flow, RenderPolicy, TimingConfig,
    run as run_app,
};
use quasar_core::FixedStepConfig;
use quasar_frame::{CompletionPolicy, FrameSurface, Gpu};
use quasar_input::Key;
use quasar_window::{
    DesktopConfig, WindowConfig,
    winit::{
        event::{ElementState, WindowEvent},
        keyboard::{KeyCode, PhysicalKey},
    },
};
use std::time::Duration;

pub fn run(width: u32, height: u32) -> Result<()> {
    let config = AppConfig {
        window: WindowConfig {
            title: "Graveyard - Quasar".into(),
            application_id: "graveyard".into(),
            size: (width, height),
            minimum_size: (640, 360),
            fullscreen: false,
            resizable: true,
        },
        desktop: DesktopConfig::for_application("graveyard"),
        timing: TimingConfig::Fixed(FixedStepConfig::for_rate(f64::from(TICK_RATE), 8)),
        render: RenderPolicy::EveryFrame,
        max_frame_wait: Duration::from_millis(16),
        frame_interval: None,
    };
    run_app(GraveyardApp::default(), config).map_err(GraveError::new)
}

#[derive(Default)]
struct GraveyardApp {
    world: World,
    graphics: Option<Graphics>,
    surface: Option<FrameSurface>,
    gal: Option<Gpu>,
    look: [f32; 2],
    toggle: bool,
}

impl GraveyardApp {
    fn initialize(&mut self, context: &mut AppContext<'_>) -> Result<()> {
        if self.graphics.is_some() {
            return Ok(());
        }
        let window = context
            .window
            .ok_or_else(|| GraveError::new("desktop runner did not provide a window"))?;
        let size = window.native().inner_size();
        let (gal, surface) =
            FrameSurface::for_window(window.shared_native(), size.width, size.height)
                .map_err(GraveError::new)?;
        self.graphics = Some(Graphics::new(&gal, &self.world).map_err(GraveError::new)?);
        self.surface = Some(surface);
        self.gal = Some(gal);
        Ok(())
    }

    fn release(&mut self) -> Result<()> {
        if let Some(gal) = self.gal.take() {
            gal.wait().map_err(GraveError::new)?;
            self.graphics = None;
            self.surface = None;
            gal.wait().map_err(GraveError::new)?;
        }
        Ok(())
    }
}

impl Application for GraveyardApp {
    type Error = GraveError;
    fn setup(&mut self, context: &mut AppContext<'_>) -> Result<()> {
        self.initialize(context)
    }
    fn frame_update(&mut self, context: &mut AppContext<'_>) -> Result<Flow> {
        if context.input.pressed(Key::Escape) {
            return Ok(Flow::Exit);
        }
        // Mouse look accumulates per frame and is applied on the next tick.
        let (dx, dy) = context.input.relative_motion();
        self.look[0] -= dx as f32 * 0.0024;
        self.look[1] -= dy as f32 * 0.0024;
        self.toggle |= context.input.pressed(Key::KeyF);
        Ok(Flow::Continue)
    }
    fn tick(&mut self, context: &mut AppContext<'_>) -> Result<Flow> {
        let i = &context.input;
        let axis = |pos: Key, neg: Key| f32::from(i.held(pos)) - f32::from(i.held(neg));
        self.world.update(Input {
            forward: axis(Key::KeyW, Key::KeyS),
            strafe: axis(Key::KeyD, Key::KeyA),
            look: std::mem::take(&mut self.look),
            sprint: i.held(Key::ShiftLeft),
            toggle_flashlight: std::mem::take(&mut self.toggle),
        });
        Ok(Flow::Continue)
    }
    fn render(&mut self, context: &mut AppContext<'_>) -> Result<Flow> {
        // `elapsed` is the wall time since the previous frame.
        let dt = context.elapsed.as_secs_f32().min(0.25);
        let (Some(surface), Some(graphics), Some(gal)) =
            (&mut self.surface, &mut self.graphics, &mut self.gal)
        else {
            return Ok(Flow::Continue);
        };
        let world = &self.world;
        surface
            .render(gal, CompletionPolicy::Poll, |gal, target| {
                graphics.frame(gal, target, world, dt)
            })
            .map_err(GraveError::new)?;
        Ok(Flow::Continue)
    }
    fn suspended(&mut self, _: &mut AppContext<'_>) -> Result<()> {
        self.release()
    }
    fn shutdown(&mut self, _: &mut AppContext<'_>) -> Result<()> {
        self.release()
    }
}

impl DesktopApplication for GraveyardApp {
    fn resumed(&mut self, context: &mut AppContext<'_>) -> Result<()> {
        self.initialize(context)
    }
    fn configure_window(&mut self, window: &mut quasar_window::GameWindow) -> Result<()> {
        let _ = window.capture_cursor(true);
        Ok(())
    }
    fn window_event(&mut self, context: &mut AppContext<'_>, event: &WindowEvent) -> Result<()> {
        if let WindowEvent::KeyboardInput { event, .. } = event
            && event.state == ElementState::Pressed
            && !event.repeat
            && matches!(event.physical_key, PhysicalKey::Code(KeyCode::F11))
            && let Some(window) = context.window
        {
            window.toggle_fullscreen();
        }
        if let WindowEvent::Focused(true) = event
            && let Some(window) = context.window
        {
            let _ = window.capture_cursor(true);
        }
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
        ) && let (Some(surface), Some(gal), Some(window)) =
            (&mut self.surface, &mut self.gal, context.window)
        {
            let size = window.native().inner_size();
            let _ = surface.finish(gal);
            surface.resize(size.width.max(1), size.height.max(1));
        }
        Ok(())
    }
}
