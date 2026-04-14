//! Minimal libmpv FFI via [`libloading`] — no link-time import library needed.
//!
//! All mpv symbols are resolved at runtime from `libmpv-2.dll`.
//! The build script (`build.rs`) copies the DLL next to the compiled binary.
//!
//! Only the subset of the mpv API that WinCue actually uses is exposed here.

#![allow(dead_code)]

use std::ffi::{c_char, c_void};

use anyhow::{anyhow, Result};
use libloading::{Library, Symbol};

// ---------------------------------------------------------------------------
// mpv_format constants
// ---------------------------------------------------------------------------

pub const MPV_FORMAT_NONE: i32 = 0;
pub const MPV_FORMAT_STRING: i32 = 1;
pub const MPV_FORMAT_OSD_STRING: i32 = 2;
pub const MPV_FORMAT_FLAG: i32 = 3;
pub const MPV_FORMAT_INT64: i32 = 4;
pub const MPV_FORMAT_DOUBLE: i32 = 5;
pub const MPV_FORMAT_NODE: i32 = 6;

// ---------------------------------------------------------------------------
// mpv_event_id constants
// ---------------------------------------------------------------------------

pub const MPV_EVENT_NONE: u32 = 0;
pub const MPV_EVENT_SHUTDOWN: u32 = 1;
pub const MPV_EVENT_LOG_MESSAGE: u32 = 2;
pub const MPV_EVENT_START_FILE: u32 = 6;
pub const MPV_EVENT_END_FILE: u32 = 7;
pub const MPV_EVENT_FILE_LOADED: u32 = 8;
pub const MPV_EVENT_PROPERTY_CHANGE: u32 = 22;

// ---------------------------------------------------------------------------
// mpv_end_file_reason constants
// ---------------------------------------------------------------------------

pub const MPV_END_FILE_REASON_EOF: i32 = 0;
pub const MPV_END_FILE_REASON_STOP: i32 = 2;
pub const MPV_END_FILE_REASON_QUIT: i32 = 3;
pub const MPV_END_FILE_REASON_ERROR: i32 = 4;

// ---------------------------------------------------------------------------
// C structs matching mpv/client.h
// ---------------------------------------------------------------------------

/// Matches `mpv_event_log_message` from `mpv/client.h`.
#[repr(C)]
pub struct MpvEventLogMessage {
    /// Log domain prefix (e.g. `"vd"`, `"vo"`, `"file"`).
    pub prefix: *const c_char,
    /// Log level name (e.g. `"warn"`, `"error"`).
    pub level: *const c_char,
    /// The actual log message text (UTF-8, newline-terminated).
    pub text: *const c_char,
}

/// Matches `mpv_event` from `mpv/client.h`.
#[repr(C)]
pub struct MpvEvent {
    /// Which event occurred (one of the `MPV_EVENT_*` constants).
    pub event_id: u32,
    /// Error code for events that can fail; 0 = success.
    pub error: i32,
    /// Opaque reply userdata passed back from async requests.
    pub reply_userdata: u64,
    /// Event-specific data pointer (e.g. `*mut MpvEventEndFile`), or null.
    pub data: *mut c_void,
}

/// Matches `mpv_event_end_file` from `mpv/client.h`.
#[repr(C)]
pub struct MpvEventEndFile {
    /// One of the `MPV_END_FILE_REASON_*` constants.
    pub reason: i32,
    /// Non-zero mpv error code when `reason == MPV_END_FILE_REASON_ERROR`.
    pub error: i32,
}

// ---------------------------------------------------------------------------
// MpvLib — runtime-loaded function table
// ---------------------------------------------------------------------------

/// Runtime-loaded handle to `libmpv-2.dll` with all required function pointers.
///
/// The `_lib` field keeps the [`Library`] alive so function pointers remain
/// valid.  It **must** be declared last so it is dropped after the (trivially
/// copy) fn-pointer fields.
pub struct MpvLib {
    pub mpv_create:               unsafe extern "C" fn() -> *mut c_void,
    pub mpv_initialize:           unsafe extern "C" fn(*mut c_void) -> i32,
    pub mpv_terminate_destroy:    unsafe extern "C" fn(*mut c_void),
    pub mpv_set_option_string:    unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> i32,
    pub mpv_set_option:           unsafe extern "C" fn(*mut c_void, *const c_char, i32, *mut c_void) -> i32,
    pub mpv_set_property_string:  unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> i32,
    pub mpv_set_property:         unsafe extern "C" fn(*mut c_void, *const c_char, i32, *mut c_void) -> i32,
    pub mpv_get_property:         unsafe extern "C" fn(*mut c_void, *const c_char, i32, *mut c_void) -> i32,
    pub mpv_command:              unsafe extern "C" fn(*mut c_void, *const *const c_char) -> i32,
    pub mpv_command_string:       unsafe extern "C" fn(*mut c_void, *const c_char) -> i32,
    pub mpv_wait_event:           unsafe extern "C" fn(*mut c_void, f64) -> *mut MpvEvent,
    pub mpv_wakeup:               unsafe extern "C" fn(*mut c_void),
    pub mpv_error_string:         unsafe extern "C" fn(i32) -> *const c_char,
    pub mpv_request_log_messages: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32,
    /// Free a pointer returned by mpv (e.g. strings from `mpv_get_property` with
    /// `MPV_FORMAT_STRING`).
    pub mpv_free: unsafe extern "C" fn(*mut c_void),
    // IMPORTANT: `_lib` is last — drops after all fn-pointer fields.
    _lib: Library,
}

// SAFETY: mpv's public API is internally synchronized for all operations
// except `mpv_wait_event`, which we call from exactly one dedicated thread.
unsafe impl Send for MpvLib {}
unsafe impl Sync for MpvLib {}

impl MpvLib {
    /// Load `libmpv-2.dll` from the executable's directory and resolve all symbols.
    ///
    /// Returns an error if the DLL is missing or any symbol cannot be found.
    pub fn load() -> Result<Self> {
        // SAFETY: loading an external DLL is inherently unsafe.
        let lib = unsafe { Library::new("libmpv-2.dll") }
            .map_err(|e| anyhow!("Failed to load libmpv-2.dll — ensure it is placed next to the executable: {e}"))?;

        // Extract a raw fn pointer from the library.  The Symbol borrows `lib`
        // but the inner fn pointer is Copy and does not carry a lifetime.
        // Each `{{...}}` block drops the Symbol before the next borrow begins.
        macro_rules! sym {
            ($name:literal : $ty:ty) => {{
                let s: Symbol<$ty> = unsafe { lib.get(concat!($name, "\0").as_bytes()) }
                    .map_err(|e| anyhow!("libmpv: symbol '{}' not found: {}", $name, e))?;
                *s
            }};
        }

        Ok(Self {
            mpv_create:               sym!("mpv_create":               unsafe extern "C" fn() -> *mut c_void),
            mpv_initialize:           sym!("mpv_initialize":           unsafe extern "C" fn(*mut c_void) -> i32),
            mpv_terminate_destroy:    sym!("mpv_terminate_destroy":    unsafe extern "C" fn(*mut c_void)),
            mpv_set_option_string:    sym!("mpv_set_option_string":    unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> i32),
            mpv_set_option:           sym!("mpv_set_option":           unsafe extern "C" fn(*mut c_void, *const c_char, i32, *mut c_void) -> i32),
            mpv_set_property_string:  sym!("mpv_set_property_string":  unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> i32),
            mpv_set_property:         sym!("mpv_set_property":         unsafe extern "C" fn(*mut c_void, *const c_char, i32, *mut c_void) -> i32),
            mpv_get_property:         sym!("mpv_get_property":         unsafe extern "C" fn(*mut c_void, *const c_char, i32, *mut c_void) -> i32),
            mpv_command:              sym!("mpv_command":              unsafe extern "C" fn(*mut c_void, *const *const c_char) -> i32),
            mpv_command_string:       sym!("mpv_command_string":       unsafe extern "C" fn(*mut c_void, *const c_char) -> i32),
            mpv_wait_event:           sym!("mpv_wait_event":           unsafe extern "C" fn(*mut c_void, f64) -> *mut MpvEvent),
            mpv_wakeup:               sym!("mpv_wakeup":               unsafe extern "C" fn(*mut c_void)),
            mpv_error_string:         sym!("mpv_error_string":         unsafe extern "C" fn(i32) -> *const c_char),
            mpv_request_log_messages: sym!("mpv_request_log_messages": unsafe extern "C" fn(*mut c_void, *const c_char) -> i32),
            mpv_free:                 sym!("mpv_free":                 unsafe extern "C" fn(*mut c_void)),
            _lib: lib,
        })
    }
}
