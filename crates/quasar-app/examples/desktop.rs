use quasar_app::{
    AppConfig, AppContext, DesktopApplication, Flow, HeadlessConfig, RenderPolicy, run,
    run_headless,
};

#[derive(Default)]
struct Demo {
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
    fn tick(&mut self, _: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        self.ticks += 1;
        Ok(if self.ticks == 30 {
            Flow::Exit
        } else {
            Flow::Continue
        })
    }
    fn shutdown(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        println!("shutdown after {} ticks", self.ticks);
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
    run(
        Demo::default(),
        AppConfig {
            render: RenderPolicy::Manual,
            ..AppConfig::default()
        },
    )
}
