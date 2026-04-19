//! Cue system module.
//!
//! The `Cue` trait and all cue implementations live here.  To add a new cue
//! type, implement [`traits::Cue`] + [`traits::CueFactory`], add it to
//! [`types::CueType`], and register the factory in
//! [`registry::CueRegistry`] at startup.

pub mod audio_cue;
pub mod context;
pub mod image_cue;
pub mod memo_cue;
pub mod registry;
pub mod stop_cue;
pub mod traits;
pub mod types;
pub mod video_cue;

pub use registry::CueRegistry;
pub use traits::Cue;
pub use types::{CueId, CueState, CueType};
