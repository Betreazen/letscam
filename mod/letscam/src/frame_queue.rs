//! Frames on their way to ffmpeg's stdin: a small bounded channel and a writer thread, with
//! the time the render loop waited for room in the channel and the time the writer was
//! blocked writing to the pipe.

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub(crate) struct FrameQueue {
    tx: Option<SyncSender<Vec<u8>>>,
    writer: Option<JoinHandle<Result<(), String>>>,
    queue_wait: Duration,
    write_nanos: Arc<AtomicU64>,
}

fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

impl FrameQueue {
    /// Starts the writer thread; up to `depth` frames wait in the channel.
    pub(crate) fn start<W: Write + Send + 'static>(
        mut sink: W,
        depth: usize,
    ) -> Result<FrameQueue, String> {
        let (tx, rx) = sync_channel::<Vec<u8>>(depth);
        let write_nanos = Arc::new(AtomicU64::new(0));
        let spent = write_nanos.clone();
        let writer = std::thread::Builder::new()
            .name("letscam-ffmpeg-writer".into())
            .spawn(move || {
                for frame in rx {
                    let t = Instant::now();
                    let written = sink.write_all(&frame);
                    spent.fetch_add(nanos(t.elapsed()), Ordering::Relaxed);
                    written.map_err(|e| format!("запись кадра в ffmpeg: {e}"))?;
                }
                sink.flush().map_err(|e| format!("ffmpeg stdin: {e}"))
            })
            .map_err(|e| format!("поток записи кадров: {e}"))?;
        Ok(FrameQueue {
            tx: Some(tx),
            writer: Some(writer),
            queue_wait: Duration::ZERO,
            write_nanos,
        })
    }

    /// Hands a frame to the writer (blocks while the channel is full); `false` when the
    /// writer has stopped.
    pub(crate) fn push(&mut self, frame: Vec<u8>) -> bool {
        let alive = self.writer.as_ref().is_some_and(|w| !w.is_finished());
        let Some(tx) = self.tx.as_ref().filter(|_| alive) else {
            return false;
        };
        let t = Instant::now();
        let sent = tx.send(frame).is_ok();
        self.queue_wait += t.elapsed();
        sent
    }

    /// No more frames: the writer drains the channel, flushes and ends.
    pub(crate) fn hang_up(&mut self) {
        drop(self.tx.take());
    }

    /// Hangs up and waits for the writer; its write error, if any.
    pub(crate) fn join(&mut self) -> Result<(), String> {
        self.hang_up();
        self.writer.take().map_or(Ok(()), |w| {
            w.join().unwrap_or(Err("поток записи кадров упал".into()))
        })
    }

    /// Total time [`FrameQueue::push`] waited for room in the channel.
    pub(crate) fn queue_wait(&self) -> Duration {
        self.queue_wait
    }

    /// Total time the writer thread spent inside writes to the sink.
    pub(crate) fn pipe_write(&self) -> Duration {
        Duration::from_nanos(self.write_nanos.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sink that takes `delay` per write and keeps what it got.
    struct SlowSink {
        delay: Duration,
        got: Arc<parking_lot::Mutex<Vec<u8>>>,
    }

    impl Write for SlowSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            std::thread::sleep(self.delay);
            self.got.lock().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn slow_sink_shows_as_pipe_write_and_queue_wait() {
        let got = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let sink = SlowSink {
            delay: Duration::from_millis(5),
            got: got.clone(),
        };
        let mut q = FrameQueue::start(sink, 2).expect("start");
        for i in 0..12u8 {
            assert!(q.push(vec![i; 3]));
        }
        q.join().expect("join");
        let expected: Vec<u8> = (0..12u8).flat_map(|i| [i; 3]).collect();
        assert_eq!(*got.lock(), expected, "frames arrive whole and in order");
        assert!(
            q.pipe_write() >= Duration::from_millis(60),
            "{:?}",
            q.pipe_write()
        );
        // 12 frames, room for 2 queued + 1 in the writer: the loop waited for ~9 writes.
        assert!(
            q.queue_wait() >= Duration::from_millis(30),
            "{:?}",
            q.queue_wait()
        );
        assert!(q.queue_wait() <= q.pipe_write() + Duration::from_millis(20));
    }

    #[test]
    fn fast_sink_barely_waits_and_push_after_hang_up_fails() {
        let mut q = FrameQueue::start(std::io::sink(), 4).expect("start");
        for _ in 0..3 {
            assert!(q.push(vec![0; 16]));
        }
        assert!(
            q.queue_wait() < Duration::from_millis(50),
            "{:?}",
            q.queue_wait()
        );
        q.hang_up();
        assert!(!q.push(vec![0; 16]));
        q.join().expect("join");
    }

    #[test]
    fn failing_sink_stops_the_queue_with_its_error() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("pipe closed"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut q = FrameQueue::start(Broken, 1).expect("start");
        let mut pushed = 0;
        while q.push(vec![1; 8]) && pushed < 1000 {
            pushed += 1;
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(pushed < 1000, "a dead writer must refuse frames");
        let err = q.join().expect_err("write error");
        assert!(err.contains("pipe closed"), "{err}");
    }
}
