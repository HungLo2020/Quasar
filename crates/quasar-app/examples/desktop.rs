use quasar_app::{
    AppConfig, AppContext, DesktopApplication, Flow, HeadlessConfig, run, run_headless,
};

#[derive(Default)]
struct Demo {
    frames: u64,
    ticks: u64,
}
impl quasar_app::Application for Demo {
    type Error = std::convert::Infallible;

    fn setup(&mut self, context: &mut AppContext<'_>) -> Result<(), Self::Error> {
        println!(
            "{} setup size={:?}",
            if context.window.is_some() {
                "desktop"
            } else {
                "headless"
            },
            context.size
        );
        Ok(())
    }

    fn frame_update(&mut self, context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        self.frames += 1;
        // Variable-rate work sees this frame's input snapshot even when no
        // fixed tick is due.
        if context.input.pressed(quasar_input::Key::Space) {
            println!("frame {} space pressed", context.frame);
        }
        Ok(Flow::Continue)
    }

    fn tick(&mut self, context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        self.ticks += 1;
        if self.ticks == 30 {
            println!(
                "stopping after {} ticks at simulation_time={:?}",
                self.ticks, context.timing.simulation_time
            );
            return Ok(Flow::Exit);
        }
        Ok(Flow::Continue)
    }

    fn render(&mut self, context: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        println!(
            "render frame={} tick={} interpolation={:.3}",
            context.frame, context.tick, context.interpolation
        );
        Ok(Flow::Continue)
    }

    fn shutdown(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        println!(
            "shutdown after {} frames and {} ticks",
            self.frames, self.ticks
        );
        Ok(())
    }
}
impl DesktopApplication for Demo {}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The same application type runs under the native runner or headlessly.
    if std::env::args().any(|argument| argument == "--headless") {
        return run_headless(
            Demo::default(),
            HeadlessConfig {
                ticks: 100,
                ..HeadlessConfig::default()
            },
        );
    }
    run(Demo::default(), AppConfig::default())
}
