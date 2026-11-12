//! Runs ffmpeg: frames go to its stdin from a writer thread through a small bounded channel.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::config::{Codec, Config};
use crate::encode::{AudioInput, ffmpeg_args_with_duration};

const QUEUE_FRAMES: usize = 4;
const STDERR_LINES: usize = 40;
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

type Tail = Arc<parking_lot::Mutex<VecDeque<String>>>;

pub struct Encoder {
    child: Option<Child>,
    tx: Option<SyncSender<Vec<u8>>>,
    writer: Option<JoinHandle<Result<(), String>>>,
    stderr: Tail,
    stderr_thread: Option<JoinHandle<()>>,
    out: PathBuf,
    frame_bytes: usize,
    frames: u64,
}

/// A command for ffmpeg that never opens a console window over the game.
pub(crate) fn command(ffmpeg: &str) -> Command {
    let mut cmd = Command::new(ffmpeg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn tail_text(tail: &Tail) -> String {
    let lines = tail.lock();
    if lines.is_empty() {
        "(ffmpeg ничего не сообщил)".into()
    } else {
        lines.iter().cloned().collect::<Vec<_>>().join("\n")
    }
}

impl Encoder {
    pub fn start(
        cfg: &Config,
        out: &Path,
        src_w: u32,
        src_h: u32,
        src_bgra: bool,
        audio: &[AudioInput],
    ) -> Result<Encoder, String> {
        Encoder::start_with_duration(cfg, out, src_w, src_h, src_bgra, audio, None)
    }

    /// Like [`Encoder::start`]; `duration_s` cuts the audio tracks to the video length.
    pub fn start_with_duration(
        cfg: &Config,
        out: &Path,
        src_w: u32,
        src_h: u32,
        src_bgra: bool,
        audio: &[AudioInput],
        duration_s: Option<f64>,
    ) -> Result<Encoder, String> {
        if src_w == 0 || src_h == 0 {
            return Err(format!("неверный размер кадра {src_w}x{src_h}"));
        }
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let args = ffmpeg_args_with_duration(cfg, out, src_w, src_h, src_bgra, audio, duration_s);
        log::info!("LetsCam: {} {}", cfg.ffmpeg, args.join(" "));
        let mut child = command(&cfg.ffmpeg)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("не удалось запустить ffmpeg ({}): {e}", cfg.ffmpeg))?;
        let stderr: Tail = Arc::new(parking_lot::Mutex::new(VecDeque::new()));
        let stderr_thread = child.stderr.take().map(|pipe| {
            let tail = stderr.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                    log::debug!("ffmpeg: {line}");
                    let mut t = tail.lock();
                    if t.len() == STDERR_LINES {
                        t.pop_front();
                    }
                    t.push_back(line);
                }
            })
        });
        let mut stdin = child.stdin.take().ok_or("нет stdin у ffmpeg")?;
        let (tx, rx) = sync_channel::<Vec<u8>>(QUEUE_FRAMES);
        let writer = std::thread::Builder::new()
            .name("letscam-ffmpeg-writer".into())
            .spawn(move || {
                for frame in rx {
                    stdin
                        .write_all(&frame)
                        .map_err(|e| format!("запись кадра в ffmpeg: {e}"))?;
                }
                stdin.flush().map_err(|e| format!("ffmpeg stdin: {e}"))
            })
            .map_err(|e| format!("поток записи кадров: {e}"))?;
        Ok(Encoder {
            child: Some(child),
            tx: Some(tx),
            writer: Some(writer),
            stderr,
            stderr_thread,
            out: out.to_path_buf(),
            frame_bytes: src_w as usize * src_h as usize * 4,
            frames: 0,
        })
    }

    fn died(&mut self) -> String {
        if let Some(child) = self.child.as_mut() {
            let _ = child.wait();
        }
        if let Some(t) = self.stderr_thread.take() {
            let _ = t.join();
        }
        format!("ffmpeg остановился:\n{}", tail_text(&self.stderr))
    }

    /// Queues one frame (blocks while ffmpeg is behind by more than a few frames).
    pub fn push_frame(&mut self, pixels: Vec<u8>) -> Result<(), String> {
        if pixels.len() != self.frame_bytes {
            return Err(format!(
                "кадр {} байт, ожидалось {}",
                pixels.len(),
                self.frame_bytes
            ));
        }
        let alive = self.writer.as_ref().is_some_and(|w| !w.is_finished());
        let sent = alive && self.tx.as_ref().is_some_and(|tx| tx.send(pixels).is_ok());
        if !sent {
            return Err(self.died());
        }
        self.frames += 1;
        Ok(())
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Closes stdin, waits for ffmpeg and checks its exit status.
    pub fn finish(mut self) -> Result<PathBuf, String> {
        drop(self.tx.take());
        let write = self.writer.take().map_or(Ok(()), |w| {
            w.join().unwrap_or(Err("поток записи кадров упал".into()))
        });
        let status = self
            .child
            .take()
            .ok_or("ffmpeg уже остановлен")?
            .wait()
            .map_err(|e| format!("ожидание ffmpeg: {e}"))?;
        if let Some(t) = self.stderr_thread.take() {
            let _ = t.join();
        }
        if !status.success() {
            return Err(format!(
                "ffmpeg завершился с ошибкой ({status}):\n{}",
                tail_text(&self.stderr)
            ));
        }
        write?;
        Ok(self.out.clone())
    }

    /// Kills ffmpeg and removes the partial file.
    pub fn cancel(mut self) {
        self.kill();
        if let Err(e) = std::fs::remove_file(&self.out)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("LetsCam: не удалось удалить {}: {e}", self.out.display());
        }
    }

    fn kill(&mut self) {
        drop(self.tx.take());
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
        if let Some(t) = self.stderr_thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        if self.child.is_some() {
            self.kill();
        }
    }
}

static PROBES: parking_lot::Mutex<Vec<((String, Codec), bool)>> =
    parking_lot::Mutex::new(Vec::new());

fn run_probe(ffmpeg: &str, codec: Codec) -> bool {
    let enc = match codec {
        Codec::H264 => "h264_nvenc",
        Codec::Hevc => "hevc_nvenc",
        Codec::Av1 => "av1_nvenc",
    };
    let spawned = command(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg("color=c=black:s=256x256:r=30")
        .args(["-frames:v", "1", "-c:v", enc, "-f", "null", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = spawned else {
        return false;
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if start.elapsed() < PROBE_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(30));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// Whether ffmpeg can encode `codec` on NVENC here (a 1-frame test encode, cached).
pub fn probe_hardware(ffmpeg: &str, codec: Codec) -> bool {
    let key = (ffmpeg.to_string(), codec);
    if let Some((_, ok)) = PROBES.lock().iter().find(|(k, _)| *k == key) {
        return *ok;
    }
    let ok = run_probe(ffmpeg, codec);
    log::info!(
        "LetsCam: NVENC {codec:?} через {ffmpeg}: {}",
        if ok { "есть" } else { "нет" }
    );
    PROBES.lock().push((key, ok));
    ok
}
