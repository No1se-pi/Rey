use anyhow::{Result, anyhow, ensure};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

#[cfg(target_os = "linux")]
#[path = "audio/linux.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "audio/windows.rs"]
mod platform;

const RATE: u32 = 16_000;
pub struct Capture {
    device: platform::Stream,
    samples: Arc<Mutex<Vec<i16>>>,
    started: Instant,
    pub max_seconds: u32,
}

impl Capture {
    pub fn start(max_seconds: u32, max_bytes: u64) -> Result<Self> {
        let max_samples =
            ((max_bytes.saturating_sub(44) / 2) as usize).min(max_seconds as usize * RATE as usize);
        let max_seconds = max_seconds.min((max_samples / RATE as usize).max(1) as u32);
        let samples = Arc::new(Mutex::new(Vec::with_capacity(max_samples)));
        let buffer = samples.clone();
        let device = platform::capture(RATE, move |input| {
            let Ok(mut buffer) = buffer.try_lock() else {
                return;
            };
            let count = input.len().min(max_samples.saturating_sub(buffer.len()));
            buffer.extend_from_slice(&input[..count]);
        })?;
        Ok(Self {
            device,
            samples,
            started: Instant::now(),
            max_seconds,
        })
    }

    pub fn seconds(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    pub fn stop(self) -> Result<Vec<i16>> {
        platform::stop(&self.device)?;
        let mut samples = self
            .samples
            .lock()
            .map_err(|_| anyhow!("Буфер записи недоступен"))?;
        ensure!(
            samples.len() >= RATE as usize / 10,
            "Запись слишком короткая"
        );
        Ok(std::mem::take(&mut *samples))
    }
}

pub fn write_wav(path: &Path, samples: &[i16]) -> Result<()> {
    let mut wav = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 1,
            sample_rate: RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    let mut writer = wav.get_i16_writer(samples.len() as u32);
    for &sample in samples {
        writer.write_sample(sample);
    }
    writer.flush()?;
    wav.finalize()?;
    Ok(())
}

pub struct Pcm {
    pub samples: Vec<f32>,
    pub rate: u32,
    pub channels: u16,
}

pub fn read_wav(path: &Path) -> Result<Pcm> {
    let mut file = hound::WavReader::open(path)?;
    let format = file.spec();
    ensure!(
        (1..=8).contains(&format.channels) && (8000..=192000).contains(&format.sample_rate),
        "Неподдерживаемые параметры WAV"
    );
    let samples: Vec<f32> = match format.sample_format {
        hound::SampleFormat::Float => file.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let max = (1_u32 << (format.bits_per_sample - 1)) as f32;
            file.samples::<i32>()
                .map(|sample| sample.map(|sample| sample as f32 / max))
                .collect::<Result<_, _>>()?
        }
    };
    ensure!(
        !samples.is_empty() && samples.iter().all(|value| value.is_finite()),
        "WAV не содержит валидных сэмплов"
    );
    Ok(Pcm {
        samples,
        rate: format.sample_rate,
        channels: format.channels,
    })
}

pub struct Playback {
    _device: platform::Stream,
    done: Arc<AtomicBool>,
    amplitude: Arc<AtomicU32>,
    started: Instant,
    total: Duration,
}

impl Playback {
    pub fn start(pcm: Pcm) -> Result<Self> {
        let done = Arc::new(AtomicBool::new(false));
        let amplitude = Arc::new(AtomicU32::new(0));
        let total = Duration::from_secs_f64(
            pcm.samples.len() as f64 / f64::from(pcm.rate) / f64::from(pcm.channels),
        );
        let signal = done.clone();
        let volume = amplitude.clone();
        let rate = pcm.rate;
        let channels = pcm.channels;
        let mut source = RenderSource { pcm, cursor: 0.0 };
        let mut drained = false;
        let mut envelope = 0.0_f32;
        let device = platform::playback(rate, channels, move |output, rate, channels| {
            let count = source.render(output, rate, channels);
            let power = output.iter().map(|sample| sample * sample).sum::<f32>()
                / output.len().max(1) as f32;
            let loudness = power.sqrt();
            let smoothing = if loudness > envelope { 0.62 } else { 0.31 };
            envelope += (loudness - envelope) * smoothing;
            if count == 0 {
                envelope = 0.0;
            }
            volume.store(envelope.to_bits(), Ordering::Relaxed);
            if count == 0 {
                if drained {
                    signal.store(true, Ordering::Release);
                }
                drained = true;
            }
        })?;
        Ok(Self {
            _device: device,
            done,
            amplitude,
            started: Instant::now(),
            total,
        })
    }

    pub fn done(&self) -> bool {
        self.done.load(Ordering::Acquire)
            || self.started.elapsed() > self.total + Duration::from_secs(3)
    }
    pub fn amplitude(&self) -> f32 {
        f32::from_bits(self.amplitude.load(Ordering::Relaxed))
    }
}

struct RenderSource {
    pcm: Pcm,
    cursor: f64,
}

impl RenderSource {
    fn render(&mut self, target: &mut [f32], rate: u32, channels: u16) -> usize {
        let input_channels = usize::from(self.pcm.channels);
        let output_channels = usize::from(channels);
        let total_frames = self.pcm.samples.len() / input_channels;
        let step = f64::from(self.pcm.rate) / f64::from(rate);
        let mut frames = 0;
        target.fill(0.0);

        for frame in target.chunks_exact_mut(output_channels) {
            let index = self.cursor as usize;
            if index >= total_frames {
                break;
            }
            let next = (index + 1).min(total_frames - 1);
            let fraction = self.cursor.fract() as f32;
            let first = &self.pcm.samples[index * input_channels..][..input_channels];
            let second = &self.pcm.samples[next * input_channels..][..input_channels];
            if output_channels == 1 && input_channels > 1 {
                let average = |samples: &[f32]| samples.iter().sum::<f32>() / input_channels as f32;
                let start = average(first);
                frame[0] = start + (average(second) - start) * fraction;
            } else {
                for (channel, sample) in frame.iter_mut().enumerate() {
                    let channel = channel.min(input_channels - 1);
                    *sample = first[channel] + (second[channel] - first[channel]) * fraction;
                }
            }
            self.cursor += step;
            frames += 1;
        }
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renders_mono_at_the_native_stereo_device_rate() {
        let mut player = RenderSource {
            pcm: Pcm {
                samples: vec![0.0, 0.5, 1.0],
                rate: 24000,
                channels: 1,
            },
            cursor: 0.0,
        };
        let mut output = [0.0; 16];
        assert_eq!(player.render(&mut output, 48000, 2), 6);
        assert_eq!(
            output,
            [
                0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0
            ]
        );
        assert_eq!(player.render(&mut output, 48000, 2), 0);
        assert_eq!(output, [0.0; 16]);
    }

    #[test]
    fn mic_wav_round_trip() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("mic.wav");
        write_wav(&path, &[i16::MIN, 0, 16384]).unwrap();
        let pcm = read_wav(&path).unwrap();
        assert_eq!(pcm.channels, 1);
        assert_eq!(pcm.rate, 16000);
        assert_eq!(pcm.samples, [-1.0, 0.0, 0.5]);
    }
}
