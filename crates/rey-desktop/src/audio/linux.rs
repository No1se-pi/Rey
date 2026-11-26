use anyhow::{Result, anyhow};
use miniaudio::{Backend, Context, Device, DeviceConfig, DeviceType, Format};

pub type Stream = Device;

fn device(config: &DeviceConfig) -> Result<Device> {
    for backend in [Backend::PulseAudio, Backend::Alsa] {
        if let Ok(context) = Context::new(&[backend], None)
            && let Ok(device) = Device::new(Some(context), config)
        {
            return Ok(device);
        }
    }
    Err(anyhow!("Аудиоустройство PulseAudio или ALSA недоступно"))
}

pub fn capture(rate: u32, mut callback: impl FnMut(&[i16]) + Send + 'static) -> Result<Stream> {
    let mut config = DeviceConfig::new(DeviceType::Capture);
    config.set_sample_rate(rate);
    config.capture_mut().set_format(Format::S16);
    config.capture_mut().set_channels(1);
    let mut stream = device(&config)?;
    stream.set_data_callback(move |_, _, input| callback(input.as_samples::<i16>()));
    stream
        .start()
        .map_err(|error| anyhow!("Не удалось начать запись: {error:?}"))?;
    Ok(stream)
}

pub fn playback(
    rate: u32,
    channels: u16,
    mut callback: impl FnMut(&mut [f32], u32, u16) + Send + 'static,
) -> Result<Stream> {
    let mut config = DeviceConfig::new(DeviceType::Playback);
    config.set_sample_rate(rate);
    config.playback_mut().set_format(Format::F32);
    config.playback_mut().set_channels(u32::from(channels));
    let mut stream = device(&config)?;
    stream.set_data_callback(move |_, output, _| {
        callback(output.as_samples_mut::<f32>(), rate, channels);
    });
    stream
        .start()
        .map_err(|error| anyhow!("Не удалось начать воспроизведение: {error:?}"))?;
    Ok(stream)
}

pub fn stop(stream: &Stream) -> Result<()> {
    stream
        .stop()
        .map_err(|error| anyhow!("Не удалось остановить запись: {error:?}"))
}
