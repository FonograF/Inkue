//! Audio and video engine modules.
//!
//! Contains the real-time audio pipeline and the video playback engine:
//! - [`audio_engine::AudioEngine`]: top-level audio coordinator
//! - [`video_engine::VideoEngine`]: libmpv-based video output (Win32 window)
//! - [`device_manager::DeviceManager`]: OS device enumeration + Output Patches
//! - [`voice::Voice`]: a single playing audio stream
//! - [`ring_command`]: command/status types for lock-free RT communication
//! - [`mpv_sys`]: runtime-loaded libmpv FFI symbols

pub mod audio_engine;
pub mod device_manager;
pub mod mpv_sys;
pub mod ring_command;
pub mod video_engine;
pub mod voice;

pub use audio_engine::AudioEngine;
pub use video_engine::VideoEngine;
