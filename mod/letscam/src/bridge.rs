// The hand-off of captured frames from the screenshot readback (the `save_screenshot` mixin,
// mixins/save_screenshot.rs) to the game glue (modules/glue.rs).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A screenshot request whose path starts with this is LetsCam's: its pixels are handed over,
/// not written as a PNG.
const MARKER: &str = "letscam-capture:";

/// One frame read back from the GPU, 4 bytes a pixel, rows packed: the renderer's own buffer,
/// handed over without a copy (a 1080p frame is 8 MB).
pub struct Captured {
    pub tag: String,
    pub size: [u32; 2],
    pub bgra: bool,
    pub pixels: Vec<u8>,
}

static CAPTURED: Mutex<Option<Captured>> = Mutex::new(None);

/// The path of a screenshot request that hands its frame to LetsCam under `tag`.
pub fn capture_path(tag: &str) -> PathBuf {
    PathBuf::from(format!("{MARKER}{tag}"))
}

/// The tag of a LetsCam request, None for any other screenshot.
pub fn capture_tag(path: &Path) -> Option<String> {
    path.to_str()?.strip_prefix(MARKER).map(str::to_owned)
}

pub fn put(tag: String, size: [u32; 2], bgra: bool, pixels: Vec<u8>) {
    let frame = Captured {
        tag,
        size,
        bgra,
        pixels,
    };
    match CAPTURED.lock() {
        Ok(mut slot) => *slot = Some(frame),
        Err(poisoned) => *poisoned.into_inner() = Some(frame),
    }
}

/// The last captured frame, once.
pub fn take() -> Option<Captured> {
    match CAPTURED.lock() {
        Ok(mut slot) => slot.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    }
}
