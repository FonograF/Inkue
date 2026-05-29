//! Persistent named-pipe PCM reader for mpv `ao=pcm` audio routing.
//!
//! mpv keeps its `ao=pcm` file descriptor open across `loadfile` calls — it
//! only reconnects after going idle (explicit `stop` command).
//!
//! The `discard` flag controls sample routing:
//! - `true`  (idle / pre-arm / image): bytes consumed but discarded so mpv never blocks.
//! - `false` (video actively playing): samples pushed into the AudioEngine ring buffer.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use ringbuf::traits::{Observer, Producer, Split};
use ringbuf::HeapRb;

use crate::engine::AudioEngine;

// ---------------------------------------------------------------------------
// Win32 pipe declarations
// ---------------------------------------------------------------------------

#[link(name = "kernel32")]
extern "system" {
    fn CreateNamedPipeW(
        lpname: *const u16,
        dwopenmode: u32,
        dwpipemode: u32,
        nmaxinstances: u32,
        noutbuffersize: u32,
        ninbuffersize: u32,
        ndefaulttimeout: u32,
        lpsecurityattributes: *const std::ffi::c_void,
    ) -> isize;
    fn ConnectNamedPipe(hnamedpipe: isize, lpoverlapped: *mut std::ffi::c_void) -> i32;
    fn DisconnectNamedPipe(hnamedpipe: isize) -> i32;
    fn ReadFile(
        hfile: isize,
        lpbuffer: *mut std::ffi::c_void,
        nnumberofbytestoread: u32,
        lpnumberofbytesread: *mut u32,
        lpoverlapped: *mut std::ffi::c_void,
    ) -> i32;
    fn CloseHandle(hobject: isize) -> i32;
}

const PIPE_ACCESS_INBOUND: u32   = 0x0000_0001;
const PIPE_TYPE_BYTE: u32        = 0x0000_0000;
const PIPE_READMODE_BYTE: u32    = 0x0000_0000;
const PIPE_WAIT: u32             = 0x0000_0000;
const PIPE_UNLIMITED_INSTANCES: u32 = 255;
const INVALID_HANDLE_VALUE: isize   = -1_isize;

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

pub(super) unsafe fn create_pipe_instance() -> Result<isize> {
    let pipe_name: Vec<u16> = r"\\.\pipe\wincue-mpv-audio"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let handle = CreateNamedPipeW(
        pipe_name.as_ptr(),
        PIPE_ACCESS_INBOUND,
        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
        PIPE_UNLIMITED_INSTANCES,
        0, 65536, 0,
        std::ptr::null(),
    );

    if handle == INVALID_HANDLE_VALUE {
        return Err(anyhow!("CreateNamedPipeW failed"));
    }
    Ok(handle)
}

/// Persistent PCM pipe reader.  Runs for the lifetime of the application.
///
/// Loops: create named-pipe server instance → block until mpv connects →
/// read samples until the connection drops → repeat.
pub(super) fn pcm_pipe_manager(audio_engine: Arc<AudioEngine>, discard: Arc<AtomicBool>) {
    let sample_rate = audio_engine.sample_rate();
    let max_prebuffer: usize = (sample_rate as usize * 60 / 1000) * 2;

    loop {
        let handle = match unsafe { create_pipe_instance() } {
            Ok(h) => h,
            Err(e) => {
                log::error!("[pcm-pipe] Failed to create pipe instance: {e}");
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
        };

        log::info!("[pcm-pipe] Waiting for mpv to connect...");
        unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
        log::info!("[pcm-pipe] mpv connected — ring buffer created");

        let ring_size = (sample_rate as usize * 2 * 3).max(16384);
        let (mut prod, cons) = HeapRb::<f32>::new(ring_size).split();
        audio_engine.set_video_pcm_consumer(Some(cons));

        let mut raw = [0u8; 4096];
        let mut samples_pushed: u64 = 0;

        loop {
            let is_discard = discard.load(Ordering::Acquire);

            if !is_discard && prod.occupied_len() > max_prebuffer {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }

            let mut bytes_read: u32 = 0;
            let ok = unsafe {
                ReadFile(
                    handle,
                    raw.as_mut_ptr().cast(),
                    raw.len() as u32,
                    &mut bytes_read,
                    std::ptr::null_mut(),
                )
            };

            if ok == 0 || bytes_read == 0 {
                break;
            }

            if !is_discard {
                for chunk in raw[..bytes_read as usize].chunks_exact(4) {
                    let sample = f32::from_le_bytes(chunk.try_into().unwrap());
                    let _ = prod.try_push(sample);
                    samples_pushed += 1;
                }
            }
        }

        let sr = sample_rate as f64;
        log::info!(
            "[pcm-pipe] mpv disconnected — {samples_pushed} samples ({:.1}ms stereo) — \
             clearing consumer",
            samples_pushed as f64 / 2.0 / sr * 1000.0,
        );
        audio_engine.set_video_pcm_consumer(None);
        unsafe {
            DisconnectNamedPipe(handle);
            CloseHandle(handle);
        }
    }
}
