//! Decoded PCM assets. WAV and Ogg Vorbis decode into a bounded, immutable
//! interleaved `f32` buffer shared by voices without copying.

use crate::error::{AudioError, AudioResult};
use hound::{SampleFormat, WavReader};
use lewton::inside_ogg::OggStreamReader;
use lewton::samples::InterleavedSamples;
use std::{fs, io::Cursor, path::Path, sync::Arc};

/// Default aggregate decoded-PCM budget for one asset.
pub const MAX_DECODED_BYTES: usize = 256 * 1024 * 1024;
/// Supported sample-rate range for decoded and generated sources.
pub const MIN_SAMPLE_RATE: u32 = 4_000;
/// Upper supported sample rate.
pub const MAX_SAMPLE_RATE: u32 = 384_000;
/// Maximum interleaved channel count in this mixer.
pub const MAX_CHANNELS: u16 = 2;

/// Container formats understood by [`Clip::from_bytes`] and streaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    /// RIFF/WAVE, including PCM 8/16/24/32 and IEEE float 32.
    Wav,
    /// Ogg Vorbis.
    Ogg,
}
impl AudioFormat {
    /// Detect a container from its leading bytes.
    pub fn detect(bytes: &[u8]) -> Option<Self> {
        if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
            Some(Self::Wav)
        } else if bytes.starts_with(b"OggS") {
            Some(Self::Ogg)
        } else {
            None
        }
    }
}

#[derive(Debug)]
struct ClipInner {
    samples: Arc<[f32]>,
    channels: u16,
    sample_rate: u32,
    label: Arc<str>,
}
/// Immutable decoded audio, safe to share between many voices.
///
/// Samples are interleaved `f32` in `[-1, 1]` for file assets.
/// `Arc` sharing means starting a voice does not copy PCM.
#[derive(Clone, Debug)]
pub struct Clip {
    inner: Arc<ClipInner>,
}
impl Clip {
    /// Creates a clip from interleaved `f32` samples.
    pub fn from_samples(
        samples: Vec<f32>,
        channels: u16,
        sample_rate: u32,
        label: impl Into<Arc<str>>,
    ) -> AudioResult<Self> {
        validate_format(channels, sample_rate)?;
        if samples.is_empty() {
            return Err(AudioError::Decode("clip contains no samples".into()));
        }
        if !samples.len().is_multiple_of(channels as usize) {
            return Err(AudioError::Decode(
                "interleaved sample count is not a whole number of frames".into(),
            ));
        }
        let bytes = samples
            .len()
            .checked_mul(std::mem::size_of::<f32>())
            .ok_or_else(|| AudioError::Decode("clip byte count overflow".into()))?;
        if bytes > MAX_DECODED_BYTES {
            return Err(AudioError::Decode(format!(
                "clip exceeds {MAX_DECODED_BYTES} decoded bytes"
            )));
        }
        if samples.iter().any(|sample| !sample.is_finite()) {
            return Err(AudioError::Decode(
                "clip contains non-finite samples".into(),
            ));
        }
        Ok(Self {
            inner: Arc::new(ClipInner {
                samples: samples.into(),
                channels,
                sample_rate,
                label: label.into(),
            }),
        })
    }

    /// Creates a clip from signed 16-bit PCM.
    pub fn from_pcm16(
        samples: Vec<i16>,
        channels: u16,
        sample_rate: u32,
        label: impl Into<Arc<str>>,
    ) -> AudioResult<Self> {
        if samples.len() > MAX_DECODED_BYTES / std::mem::size_of::<f32>() {
            return Err(AudioError::Decode(format!(
                "PCM exceeds the {MAX_DECODED_BYTES}-byte decoded budget"
            )));
        }
        Self::from_samples(
            samples
                .into_iter()
                .map(|sample| f32::from(sample) / 32768.0)
                .collect(),
            channels,
            sample_rate,
            label,
        )
    }

    /// Decodes a WAV or Ogg Vorbis file, auto-detecting the container.
    pub fn load(path: impl AsRef<Path>) -> AudioResult<Self> {
        let bytes = read_file_bounded(path.as_ref())?;
        Self::from_bytes(&bytes, None, path.as_ref().to_string_lossy())
    }

    /// Decodes an in-memory WAV or Ogg Vorbis asset.
    pub fn from_bytes(
        bytes: &[u8],
        hint: Option<AudioFormat>,
        label: impl Into<Arc<str>>,
    ) -> AudioResult<Self> {
        let format = hint
            .or_else(|| AudioFormat::detect(bytes))
            .ok_or_else(|| AudioError::Decode("unrecognized audio container".into()))?;
        match format {
            AudioFormat::Wav => decode_wav(bytes, label),
            AudioFormat::Ogg => decode_ogg(bytes, label),
        }
    }

    /// Interleaved samples.
    pub fn samples(&self) -> &[f32] {
        &self.inner.samples
    }
    /// Channel count (1 or 2).
    pub fn channels(&self) -> u16 {
        self.inner.channels
    }
    /// Source sample rate in Hz.
    pub fn sample_rate(&self) -> u32 {
        self.inner.sample_rate
    }
    /// Number of sample frames.
    pub fn frames(&self) -> u64 {
        self.inner.samples.len() as u64 / u64::from(self.inner.channels)
    }
    /// Duration in seconds.
    pub fn duration_seconds(&self) -> f64 {
        self.frames() as f64 / f64::from(self.inner.sample_rate)
    }
    /// Diagnostic label retained from the source path or caller.
    pub fn label(&self) -> &str {
        &self.inner.label
    }
    pub(crate) fn sample_at(&self, frame: u64, channel: usize) -> f32 {
        let index = frame as usize * self.inner.channels as usize + channel;
        self.inner.samples.get(index).copied().unwrap_or(0.0)
    }
}

fn validate_format(channels: u16, sample_rate: u32) -> AudioResult<()> {
    if !(1..=MAX_CHANNELS).contains(&channels) {
        return Err(AudioError::Unsupported(format!(
            "{channels} channels; only mono/stereo are supported"
        )));
    }
    if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&sample_rate) {
        return Err(AudioError::Unsupported(format!(
            "sample rate {sample_rate} Hz is outside {MIN_SAMPLE_RATE}..={MAX_SAMPLE_RATE}"
        )));
    }
    Ok(())
}
fn read_file_bounded(path: &Path) -> AudioResult<Vec<u8>> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_DECODED_BYTES as u64 {
        return Err(AudioError::Decode(format!(
            "{} exceeds the {MAX_DECODED_BYTES}-byte full-decode limit; use streaming",
            path.display()
        )));
    }
    Ok(fs::read(path)?)
}

fn decode_wav(bytes: &[u8], label: impl Into<Arc<str>>) -> AudioResult<Clip> {
    let mut reader = WavReader::new(Cursor::new(bytes))?;
    let spec = reader.spec();
    validate_format(spec.channels, spec.sample_rate)?;
    if !matches!(spec.bits_per_sample, 8 | 16 | 24 | 32) {
        return Err(AudioError::Unsupported(format!(
            "WAV bit depth {}",
            spec.bits_per_sample
        )));
    }
    let bytes_per_sample = match spec.sample_format {
        SampleFormat::Float => 4usize,
        SampleFormat::Int => usize::from(spec.bits_per_sample / 8),
    };
    let expected_bytes = usize::try_from(reader.len())
        .map_err(|_| AudioError::Decode("WAV sample count overflow".into()))?
        .checked_mul(bytes_per_sample)
        .ok_or_else(|| AudioError::Decode("WAV sample count overflow".into()))?;
    if expected_bytes > MAX_DECODED_BYTES {
        return Err(AudioError::Decode("WAV exceeds the decoded budget".into()));
    }
    let samples = match spec.sample_format {
        SampleFormat::Float => reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?,
        SampleFormat::Int => {
            let scale = 1.0f32 / (1u64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|sample| sample.map(|value| (value as f32 * scale).clamp(-1.0, 1.0)))
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    Clip::from_samples(samples, spec.channels, spec.sample_rate, label)
}

fn decode_ogg(bytes: &[u8], label: impl Into<Arc<str>>) -> AudioResult<Clip> {
    let final_frames = final_ogg_granule_position(bytes)?;
    let mut reader = OggStreamReader::new(Cursor::new(bytes))?;
    let channels = reader.ident_hdr.audio_channels as u16;
    let sample_rate = reader.ident_hdr.audio_sample_rate;
    validate_format(channels, sample_rate)?;
    let mut samples = Vec::new();
    while let Some(packet) = reader.read_dec_packet_generic::<InterleavedSamples<f32>>()? {
        let next = samples
            .len()
            .checked_add(packet.samples.len())
            .ok_or_else(|| AudioError::Decode("Ogg sample count overflow".into()))?;
        if next
            .checked_mul(4)
            .is_none_or(|bytes| bytes > MAX_DECODED_BYTES)
        {
            return Err(AudioError::Decode("Ogg exceeds the decoded budget".into()));
        }
        samples.extend(packet.samples);
    }
    if let Some(frames) = final_frames {
        let target = usize::try_from(frames)
            .ok()
            .and_then(|frames| frames.checked_mul(channels as usize))
            .ok_or_else(|| AudioError::Decode("Ogg granule overflow".into()))?;
        if samples.len() > target {
            samples.truncate(target);
        }
    }
    Clip::from_samples(samples, channels, sample_rate, label)
}

/// Reads the last Ogg page granule position, used to trim decoder padding.
///
/// This walks real page boundaries instead of searching for `OggS` bytes, which
/// could otherwise match audio payload data. A granule of `u64::MAX` means "no
/// completed packets" and is ignored.
fn final_ogg_granule_position(bytes: &[u8]) -> AudioResult<Option<u64>> {
    let mut cursor = 0usize;
    let mut final_granule = None;
    while cursor < bytes.len() {
        if bytes.len() - cursor < 27 {
            return Err(AudioError::Decode("truncated Ogg page header".into()));
        }
        if &bytes[cursor..cursor + 4] != b"OggS" {
            return Err(AudioError::Decode(format!(
                "missing Ogg capture pattern at byte {cursor}"
            )));
        }
        let header_type = bytes[cursor + 5];
        let segments = usize::from(bytes[cursor + 26]);
        let table_start = cursor + 27;
        let table_end = table_start
            .checked_add(segments)
            .ok_or_else(|| AudioError::Decode("Ogg segment table overflow".into()))?;
        if table_end > bytes.len() {
            return Err(AudioError::Decode("truncated Ogg segment table".into()));
        }
        let mut page_end = table_end;
        for lacing in &bytes[table_start..table_end] {
            page_end = page_end
                .checked_add(usize::from(*lacing))
                .ok_or_else(|| AudioError::Decode("Ogg page length overflow".into()))?;
        }
        if page_end > bytes.len() {
            return Err(AudioError::Decode("truncated Ogg page data".into()));
        }
        let granule = u64::from_le_bytes([
            bytes[cursor + 6],
            bytes[cursor + 7],
            bytes[cursor + 8],
            bytes[cursor + 9],
            bytes[cursor + 10],
            bytes[cursor + 11],
            bytes[cursor + 12],
            bytes[cursor + 13],
        ]);
        if granule != u64::MAX {
            final_granule = Some(granule);
        }
        cursor = page_end;
        if header_type & 0x04 != 0 {
            break;
        }
    }
    Ok(final_granule)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_pcm_is_released_with_the_last_reference() {
        let clip = Clip::from_samples(vec![0.1; 1024], 1, 48_000, "reclaim").unwrap();
        let weak = Arc::downgrade(&clip.inner);
        let clone = clip.clone();
        drop(clip);
        assert!(weak.upgrade().is_some());
        drop(clone);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn pcm16_scaling_and_validation_are_exact() {
        let clip = Clip::from_pcm16(vec![0, 16_384, -32_768], 1, 48_000, "pcm16").unwrap();
        assert_eq!(clip.frames(), 3);
        assert_eq!(clip.samples()[0], 0.0);
        assert!((clip.samples()[1] - 0.5).abs() < 1e-6);
        assert_eq!(clip.samples()[2], -1.0);

        assert!(Clip::from_samples(vec![0.0, 1.0, 0.0], 2, 48_000, "odd").is_err());
        assert!(Clip::from_samples(vec![0.0], 1, 1_000, "slow").is_err());
        assert!(Clip::from_samples(vec![f32::NAN], 1, 48_000, "nan").is_err());
    }

    #[test]
    fn float_wav_samples_are_decoded_without_quantization() {
        let mut bytes = Vec::new();
        {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: 48_000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            };
            let mut writer = hound::WavWriter::new(Cursor::new(&mut bytes), spec).unwrap();
            for sample in [0.0f32, 0.25, -0.5] {
                writer.write_sample(sample).unwrap();
            }
            writer.finalize().unwrap();
        }
        let clip = Clip::from_bytes(&bytes, Some(AudioFormat::Wav), "float").unwrap();
        assert_eq!(clip.frames(), 3);
        assert!((clip.samples()[1] - 0.25).abs() < 1e-7);
        assert!((clip.samples()[2] + 0.5).abs() < 1e-7);
    }
}
