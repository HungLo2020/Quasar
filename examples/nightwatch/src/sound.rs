//! Small procedural sounds demonstrate optional quasar-audio reuse, without asset files.
use nightwatch::world::Weapon;
use quasar_audio::{AudioEngine, Clip, EngineConfig, PlaybackConfig};
pub struct Audio {
    engine: AudioEngine,
    pistol: Clip,
    rifle: Clip,
    hurt: Clip,
    reload: Clip,
}
impl Audio {
    pub fn open(device: Option<&str>) -> Option<Self> {
        match AudioEngine::open(device, EngineConfig::default()) {
            Ok(engine) => Some(Self {
                engine,
                pistol: clip(0.20, 90., 42.),
                rifle: clip(0.14, 130., 65.),
                hurt: clip(0.23, 48., 12.),
                reload: clip(0.08, 900., 80.),
            }),
            Err(e) => {
                eprintln!("nightwatch audio unavailable: {e}; continuing silently");
                None
            }
        }
    }
    fn play(&self, clip: &Clip, volume: f32) {
        let _ = self.engine.play(
            clip.clone(),
            PlaybackConfig {
                volume,
                ..PlaybackConfig::default()
            },
        );
    }
    pub fn fire(&self, weapon: Weapon) {
        self.play(
            if weapon == Weapon::Pistol {
                &self.pistol
            } else {
                &self.rifle
            },
            0.5,
        );
    }
    pub fn hurt(&self) {
        self.play(&self.hurt, 0.4);
    }
    pub fn reload(&self) {
        self.play(&self.reload, 0.25);
    }
}
fn clip(seconds: f32, pitch: f32, decay: f32) -> Clip {
    let mut rng = 0x12345678u32;
    let mut low = 0.;
    let samples = (0..(seconds * 48000.) as usize)
        .map(|i| {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            let t = i as f32 / 48000.;
            let noise = (rng as f32 / u32::MAX as f32) * 2. - 1.;
            low = low * 0.6 + noise * 0.4;
            ((t * pitch * std::f32::consts::TAU).sin() * 0.4 + low * 0.6) * (-t * decay).exp()
        })
        .collect();
    Clip::from_samples(samples, 1, 48000, "nightwatch procedural effect")
        .expect("bounded procedural PCM")
}
