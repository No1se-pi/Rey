use anyhow::{Context, Result, bail};
use cpal::{
    Device, FromSample, Sample, SampleFormat, SizedSample, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};

pub use cpal::Stream;

pub fn capture(rate: u32, callback: impl FnMut(&[i16]) + Send + 'static) -> Result<Stream> {
    let device = cpal::default_host()
        .default_input_device()
        .context("Микрофон недоступен")?;
    let supported = device
        .default_input_config()
        .context("Нет формата записи микрофона")?;
    let format = supported.sample_format();
    let config = supported.config();
    let stream = match format {
        SampleFormat::F32 => capture_stream::<f32>(&device, &config, rate, callback),
        SampleFormat::I16 => capture_stream::<i16>(&device, &config, rate, callback),
        SampleFormat::I32 => capture_stream::<i32>(&device, &config, rate, callback),
        SampleFormat::U16 => capture_stream::<u16>(&device, &config, rate, callback),
        _ => bail!("Неподдерживаемый формат микрофона: {format}"),
    }?;
    stream.play().context("Не удалось начать запись")?;
    Ok(stream)
}

fn capture_stream<T: SizedSample>(
    device: &Device,
    config: &StreamConfig,
    target_rate: u32,
    mut callback: impl FnMut(&[i16]) + Send + 'static,
) -> Result<Stream>
where
    f32: FromSample<T>,
{
    let native_rate = config.sample_rate.0;
    let channels = usize::from(config.channels);
    let mut phase = 0;
    let mut buffer = Vec::new();
    device
        .build_input_stream(
            config,
            move |input: &[T], _| {
                buffer.clear();
                for frame in input.chunks_exact(channels) {
                    phase += target_rate;
                    if phase >= native_rate {
                        let mono = frame
                            .iter()
                            .map(|sample| sample.to_sample::<f32>())
                            .sum::<f32>()
                            / channels as f32;
                        while phase >= native_rate {
                            phase -= native_rate;
                            buffer.push(mono.clamp(-1.0, 1.0).to_sample::<i16>());
                        }
                    }
                }
                callback(&buffer);
            },
            |error| eprintln!("WASAPI capture: {error}"),
            None,
        )
        .context("Не удалось открыть микрофон")
}

pub fn playback(
    _rate: u32,
    _channels: u16,
    callback: impl FnMut(&mut [f32], u32, u16) + Send + 'static,
) -> Result<Stream> {
    let device = cpal::default_host()
        .default_output_device()
        .context("Устройство воспроизведения недоступно")?;
    let supported = device
        .default_output_config()
        .context("Нет формата воспроизведения устройства")?;
    let format = supported.sample_format();
    let config = supported.config();
    let stream = match format {
        SampleFormat::F32 => playback_stream::<f32>(&device, &config, callback),
        SampleFormat::I16 => playback_stream::<i16>(&device, &config, callback),
        SampleFormat::I32 => playback_stream::<i32>(&device, &config, callback),
        SampleFormat::U16 => playback_stream::<u16>(&device, &config, callback),
        _ => bail!("Неподдерживаемый формат воспроизведения: {format}"),
    }?;
    stream.play().context("Не удалось начать воспроизведение")?;
    Ok(stream)
}

fn playback_stream<T: SizedSample + FromSample<f32>>(
    device: &Device,
    config: &StreamConfig,
    mut callback: impl FnMut(&mut [f32], u32, u16) + Send + 'static,
) -> Result<Stream> {
    let rate = config.sample_rate.0;
    let channels = config.channels;
    let mut samples = Vec::new();
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| {
                samples.resize(output.len(), 0.0);
                callback(&mut samples, rate, channels);
                for (sample, value) in output.iter_mut().zip(&samples) {
                    *sample = T::from_sample(value.clamp(-1.0, 1.0));
                }
            },
            |error| eprintln!("WASAPI playback: {error}"),
            None,
        )
        .context("Не удалось открыть устройство воспроизведения")
}

pub fn stop(stream: &Stream) -> Result<()> {
    stream.pause().context("Не удалось остановить запись")
}
