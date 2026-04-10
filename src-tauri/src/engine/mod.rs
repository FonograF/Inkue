//! Audio engine module.
//!
//! Contains the real-time audio pipeline:
//! - [`audio_engine::AudioEngine`]: top-level coordinator
//! - [`device_manager::DeviceManager`]: OS device enumeration + Output Patches
//! - [`voice::Voice`]: a single playing audio stream
//! - [`ring_command`]: command/status types for lock-free RT communication

pub mod audio_engine;
pub mod device_manager;
pub mod ring_command;
pub mod voice;

pub use audio_engine::AudioEngine;
