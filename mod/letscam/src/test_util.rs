//! Helpers shared by unit tests: unique temp dirs and ffmpeg tool lookup.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A fresh, empty directory under the system temp dir. Removed by [`TempDir`]'s drop.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "letscam-test-{tag}-{}-{nanos}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }

    pub fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Path to an ffmpeg tool if it can be run, else `None` (integration tests then skip).
pub fn tool(name: &str) -> Option<String> {
    let ok = std::process::Command::new(name)
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        Some(name.to_string())
    } else {
        println!("NOTE: {name} not found on PATH, integration check skipped");
        None
    }
}

/// Runs ffprobe and returns its JSON output for streams and format.
pub fn ffprobe_json(path: &std::path::Path) -> Option<serde_json::Value> {
    let probe = tool("ffprobe")?;
    let out = std::process::Command::new(probe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_streams",
            "-show_format",
            "-count_frames",
        ])
        .arg(path)
        .output()
        .ok()?;
    assert!(
        out.status.success(),
        "ffprobe failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).ok()
}
