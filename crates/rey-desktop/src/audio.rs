use anyhow::{Result, anyhow, ensure};
use miniaudio::{Backend, Context, Device, DeviceConfig, DeviceType, Format};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

const RATE: u32 = 16_000;
pub struct Capture {
    device: Device,
    samples: Arc<Mutex<Vec<i16>>>,
    started: Instant,
    pub max_seconds: u32,
}

fn device(config: &DeviceConfig) -> Result<Device> {
    #[cfg(target_os = "linux")]
    let backends = [Backend::PulseAudio, Backend::Alsa];
    #[cfg(target_os = "windows")]
    let backends = [Backend::Wasapi, Backend::DSound, Backend::WinMM];
    let context =
        Context::new(&backends, None).map_err(|e| anyhow!("Аудиосистема недоступна: {e:?}"))?;
    Device::new(Some(context), config).map_err(|e| anyhow!("Аудиоустройство недоступно: {e:?}"))
}

impl Capture {
    pub fn start(max_seconds: u32, max_bytes: u64) -> Result<Self> {
        let mut config = DeviceConfig::new(DeviceType::Capture);
        config.set_sample_rate(RATE);
        config.capture_mut().set_format(Format::S16);
        config.capture_mut().set_channels(1);
        let max_samples =
            ((max_bytes.saturating_sub(44) / 2) as usize).min(max_seconds as usize * RATE as usize);
        let max_seconds = max_seconds.min((max_samples / RATE as usize).max(1) as u32);
        let samples = Arc::new(Mutex::new(Vec::with_capacity(max_samples)));
        let buffer = samples.clone();
        config.set_data_callback(move |_, _, input| {
            let Ok(mut buffer) = buffer.try_lock() else {
                return;
            };
            let input = input.as_samples::<i16>();
            let count = input.len().min(max_samples.saturating_sub(buffer.len()));
            buffer.extend_from_slice(&input[..count]);
        });
        let device = device(&config)?;
        device
            .start()
            .map_err(|e| anyhow!("Не удалось начать запись: {e:?}"))?;
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
        self.device
            .stop()
            .map_err(|e| anyhow!("Не удалось остановить запись: {e:?}"))?;
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
    _device: Device,
    done: Arc<AtomicBool>,
    amplitude: Arc<AtomicU32>,
    started: Instant,
    total: Duration,
}

impl Playback {
    pub fn start(pcm: Pcm) -> Result<Self> {
        let mut config = DeviceConfig::new(DeviceType::Playback);
        config.set_sample_rate(pcm.rate);
        config.playback_mut().set_format(Format::F32);
        config.playback_mut().set_channels(u32::from(pcm.channels));
        let done = Arc::new(AtomicBool::new(false));
        let amplitude = Arc::new(AtomicU32::new(0));
        let total = Duration::from_secs_f64(
            pcm.samples.len() as f64 / f64::from(pcm.rate) / f64::from(pcm.channels),
        );
        let signal = done.clone();
        let volume = amplitude.clone();
        let samples: Arc<[f32]> = pcm.samples.into();
        let mut index = 0;
        let mut drained = false;
        let mut envelope = 0.0_f32;
        config.set_data_callback(move |_, output, _| {
            let output = output.as_samples_mut::<f32>();
            let count = output.len().min(samples.len().saturating_sub(index));
            output[..count].copy_from_slice(&samples[index..index + count]);
            output[count..].fill(0.0);
            index += count;
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
        });
        let device = device(&config)?;
        device
            .start()
            .map_err(|e| anyhow!("Не удалось начать воспроизведение: {e:?}"))?;
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

#[cfg(test)]
mod tests {
    use super::*;
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
