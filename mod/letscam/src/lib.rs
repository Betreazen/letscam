//! LetsCam engine-independent core: settings, capture of a recording session (game audio
//! via WASAPI process loopback, microphone, GUI snapshots, player track), the automatic
//! "director" camera and the ffmpeg encoder with picture-in-picture.
//!
//! The engine glue (Horizon hooks) calls into this crate; nothing here knows about the game.

pub mod bridge;
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

/// LetsCam's two keys on Options > Controls, rebindable there; the game glue (modules/glue.rs)
/// reads the player's binding with `Game::mod_key(MOD_ID, …)` and compares it in its key hook.
pub const MOD_ID: &str = "letscam";
pub const KEY_TOGGLE: &str = "toggle";
pub const KEY_TOGGLE_DEFAULT: &str = "key.keyboard.f9";
pub const KEY_MENU: &str = "menu";
pub const KEY_MENU_DEFAULT: &str = "key.keyboard.f10";

// The mod API entry point, at the crate root as #[horizon_mod] requires. LetsCam reaches the
// game through its [[native.mixin]] rows and support modules (horizon-mod.toml); the API part
// is only its keys, which the old `mods` hook used to declare.
#[derive(Default)]
struct LetsCam;

#[mod_api::horizon_mod]
impl mod_api::Hooks for LetsCam {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        // The labels are translation keys; LetsCam ships no lang file, so Controls shows them
        // as written.
        r.register_key_mapping(
            mod_api::KeyRegistration::new(KEY_TOGGLE, KEY_TOGGLE_DEFAULT)
                .label("LetsCam: начать/остановить запись"),
        );
        r.register_key_mapping(
            mod_api::KeyRegistration::new(KEY_MENU, KEY_MENU_DEFAULT)
                .label("LetsCam: окно записей"),
        );
    }
}

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
