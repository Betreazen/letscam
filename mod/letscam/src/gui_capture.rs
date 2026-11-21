//! Background thread that halves GUI snapshots and writes them as PNG files.

use std::path::PathBuf;
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::thread::JoinHandle;

use crate::pip::{downscale2, save_png};

/// Snapshots waiting for the encoder thread; more are dropped (each is a full frame).
const QUEUE: usize = 3;

pub(crate) struct GuiJob {
    pub path: PathBuf,
    /// Name recorded in the session (relative path).
    pub file: String,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[u8; 4]>,
    pub bgra: bool,
}

pub(crate) struct GuiWriter {
    tx: Option<SyncSender<GuiJob>>,
    thread: Option<JoinHandle<Vec<String>>>,
}

fn write_job(job: GuiJob) -> Result<(), String> {
    let img = downscale2(job.width, job.height, &job.pixels, job.bgra)
        .ok_or_else(|| format!("кадр интерфейса {}x{} не подходит", job.width, job.height))?;
    save_png(&job.path, &img)
}

impl GuiWriter {
    pub fn start() -> Result<GuiWriter, String> {
        let (tx, rx) = sync_channel::<GuiJob>(QUEUE);
        let thread = std::thread::Builder::new()
            .name("letscam-gui-png".into())
            .spawn(move || {
                let mut failed = Vec::new();
                for job in rx {
                    let file = job.file.clone();
                    if let Err(e) = write_job(job) {
                        log::warn!("LetsCam: кадр интерфейса {file} не сохранён: {e}");
                        failed.push(file);
                    }
                }
                failed
            })
            .map_err(|e| format!("поток кадров интерфейса: {e}"))?;
        Ok(GuiWriter {
            tx: Some(tx),
            thread: Some(thread),
        })
    }

    /// Queues a snapshot without blocking; false if the queue is full or the thread is gone.
    pub fn submit(&self, job: GuiJob) -> bool {
        match self.tx.as_ref().map(|tx| tx.try_send(job)) {
            Some(Ok(())) => true,
            Some(Err(TrySendError::Full(j))) => {
                log::debug!(
                    "LetsCam: очередь кадров интерфейса полна, {} пропущен",
                    j.file
                );
                false
            }
            _ => false,
        }
    }

    /// Waits for the queue to drain; returns the files that could not be written.
    pub fn finish(mut self) -> Vec<String> {
        drop(self.tx.take());
        self.thread
            .take()
            .map(|t| t.join().unwrap_or_default())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempDir;

    #[test]
    fn writes_half_size_png_and_reports_failures() {
        let tmp = TempDir::new("gui");
        let w = GuiWriter::start().expect("start");
        let ok = GuiJob {
            path: tmp.path().join("1.png"),
            file: "gui/000001.png".into(),
            width: 8,
            height: 6,
            pixels: vec![[10, 20, 30, 255]; 48],
            bgra: false,
        };
        let bad = GuiJob {
            path: tmp.path().join("2.png"),
            file: "gui/000002.png".into(),
            width: 8,
            height: 6,
            pixels: vec![[0; 4]; 3],
            bgra: false,
        };
        assert!(w.submit(ok));
        assert!(w.submit(bad));
        assert_eq!(w.finish(), vec!["gui/000002.png".to_string()]);
        let img = crate::pip::load_png(&tmp.path().join("1.png")).expect("png");
        assert_eq!((img.width, img.height), (4, 3));
        assert_eq!(&img.rgba[..4], &[10, 20, 30, 255]);
    }
}
