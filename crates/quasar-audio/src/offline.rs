//! Offline, device-free rendering convenience.

use crate::error::AudioResult;
use crate::mixer::{Mixer, MixerConfig, MixerDiagnostics};
use std::path::Path;

/// Deterministic offline mixer wrapper.
///
/// Use this in tests, headless replay and CI to compare actual PCM samples and
/// WAV output without speakers. Scheduling and rendering are sample-frame exact.
pub struct OfflineRenderer {
    mixer: Mixer,
}
impl OfflineRenderer {
    /// Creates an offline renderer.
    pub fn new(config: MixerConfig) -> AudioResult<Self> {
        Ok(Self {
            mixer: Mixer::new(config)?,
        })
    }
    /// Access the underlying mixer for voices, buses and listener control.
    pub fn mixer(&self) -> &Mixer {
        &self.mixer
    }
    /// Mutable mixer access.
    pub fn mixer_mut(&mut self) -> &mut Mixer {
        &mut self.mixer
    }
    /// Renders interleaved `f32` PCM.
    pub fn render(&mut self, frames: usize) -> AudioResult<Vec<f32>> {
        self.mixer.render(frames)
    }
    /// Renders a PCM16 WAV file in bounded chunks.
    pub fn render_wav(&mut self, path: impl AsRef<Path>, frames: usize) -> AudioResult<()> {
        self.mixer.render_wav(path, frames)
    }
    /// Current diagnostics.
    pub fn diagnostics(&self) -> MixerDiagnostics {
        self.mixer.diagnostics()
    }
}
