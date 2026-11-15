//! LetsCam engine-independent core: settings, capture of a recording session (game audio
//! via WASAPI process loopback, microphone, GUI snapshots, player track), the automatic
//! "director" camera and the ffmpeg encoder with picture-in-picture.
//!
//! The engine glue (Horizon hooks) calls into this crate; nothing here knows about the game.

pub mod capture;
pub mod config;
mod config_template;
mod cpal_capture;
pub mod director;
pub mod encode;
mod encoder;
mod frame_queue;
mod gui_capture;
pub mod loopback;
#[cfg(windows)]
mod loopback_win;
pub mod mic;
pub mod mob_labels;
pub mod pip;
pub mod recording;
pub mod render;
pub mod render_options;
pub mod render_stats;
pub mod session;
pub mod sheet;
mod timeline;
pub mod track;
pub mod wav;

#[cfg(test)]
mod encode_bench;
#[cfg(test)]
mod encode_tests;
#[cfg(test)]
mod encoder_gpu_tests;
#[cfg(test)]
mod encoder_tests;
#[cfg(test)]
mod render_slow_tests;
#[cfg(test)]
mod render_tests;
#[cfg(test)]
mod test_util;

pub use capture::CaptureSummary;
pub use config::{Codec, Config, Container};
pub use director::{
    CameraPose, Director, DirectorConfig, Focus, OpenWorld, ShotPreset, WorldProbe,
};
pub use encode::{AudioInput, ffmpeg_args, ffmpeg_args_with_duration};
pub use encoder::{Encoder, probe_hardware};
pub use loopback::GameAudioCapture;
pub use mic::MicCapture;
pub use pip::{Image, compose, load_png};
pub use recording::Recording;
pub use render::RenderJob;
pub use render_options::RenderOptions;
pub use render_stats::{LoopStats, RenderStats};
pub use session::{AudioTrack, GuiEvent, GuiFrame, Session};
pub use track::TrackSample;
