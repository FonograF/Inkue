//! Audio and output engine modules.
//!
//! Contains the real-time audio pipeline and the unified output engine:
//! - [`audio_engine::AudioEngine`]: top-level audio coordinator
//! - [`output_engine::OutputEngine`]: unified libmpv output for video and image (Win32 window)
//! - [`device_manager::DeviceManager`]: OS device enumeration + Output Patches
//! - [`voice::Voice`]: a single playing audio stream
//! - [`ring_command`]: command/status types for lock-free RT communication
//! - [`mpv_sys`]: runtime-loaded libmpv FFI symbols

pub mod audio_engine;
pub mod device_manager;
pub mod mpv_sys;
pub mod output_engine;
pub mod ring_command;
pub mod voice;

pub use audio_engine::AudioEngine;
pub use output_engine::OutputEngine;
