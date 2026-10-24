use quasar_app::{AppContext, Application, Flow, HeadlessConfig, run_headless};

#[derive(Default)]
struct Demo {
    ticks: u64,
}
impl Application for Demo {
    type Error = std::convert::Infallible;
    fn setup(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        println!("headless setup");
        Ok(())
    }
    fn tick(&mut self, _: &mut AppContext<'_>) -> Result<Flow, Self::Error> {
        self.ticks += 1;
        Ok(if self.ticks == 3 {
            Flow::Exit
        } else {
            Flow::Continue
        })
    }
    fn shutdown(&mut self, _: &mut AppContext<'_>) -> Result<(), Self::Error> {
        println!("headless shutdown after {} ticks", self.ticks);
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_headless(
        Demo::default(),
        HeadlessConfig {
            ticks: 100,
            ..HeadlessConfig::default()
        },
    )
}
