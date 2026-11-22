//! Audio capture through cpal on its own thread (cpal streams are `!Send`, so the stream is
//! built and kept inside the thread). Used for the microphone and for the system-wide
//! loopback fallback of game audio. The device callback never blocks or allocates: it pushes
//! into preallocated lock-free queues; the thread drains them into a [`TimelineWav`].

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, InputCallbackInfo, Sample, SampleFormat, SizedSample};
use crossbeam_queue::ArrayQueue;

use crate::capture::{CaptureSummary, CaptureThread};
use crate::timeline::TimelineWav;
use crate::wav::f32_to_i16;

const QUEUE_SECONDS: usize = 4;
const MARKERS: usize = 2048;
const POLL: Duration = Duration::from_millis(10);
/// Silence is filled only this far behind "now", so late packets still land in order.
pub(crate) const IDLE_SLACK: Duration = Duration::from_millis(250);

pub(crate) enum Source {
    /// An input device whose name contains this text (case-insensitive); empty = default.
    Input(String),
    /// The default output device in loopback mode (WASAPI): everything the system plays.
    /// Windows only, the fallback when the game's own audio can't be captured.
    #[cfg_attr(not(windows), allow(dead_code))]
    OutputLoopback,
}

struct Shared {
    samples: ArrayQueue<i16>,
    markers: ArrayQueue<(Instant, u32)>,
    dropped: AtomicU64,
    in_channels: usize,
    out_channels: usize,
}

/// Mono downmix (average) or first-two-channels stereo of one interleaved frame.
pub(crate) fn convert_frame(frame: &[f32], out_channels: usize, out: &mut impl FnMut(i16)) {
    if frame.is_empty() {
        return;
    }
    if out_channels == 1 {
        let sum: f32 = frame.iter().sum();
        out(f32_to_i16(sum / frame.len() as f32));
    } else {
        out(f32_to_i16(frame[0]));
        out(f32_to_i16(frame[1.min(frame.len() - 1)]));
    }
}

fn on_data<T>(data: &[T], info: &InputCallbackInfo, shared: &Shared)
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let now = Instant::now();
    let ts = info.timestamp();
    let delay = ts.callback.duration_since(&ts.capture).unwrap_or_default();
    let at = now.checked_sub(delay).unwrap_or(now);
    let frames = data.len() / shared.in_channels;
    let needed = frames * shared.out_channels;
    let free = shared.samples.capacity() - shared.samples.len();
    if frames == 0 || free < needed || shared.markers.is_full() {
        shared.dropped.fetch_add(needed as u64, Ordering::Relaxed);
        return;
    }
    let mut frame = [0f32; 32];
    for chunk in data.chunks_exact(shared.in_channels) {
        let n = chunk.len().min(frame.len());
        for (dst, src) in frame.iter_mut().zip(chunk.iter()) {
            *dst = f32::from_sample(*src);
        }
        convert_frame(&frame[..n], shared.out_channels, &mut |s| {
            let _ = shared.samples.push(s);
        });
    }
    let _ = shared.markers.push((at, frames as u32));
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    shared: Arc<Shared>,
    error: Arc<parking_lot::Mutex<Option<String>>>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream::<T, _, _>(
            config,
            move |data: &[T], info: &InputCallbackInfo| on_data(data, info, &shared),
            move |e| {
                log::warn!("LetsCam: ошибка аудиопотока: {e}");
                *error.lock() = Some(e.to_string());
            },
            None,
        )
        .map_err(|e| format!("не удалось открыть аудиопоток: {e}"))
}

fn pick_device(source: &Source) -> Result<(cpal::Device, cpal::SupportedStreamConfig), String> {
    let host = cpal::default_host();
    match source {
        Source::OutputLoopback => {
            let dev = host
                .default_output_device()
                .ok_or("нет устройства вывода звука")?;
            let cfg = dev
                .default_output_config()
                .map_err(|e| format!("формат устройства вывода: {e}"))?;
            Ok((dev, cfg))
        }
        Source::Input(name) => {
            let dev = if name.trim().is_empty() {
                host.default_input_device()
                    .ok_or("нет микрофона по умолчанию")?
            } else {
                let want = name.trim().to_lowercase();
                let mut names = Vec::new();
                let devices = host
                    .input_devices()
                    .map_err(|e| format!("список микрофонов: {e}"))?;
                let mut found = None;
                for d in devices {
                    let n = d.name().unwrap_or_default();
                    if found.is_none() && n.to_lowercase().contains(&want) {
                        found = Some(d);
                    } else {
                        names.push(n);
                    }
                }
                found.ok_or_else(|| {
                    format!("микрофон «{name}» не найден; есть: {}", names.join(", "))
                })?
            };
            let cfg = dev
                .default_input_config()
                .map_err(|e| format!("формат микрофона: {e}"))?;
            Ok((dev, cfg))
        }
    }
}

struct Opened {
    _stream: cpal::Stream,
    shared: Arc<Shared>,
    error: Arc<parking_lot::Mutex<Option<String>>>,
    rate: u32,
    out_channels: u16,
    name: String,
}

fn open(source: &Source, mono: bool) -> Result<Opened, String> {
    let (device, supported) = pick_device(source)?;
    let name = device.name().unwrap_or_else(|_| "?".into());
    let rate = supported.sample_rate().0;
    let in_channels = usize::from(supported.channels().max(1));
    let out_channels = if mono { 1 } else { in_channels.min(2) };
    let shared = Arc::new(Shared {
        samples: ArrayQueue::new((rate as usize).max(8000) * out_channels * QUEUE_SECONDS),
        markers: ArrayQueue::new(MARKERS),
        dropped: AtomicU64::new(0),
        in_channels,
        out_channels,
    });
    let error = Arc::new(parking_lot::Mutex::new(None));
    let config = supported.config();
    let (s, e) = (shared.clone(), error.clone());
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, s, e)?,
        SampleFormat::I16 => build::<i16>(&device, &config, s, e)?,
        SampleFormat::U16 => build::<u16>(&device, &config, s, e)?,
        SampleFormat::I32 => build::<i32>(&device, &config, s, e)?,
        SampleFormat::F64 => build::<f64>(&device, &config, s, e)?,
        other => return Err(format!("формат сэмплов {other} не поддерживается")),
    };
    stream
        .play()
        .map_err(|e| format!("не удалось запустить аудиопоток: {e}"))?;
    Ok(Opened {
        _stream: stream,
        shared,
        error,
        rate,
        out_channels: out_channels as u16,
        name,
    })
}

fn drain(shared: &Shared, wav: &mut TimelineWav, buf: &mut Vec<i16>) -> Result<(), String> {
    while let Some((at, frames)) = shared.markers.pop() {
        buf.clear();
        for _ in 0..frames as usize * shared.out_channels {
            if let Some(s) = shared.samples.pop() {
                buf.push(s);
            }
        }
        wav.write_at(at, buf)?;
    }
    Ok(())
}

fn run(opened: Opened, mut wav: TimelineWav, stop: &AtomicBool) -> Result<CaptureSummary, String> {
    let mut buf = Vec::new();
    let mut failure = None;
    while !stop.load(Ordering::Acquire) {
        std::thread::sleep(POLL);
        let now = Instant::now();
        let step = drain(&opened.shared, &mut wav, &mut buf)
            .and_then(|()| wav.pad_until(now.checked_sub(IDLE_SLACK).unwrap_or(now)));
        if let Err(e) = step {
            failure = Some(e);
            break;
        }
    }
    let end = Instant::now();
    let tail = drain(&opened.shared, &mut wav, &mut buf).and_then(|()| wav.pad_until(end));
    let error = failure
        .or(tail.err())
        .or_else(|| opened.error.lock().take());
    let (origin, rate, channels) = (wav.origin(), wav.rate(), wav.channels());
    let frames = wav.finalize()?;
    Ok(CaptureSummary {
        origin,
        sample_rate: rate,
        channels,
        frames,
        dropped_samples: opened.shared.dropped.load(Ordering::Relaxed),
        error,
    })
}

/// Starts capturing `source` into `path`. Returns once the stream runs (or with its error).
pub(crate) fn start(
    source: Source,
    path: PathBuf,
    mono: bool,
) -> Result<(CaptureThread, String), String> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_t = stop.clone();
    let (tx, rx) = mpsc::sync_channel::<Result<String, String>>(1);
    let handle = std::thread::Builder::new()
        .name("letscam-cpal-capture".into())
        .spawn(move || {
            let origin = Instant::now();
            let opened = match open(&source, mono) {
                Ok(o) => o,
                Err(e) => {
                    let _ = tx.send(Err(e.clone()));
                    return Err(e);
                }
            };
            let wav = match TimelineWav::create(&path, opened.out_channels, opened.rate, origin) {
                Ok(w) => w,
                Err(e) => {
                    let _ = tx.send(Err(e.clone()));
                    return Err(e);
                }
            };
            let _ = tx.send(Ok(opened.name.clone()));
            run(opened, wav, &stop_t)
        })
        .map_err(|e| format!("не удалось запустить поток записи звука: {e}"))?;
    match rx.recv() {
        Ok(Ok(name)) => Ok((CaptureThread::new(stop, handle), name)),
        Ok(Err(e)) => {
            let _ = handle.join();
            Err(e)
        }
        Err(_) => {
            let _ = handle.join();
            Err("поток записи звука завершился при запуске".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(frame: &[f32], ch: usize) -> Vec<i16> {
        let mut v = Vec::new();
        convert_frame(frame, ch, &mut |s| v.push(s));
        v
    }

    #[test]
    fn mono_downmix_averages() {
        assert_eq!(collect(&[0.5, -0.5], 1), vec![0]);
        assert_eq!(collect(&[1.0, 1.0, 1.0], 1), vec![32767]);
        assert_eq!(collect(&[0.5], 1), vec![16384]);
    }

    #[test]
    fn stereo_takes_first_two_channels() {
        assert_eq!(collect(&[1.0, -1.0, 0.0, 0.0], 2), vec![32767, -32767]);
        assert_eq!(collect(&[0.5], 2), vec![16384, 16384]);
        assert!(collect(&[], 2).is_empty());
    }

    #[test]
    fn unknown_input_device_is_an_error() {
        let r = start(
            Source::Input("no-such-device-letscam-xyz".into()),
            std::env::temp_dir().join("letscam-never.wav"),
            true,
        );
        assert!(r.is_err());
    }
}
