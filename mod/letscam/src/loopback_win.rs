//! WASAPI process loopback (Windows 10 2004+): captures only the sound of our process tree.
//! Runs on its own MTA thread; packets are placed on the timeline by their QPC time,
//! silent packets and gaps become zeros.

use std::mem::ManuallyDrop;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
    AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, ActivateAudioInterfaceAsync,
    IActivateAudioInterfaceAsyncOperation, IActivateAudioInterfaceCompletionHandler,
    IActivateAudioInterfaceCompletionHandler_Impl, IAudioCaptureClient, IAudioClient,
    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
    WAVE_FORMAT_PCM, WAVEFORMATEX,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{BLOB, COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::System::Variant::VT_BLOB;
use windows::core::{HRESULT, IUnknown, Interface, Ref, implement};

use crate::capture::{CaptureSummary, CaptureThread};
use crate::cpal_capture::IDLE_SLACK;
use crate::timeline::TimelineWav;

const RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
/// 200 ms engine buffer, in 100-ns units.
const BUFFER_HNS: i64 = 2_000_000;
const ACTIVATE_TIMEOUT: Duration = Duration::from_secs(5);
const WAIT_MS: u32 = 50;

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Activation {
    done: parking_lot::Mutex<Option<mpsc::SyncSender<()>>>,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for Activation_Impl {
    fn ActivateCompleted(
        &self,
        _op: Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        if let Some(tx) = self.done.lock().take() {
            let _ = tx.send(());
        }
        Ok(())
    }
}

fn win(context: &str) -> impl Fn(windows::core::Error) -> String + '_ {
    move |e| format!("{context}: {e}")
}

/// Current QPC time in 100-ns units (the unit of `IAudioCaptureClient::GetBuffer` positions).
fn qpc_100ns() -> Result<u64, String> {
    let (mut count, mut freq) = (0i64, 0i64);
    // SAFETY: both calls only write into the provided integers.
    unsafe {
        QueryPerformanceCounter(&mut count).map_err(win("QueryPerformanceCounter"))?;
        QueryPerformanceFrequency(&mut freq).map_err(win("QueryPerformanceFrequency"))?;
    }
    if freq <= 0 || count < 0 {
        return Err("QPC недоступен".into());
    }
    Ok((count as u128 * 10_000_000 / freq as u128) as u64)
}

fn activate() -> Result<IAudioClient, String> {
    let params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: std::process::id(),
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    // windows-rs drops PROPVARIANT with PropVariantClear, which would free our stack blob.
    let mut prop = ManuallyDrop::new(PROPVARIANT::default());
    prop.Anonymous.Anonymous = ManuallyDrop::new(PROPVARIANT_0_0 {
        vt: VT_BLOB,
        wReserved1: 0,
        wReserved2: 0,
        wReserved3: 0,
        Anonymous: PROPVARIANT_0_0_0 {
            blob: BLOB {
                cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                pBlobData: (&raw const params).cast_mut().cast(),
            },
        },
    });
    let (tx, rx) = mpsc::sync_channel(1);
    let handler: IActivateAudioInterfaceCompletionHandler = Activation {
        done: parking_lot::Mutex::new(Some(tx)),
    }
    .into();
    // SAFETY: `prop` points at `params`, both alive until the activation completes below
    // (we wait for the completion handler before leaving this function).
    let op = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&raw const *prop),
            &handler,
        )
    }
    .map_err(win("ActivateAudioInterfaceAsync"))?;
    rx.recv_timeout(ACTIVATE_TIMEOUT)
        .map_err(|_| "активация захвата звука не завершилась за 5 с".to_string())?;
    let mut hr = HRESULT(0);
    let mut unknown: Option<IUnknown> = None;
    // SAFETY: the operation has completed; out-pointers are valid locals.
    unsafe { op.GetActivateResult(&mut hr, &mut unknown) }.map_err(win("GetActivateResult"))?;
    hr.ok().map_err(win("активация захвата звука процесса"))?;
    unknown
        .ok_or("активация вернула пустой интерфейс")?
        .cast::<IAudioClient>()
        .map_err(win("IAudioClient"))
}

struct Client {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    event: HANDLE,
}

impl Drop for Client {
    fn drop(&mut self) {
        // SAFETY: the client was started by us; the event handle is owned by this struct.
        unsafe {
            let _ = self.client.Stop();
            let _ = CloseHandle(self.event);
        }
    }
}

fn open() -> Result<Client, String> {
    let client = activate()?;
    let block_align = CHANNELS * 2;
    let format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: CHANNELS,
        nSamplesPerSec: RATE,
        nAvgBytesPerSec: RATE * u32::from(block_align),
        nBlockAlign: block_align,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    let flags = AUDCLNT_STREAMFLAGS_LOOPBACK
        | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM;
    // SAFETY: plain COM calls with valid arguments; the event is closed by `Client::drop`.
    unsafe {
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                flags,
                BUFFER_HNS,
                0,
                &format,
                None,
            )
            .map_err(win("IAudioClient::Initialize"))?;
        let event = CreateEventW(None, false, false, None).map_err(win("CreateEventW"))?;
        let capture = client.GetService::<IAudioCaptureClient>();
        let c = Client {
            client,
            capture: match capture {
                Ok(c) => c,
                Err(e) => {
                    let _ = CloseHandle(event);
                    return Err(format!("IAudioCaptureClient: {e}"));
                }
            },
            event,
        };
        c.client
            .SetEventHandle(c.event)
            .map_err(win("SetEventHandle"))?;
        Ok(c)
    }
}

/// Reads all pending packets into the timeline.
fn drain(
    c: &Client,
    wav: &mut TimelineWav,
    clock: (Instant, u64),
    buf: &mut Vec<i16>,
) -> Result<(), String> {
    let (origin, origin_qpc) = clock;
    loop {
        // SAFETY: GetBuffer/ReleaseBuffer are paired; the data pointer is read only before release.
        unsafe {
            let n = c
                .capture
                .GetNextPacketSize()
                .map_err(win("GetNextPacketSize"))?;
            if n == 0 {
                return Ok(());
            }
            let mut data = std::ptr::null_mut();
            let (mut frames, mut flags, mut qpc) = (0u32, 0u32, 0u64);
            c.capture
                .GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc))
                .map_err(win("GetBuffer"))?;
            let samples = frames as usize * usize::from(CHANNELS);
            buf.clear();
            if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null() {
                buf.resize(samples, 0);
            } else {
                let bytes = std::slice::from_raw_parts(data, samples * 2);
                buf.extend(
                    bytes
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|b| i16::from_le_bytes(*b)),
                );
            }
            c.capture
                .ReleaseBuffer(frames)
                .map_err(win("ReleaseBuffer"))?;
            let at =
                origin + Duration::from_nanos(qpc.saturating_sub(origin_qpc).saturating_mul(100));
            wav.write_at(at, buf)?;
        }
    }
}

fn run(
    c: &Client,
    wav: &mut TimelineWav,
    clock: (Instant, u64),
    stop: &AtomicBool,
) -> Result<(), String> {
    let mut buf = Vec::new();
    while !stop.load(Ordering::Acquire) {
        // SAFETY: waiting on our own event handle.
        unsafe {
            let _ = WaitForSingleObject(c.event, WAIT_MS);
        }
        drain(c, wav, clock, &mut buf)?;
        let now = Instant::now();
        wav.pad_until(now.checked_sub(IDLE_SLACK).unwrap_or(now))?;
    }
    drain(c, wav, clock, &mut buf)
}

fn thread_main(
    path: PathBuf,
    stop: &AtomicBool,
    ready: mpsc::SyncSender<Result<(), String>>,
) -> Result<CaptureSummary, String> {
    let setup = || -> Result<(Client, TimelineWav, (Instant, u64)), String> {
        let c = open()?;
        let origin_qpc = qpc_100ns()?;
        let origin = Instant::now();
        let wav = TimelineWav::create(&path, CHANNELS, RATE, origin)?;
        // SAFETY: plain COM call on an initialized client.
        unsafe { c.client.Start() }.map_err(win("IAudioClient::Start"))?;
        Ok((c, wav, (origin, origin_qpc)))
    };
    let (client, mut wav, clock) = match setup() {
        Ok(v) => v,
        Err(e) => {
            let _ = ready.send(Err(e.clone()));
            return Err(e);
        }
    };
    let _ = ready.send(Ok(()));
    let result = run(&client, &mut wav, clock, stop);
    drop(client);
    let end = Instant::now();
    let error = result.and_then(|()| wav.pad_until(end)).err();
    if let Some(e) = &error {
        log::warn!("LetsCam: запись звука игры прервалась: {e}");
    }
    let frames = wav.finalize()?;
    Ok(CaptureSummary {
        origin: clock.0,
        sample_rate: RATE,
        channels: CHANNELS,
        frames,
        dropped_samples: 0,
        error,
    })
}

/// Starts process loopback capture into `path`; returns after the stream started or failed.
pub(crate) fn start(path: PathBuf) -> Result<CaptureThread, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_t = stop.clone();
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = std::thread::Builder::new()
        .name("letscam-game-audio".into())
        .spawn(move || {
            // SAFETY: COM is initialized for this thread and uninitialized at its end.
            let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if let Err(e) = com.ok() {
                let msg = format!("CoInitializeEx: {e}");
                let _ = tx.send(Err(msg.clone()));
                return Err(msg);
            }
            let r = thread_main(path, &stop_t, tx);
            // SAFETY: paired with the successful CoInitializeEx above.
            unsafe { CoUninitialize() };
            r
        })
        .map_err(|e| format!("не удалось запустить поток звука игры: {e}"))?;
    match rx.recv() {
        Ok(Ok(())) => Ok(CaptureThread::new(stop, handle)),
        Ok(Err(e)) => {
            let _ = handle.join();
            Err(e)
        }
        Err(_) => {
            let _ = handle.join();
            Err("поток звука игры завершился при запуске".into())
        }
    }
}
