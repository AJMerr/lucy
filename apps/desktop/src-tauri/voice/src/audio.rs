//! The microphone and speakers, through cpal: the mic gives 16 kHz mono, and speech plays from a
//! queue that can be cut short. cpal's streams belong to the thread that opened them, so the
//! app keeps these on its voice thread.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::VoiceError;
use crate::engine::{Audio, SAMPLE_RATE};

/// Linear resampling between rates, carrying its position across chunks.
#[derive(Debug, Clone)]
pub struct Resampler {
    step: f64,
    position: f64,
    last: f32,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        Self {
            step: f64::from(from) / f64::from(to),
            position: 0.0,
            last: 0.0,
        }
    }

    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if input.is_empty() {
            return;
        }
        // Positions are relative to `last` (index -1) then `input`.
        while self.position < input.len() as f64 {
            let at = self.position.floor();
            let frac = (self.position - at) as f32;
            let i = at as isize;
            let a = if i < 0 { self.last } else { input[i as usize] };
            let b = input[((i + 1).max(0) as usize).min(input.len() - 1)];
            out.push(a + (b - a) * frac);
            self.position += self.step;
        }
        self.position -= input.len() as f64;
        self.last = input[input.len() - 1];
    }
}

/// The microphone, open until dropped.
pub struct Mic {
    _stream: cpal::Stream,
}

impl Mic {
    /// Open the default microphone; `sink` gets 16 kHz mono chunks.
    pub fn open(mut sink: impl FnMut(Vec<f32>) + Send + 'static) -> Result<Self, VoiceError> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| VoiceError::Audio("no microphone found".into()))?;
        let config = device
            .default_input_config()
            .map_err(|err| VoiceError::Audio(format!("the microphone: {err}")))?;
        let channels = config.channels().max(1) as usize;
        let mut resampler = Resampler::new(config.sample_rate().0, SAMPLE_RATE);
        let mut mono = Vec::new();
        let mut out = Vec::new();
        let mut on_data = move |data: &[f32]| {
            mono.clear();
            mono.extend(
                data.chunks(channels)
                    .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32),
            );
            out.clear();
            resampler.push(&mono, &mut out);
            sink(std::mem::take(&mut out));
        };
        let err = |err| eprintln!("lucy-voice: microphone: {err}");
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config.into(),
                move |d: &[f32], _: &_| on_data(d),
                err,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config.into(),
                move |d: &[i16], _: &_| {
                    let f: Vec<f32> = d.iter().map(|s| f32::from(*s) / 32768.0).collect();
                    on_data(&f);
                },
                err,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                &config.into(),
                move |d: &[u16], _: &_| {
                    let f: Vec<f32> = d
                        .iter()
                        .map(|s| (f32::from(*s) - 32768.0) / 32768.0)
                        .collect();
                    on_data(&f);
                },
                err,
                None,
            ),
            other => {
                return Err(VoiceError::Audio(format!(
                    "unsupported microphone format {other}"
                )));
            }
        }
        .map_err(|err| VoiceError::Audio(format!("the microphone: {err}")))?;
        stream
            .play()
            .map_err(|err| VoiceError::Audio(format!("the microphone: {err}")))?;
        Ok(Self { _stream: stream })
    }
}

/// The speakers: speech queued to play in order, and cut short with `stop`.
pub struct Speaker {
    _stream: cpal::Stream,
    queue: Arc<Mutex<VecDeque<f32>>>,
    rate: u32,
}

impl Speaker {
    pub fn open() -> Result<Self, VoiceError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| VoiceError::Audio("no speakers found".into()))?;
        let config = device
            .default_output_config()
            .map_err(|err| VoiceError::Audio(format!("the speakers: {err}")))?;
        let channels = config.channels().max(1) as usize;
        let rate = config.sample_rate().0;
        let queue: Arc<Mutex<VecDeque<f32>>> = Arc::default();
        let source = Arc::clone(&queue);
        let err = |err| eprintln!("lucy-voice: speakers: {err}");
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &config.into(),
                move |out: &mut [f32], _: &_| fill(out, channels, &source, |s| s),
                err,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                &config.into(),
                move |out: &mut [i16], _: &_| {
                    fill(out, channels, &source, |s| {
                        (s.clamp(-1.0, 1.0) * 32767.0) as i16
                    })
                },
                err,
                None,
            ),
            other => {
                return Err(VoiceError::Audio(format!(
                    "unsupported speaker format {other}"
                )));
            }
        }
        .map_err(|err| VoiceError::Audio(format!("the speakers: {err}")))?;
        stream
            .play()
            .map_err(|err| VoiceError::Audio(format!("the speakers: {err}")))?;
        Ok(Self {
            _stream: stream,
            queue,
            rate,
        })
    }

    /// Queue `audio` after whatever is playing.
    pub fn play(&self, audio: &Audio) {
        let mut resampled = Vec::new();
        Resampler::new(audio.sample_rate, self.rate).push(&audio.samples, &mut resampled);
        self.queue.lock().unwrap().extend(resampled);
    }

    /// Stop now, dropping anything queued.
    pub fn stop(&self) {
        self.queue.lock().unwrap().clear();
    }

    pub fn is_playing(&self) -> bool {
        !self.queue.lock().unwrap().is_empty()
    }
}

fn fill<T: Copy>(
    out: &mut [T],
    channels: usize,
    queue: &Mutex<VecDeque<f32>>,
    convert: impl Fn(f32) -> T,
) {
    let mut queue = queue.lock().unwrap();
    for frame in out.chunks_mut(channels) {
        let sample = convert(queue.pop_front().unwrap_or(0.0));
        frame.fill(sample);
    }
}
