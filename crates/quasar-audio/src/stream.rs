//! Streaming Ogg Vorbis playback.
//!
//! Decoding runs on a bounded producer thread; the mixing callback only drains
//! decoded stereo chunks. Whole-file loops and sample-frame regions are decoded
//! and trimmed in the worker, so the mixer never seeks or reads files.
//!
//! Streaming WAV is deliberately rejected: WAV is uncompressed and is loaded as
//! a [`Clip`](crate::Clip); converting one into a stream would not save memory.
//! This is a documented limitation, not a silent fallback.

use crate::decode::{AudioFormat, MAX_CHANNELS, MAX_SAMPLE_RATE, MIN_SAMPLE_RATE};
use crate::error::{AudioError, AudioResult};
use crate::mixer::LoopMode;
use crossbeam_channel::{Receiver, Sender};
use lewton::inside_ogg::OggStreamReader;
use lewton::samples::InterleavedSamples;
use std::{
    fs::File,
    io::{Cursor, Read, Seek},
    path::{Path, PathBuf},
    sync::Arc,
};

const STREAM_CHUNK_FRAMES: usize = 1024;
const STREAM_QUEUE_DEPTH: usize = 4;

#[derive(Clone)]
enum Input {
    File(PathBuf),
    Memory(Arc<[u8]>),
}
impl Input {
    fn open(&self) -> AudioResult<Box<dyn ReadSeek>> {
        match self {
            Self::File(path) => Ok(Box::new(std::io::BufReader::new(File::open(path)?))),
            Self::Memory(bytes) => Ok(Box::new(Cursor::new(bytes.clone()))),
        }
    }
}
trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

#[derive(Clone)]
struct StreamSpec {
    input: Input,
    channels: u16,
    sample_rate: u32,
    label: Arc<str>,
}
/// A file- or memory-backed Ogg Vorbis stream. Starting a voice spawns a
/// bounded decoder thread; dropping the last stream voice closes the channel and
/// lets the thread exit.
#[derive(Clone)]
pub struct StreamSource {
    spec: Arc<StreamSpec>,
}
impl StreamSource {
    /// Opens an Ogg file for streaming. The container is detected from bytes,
    /// not the filename extension.
    pub fn from_path(path: impl AsRef<Path>) -> AudioResult<Self> {
        let path = path.as_ref().to_path_buf();
        let mut file = File::open(&path)?;
        let mut signature = [0u8; 4];
        file.read_exact(&mut signature)?;
        if &signature == b"RIFF" {
            return Err(AudioError::Unsupported(
                "streaming WAV is not supported; load WAV with Clip::load".into(),
            ));
        }
        if &signature != b"OggS" {
            return Err(AudioError::Decode(format!(
                "{} is not an Ogg Vorbis stream",
                path.display()
            )));
        }
        drop(file);
        Self::new(Input::File(path.clone()), path.to_string_lossy().into())
    }
    /// Opens an in-memory Ogg Vorbis stream.
    pub fn from_bytes(
        bytes: impl Into<Arc<[u8]>>,
        label: impl Into<Arc<str>>,
    ) -> AudioResult<Self> {
        let bytes = bytes.into();
        if AudioFormat::detect(&bytes) != Some(AudioFormat::Ogg) {
            return Err(AudioError::Unsupported(
                "streaming memory sources must be Ogg Vorbis".into(),
            ));
        }
        Self::new(Input::Memory(bytes), label.into())
    }
    fn new(input: Input, label: Arc<str>) -> AudioResult<Self> {
        let (channels, sample_rate) = probe(&input)?;
        if !(1..=MAX_CHANNELS).contains(&channels) {
            return Err(AudioError::Unsupported(
                "stream must be mono or stereo".into(),
            ));
        }
        if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&sample_rate) {
            return Err(AudioError::Unsupported(
                "stream sample rate is outside the supported range".into(),
            ));
        }
        Ok(Self {
            spec: Arc::new(StreamSpec {
                input,
                channels,
                sample_rate,
                label,
            }),
        })
    }
    /// Source channel count.
    pub fn channels(&self) -> u16 {
        self.spec.channels
    }
    /// Source sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.spec.sample_rate
    }
    /// Diagnostic label.
    pub fn label(&self) -> &str {
        &self.spec.label
    }
    pub(crate) fn reader(&self, looping: LoopMode) -> AudioResult<StreamReader> {
        if let LoopMode::Region(region) = looping
            && region.start >= region.end
        {
            return Err(AudioError::InvalidArgument(
                "stream loop region must have start < end".into(),
            ));
        }
        let (sender, receiver) = crossbeam_channel::bounded(STREAM_QUEUE_DEPTH);
        let input = self.spec.input.clone();
        let channels = self.spec.channels;
        let sample_rate = self.spec.sample_rate;
        std::thread::Builder::new()
            .name("quasar-audio-stream".into())
            .spawn(move || stream_worker(input, looping, channels, sender))
            .map_err(|error| AudioError::Device(format!("start stream decoder: {error}")))?;
        Ok(StreamReader {
            receiver,
            // Reserve the entire bounded queue so the mixing callback never
            // grows this buffer for ordinary playback.
            frames: Vec::with_capacity(STREAM_QUEUE_DEPTH * STREAM_CHUNK_FRAMES * 2),
            base: 0,
            sample_rate,
            done: false,
            error: None,
        })
    }
}

fn probe(input: &Input) -> AudioResult<(u16, u32)> {
    let reader = OggStreamReader::new(input.open()?)?;
    Ok((
        reader.ident_hdr.audio_channels as u16,
        reader.ident_hdr.audio_sample_rate,
    ))
}

fn stream_worker(
    input: Input,
    looping: LoopMode,
    channels: u16,
    sender: Sender<Result<Vec<[f32; 2]>, AudioError>>,
) {
    let mut skip_after_loop = false;
    loop {
        let mut decoder = match OggDecoder::open(input.clone(), channels) {
            Ok(decoder) => decoder,
            Err(error) => {
                let _ = sender.send(Err(error));
                return;
            }
        };
        let mut emitted = 0u64;
        if skip_after_loop {
            let skip = match looping {
                LoopMode::Region(region) => region.start,
                _ => 0,
            };
            if let Err(error) = decoder.skip(skip) {
                let _ = sender.send(Err(error));
                return;
            }
            emitted = skip;
        }
        loop {
            let chunk = match decoder.next_chunk(STREAM_CHUNK_FRAMES) {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(error) => {
                    let _ = sender.send(Err(error));
                    return;
                }
            };
            let mut chunk = chunk;
            if let LoopMode::Region(region) = looping {
                if emitted >= region.end {
                    break;
                }
                let room = usize::try_from(region.end - emitted).unwrap_or(usize::MAX);
                if chunk.len() > room {
                    chunk.truncate(room);
                }
            }
            emitted = emitted.saturating_add(chunk.len() as u64);
            if sender.send(Ok(chunk)).is_err() {
                return;
            }
            if let LoopMode::Region(region) = looping
                && emitted >= region.end
            {
                break;
            }
        }
        if emitted == 0 {
            // An empty or malformed stream produced no progress. Do not spin
            // forever trying to loop it.
            return;
        }
        match looping {
            LoopMode::None => return,
            LoopMode::Whole => {}
            LoopMode::Region(_) => skip_after_loop = true,
        }
    }
}

struct OggDecoder {
    reader: OggStreamReader<Box<dyn ReadSeek>>,
    channels: u16,
    pending: Vec<f32>,
    eof: bool,
}
impl OggDecoder {
    fn open(input: Input, channels: u16) -> AudioResult<Self> {
        let reader = OggStreamReader::new(input.open()?)?;
        let actual_channels = reader.ident_hdr.audio_channels as u16;
        if actual_channels != channels {
            return Err(AudioError::Decode(
                "stream channel layout changed while reopening".into(),
            ));
        }
        Ok(Self {
            reader,
            channels,
            pending: Vec::new(),
            eof: false,
        })
    }
    fn next_chunk(&mut self, max_frames: usize) -> AudioResult<Option<Vec<[f32; 2]>>> {
        let wanted = max_frames
            .checked_mul(self.channels as usize)
            .ok_or_else(|| AudioError::Decode("stream frame count overflow".into()))?;
        while self.pending.len() < wanted && !self.eof {
            match self
                .reader
                .read_dec_packet_generic::<InterleavedSamples<f32>>()?
            {
                Some(packet) => {
                    let next = self
                        .pending
                        .len()
                        .checked_add(packet.samples.len())
                        .ok_or_else(|| AudioError::Decode("stream sample count overflow".into()))?;
                    if next > 64 * 1024 * 1024 {
                        return Err(AudioError::Decode("stream chunk exceeds budget".into()));
                    }
                    self.pending.extend(packet.samples);
                }
                None => {
                    self.eof = true;
                    break;
                }
            }
        }
        if self.pending.is_empty() {
            return Ok(None);
        }
        let take = self.pending.len().min(wanted);
        let take = take - (take % self.channels as usize);
        if take == 0 {
            return Ok(None);
        }
        let samples: Vec<f32> = self.pending.drain(..take).collect();
        let frames = samples
            .chunks_exact(self.channels as usize)
            .map(|frame| {
                if frame.len() == 2 {
                    [frame[0], frame[1]]
                } else {
                    [frame[0], frame[0]]
                }
            })
            .collect::<Vec<_>>();
        Ok(Some(frames))
    }
    fn skip(&mut self, frames: u64) -> AudioResult<()> {
        let mut left = frames;
        while left > 0 {
            let take = usize::try_from(left.min(STREAM_CHUNK_FRAMES as u64))
                .unwrap_or(STREAM_CHUNK_FRAMES);
            let Some(chunk) = self.next_chunk(take)? else {
                return Err(AudioError::Decode(
                    "stream loop start is beyond the decoded stream".into(),
                ));
            };
            left = left.saturating_sub(chunk.len() as u64);
        }
        Ok(())
    }
}

pub(crate) struct StreamReader {
    receiver: Receiver<Result<Vec<[f32; 2]>, AudioError>>,
    frames: Vec<[f32; 2]>,
    base: u64,
    sample_rate: u32,
    done: bool,
    error: Option<String>,
}
impl StreamReader {
    pub(crate) fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    /// Samples at a source-relative frame position. Returns `(frame, underrun)`.
    pub(crate) fn sample(&mut self, position: f64) -> (Option<[f32; 2]>, bool) {
        let needed = position.floor() as u64 + 2;
        let mut underrun = false;
        while self.base + (self.frames.len() as u64) < needed && !self.done {
            match self.receiver.try_recv() {
                Ok(Ok(frames)) => self.frames.extend_from_slice(&frames),
                Ok(Err(error)) => {
                    self.error = Some(error.to_string());
                    self.done = true;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {
                    underrun = true;
                    break;
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.done = true;
                }
            }
        }
        if position < self.base as f64 {
            return (None, underrun);
        }
        let index = (position.floor() as u64 - self.base) as usize;
        let fraction = position - position.floor();
        let Some(&a) = self.frames.get(index) else {
            return (None, underrun && !self.done);
        };
        let Some(&b) = self.frames.get(index + 1) else {
            return (Some(a), underrun);
        };
        if position - self.base as f64 > 8192.0 {
            let drain = (position - self.base as f64) as usize - 4096;
            if drain < self.frames.len() {
                self.frames.drain(..drain);
                self.base += drain as u64;
            }
        }
        (
            Some([
                a[0] + (b[0] - a[0]) * fraction as f32,
                a[1] + (b[1] - a[1]) * fraction as f32,
            ]),
            underrun,
        )
    }
}
