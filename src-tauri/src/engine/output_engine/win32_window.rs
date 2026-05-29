//! Win32 window creation, message loop, and fullscreen helpers for the output window.
//!
//! The parent popup window owns mpv's D3D11 render child.
//! The fade overlay child window sits above it for dip-to-black transitions.

use anyhow::{anyhow, Result};

use super::fade::{execute_fade_pending, set_overlay_alpha};
use super::types::OutputWndState;
use super::{
    wide, FADE_OVERLAY_HWND, FADE_STATE, FADE_TIMER_ID, OUTPUT_PARENT_HWND,
    OUTPUT_WND_STATE, WM_DO_FADE, WM_SETUP_MPV_CHILD,
};

// ---------------------------------------------------------------------------
// Window creation
// ---------------------------------------------------------------------------

/// Spawn a dedicated thread that owns the parent popup window and its Win32
/// message loop.  Returns the parent HWND.
pub(super) fn create_output_window() -> Result<isize> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<isize>>();

    std::thread::Builder::new()
        .name("wincue-output-win32".into())
        .spawn(move || {
            unsafe {
                use windows_sys::Win32::Graphics::Gdi::{GetStockObject, BLACK_BRUSH};
                use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    CreateWindowExW, DispatchMessageW, GetMessageW, RegisterClassExW,
                    ShowWindow, TranslateMessage,
                    CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW,
                    MSG, SW_SHOWNA, WS_CLIPCHILDREN, WS_EX_NOACTIVATE, WS_POPUP, WS_SIZEBOX,
                    WNDCLASSEXW,
                };

                let hinstance = GetModuleHandleW(std::ptr::null());

                let parent_class = wide("WinCueOutputWnd\0");
                let wc_parent = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
                    lpfnWndProc: Some(output_wnd_proc),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: hinstance,
                    hIcon: 0,
                    hCursor: 0,
                    hbrBackground: GetStockObject(BLACK_BRUSH) as isize,
                    lpszMenuName: std::ptr::null(),
                    lpszClassName: parent_class.as_ptr(),
                    hIconSm: 0,
                };
                RegisterClassExW(&wc_parent);

                let overlay_class = wide("WinCueFadeOverlay\0");
                let wc_overlay = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: 0,
                    lpfnWndProc: Some(overlay_wnd_proc),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: hinstance,
                    hIcon: 0,
                    hCursor: 0,
                    hbrBackground: GetStockObject(BLACK_BRUSH) as isize,
                    lpszMenuName: std::ptr::null(),
                    lpszClassName: overlay_class.as_ptr(),
                    hIconSm: 0,
                };
                RegisterClassExW(&wc_overlay);

                let window_name = wide("WinCue Output\0");
                let parent_hwnd = CreateWindowExW(
                    WS_EX_NOACTIVATE,
                    parent_class.as_ptr(),
                    window_name.as_ptr(),
                    WS_POPUP | WS_CLIPCHILDREN | WS_SIZEBOX,
                    100, 100, 1280, 720,
                    0, 0, hinstance, std::ptr::null(),
                );

                if parent_hwnd == 0 {
                    let _ = tx.send(Err(anyhow!("CreateWindowExW (parent) failed")));
                    return;
                }

                use std::sync::Mutex;
                OUTPUT_WND_STATE.get_or_init(|| {
                    Mutex::new(OutputWndState {
                        is_fullscreen: false,
                        saved_rect: (100, 100, 100 + 1280, 100 + 720),
                    })
                });
                OUTPUT_PARENT_HWND.get_or_init(|| parent_hwnd);

                const WS_EX_LAYERED: u32     = 0x0008_0000;
                const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
                const WS_CHILD: u32          = 0x4000_0000;
                const WS_VISIBLE: u32        = 0x1000_0000;

                let overlay_name = wide("WinCueFadeOverlay\0");
                let overlay_hwnd = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TRANSPARENT,
                    overlay_class.as_ptr(),
                    overlay_name.as_ptr(),
                    WS_CHILD | WS_VISIBLE,
                    0, 0, 1280, 720,
                    parent_hwnd, 0, hinstance, std::ptr::null(),
                );

                if overlay_hwnd != 0 {
                    use windows_sys::Win32::UI::WindowsAndMessaging::SetLayeredWindowAttributes;
                    const LWA_ALPHA: u32 = 0x2;
                    SetLayeredWindowAttributes(overlay_hwnd, 0, 0, LWA_ALPHA);
                    FADE_OVERLAY_HWND.get_or_init(|| overlay_hwnd);
                } else {
                    log::warn!(
                        "[output] CreateWindowExW (overlay) failed — fades disabled"
                    );
                }

                ShowWindow(parent_hwnd, SW_SHOWNA);
                let _ = tx.send(Ok(parent_hwnd));

                let mut msg = MSG {
                    hwnd: 0,
                    message: 0,
                    wParam: 0,
                    lParam: 0,
                    time: 0,
                    pt: windows_sys::Win32::Foundation::POINT { x: 0, y: 0 },
                };
                loop {
                    let ret = GetMessageW(&mut msg, 0, 0, 0);
                    if ret == 0 || ret == -1 {
                        break;
                    }
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        })
        .map_err(|e| anyhow!("Failed to spawn Win32 window thread: {e}"))?;

    rx.recv()
        .map_err(|_| anyhow!("Win32 window thread exited before sending HWND"))?
}

// ---------------------------------------------------------------------------
// Parent window procedure
// ---------------------------------------------------------------------------

unsafe extern "system" fn output_wnd_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, KillTimer, PostQuitMessage, SetTimer, ShowWindow,
        SW_HIDE, WM_CLOSE, WM_DESTROY, WM_NCCALCSIZE, WM_SIZE,
        WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_MOUSEACTIVATE, WM_NCHITTEST,
        WM_SETCURSOR, HTCLIENT, HTCAPTION,
    };

    const WM_TIMER: u32    = 0x0113;
    const MA_NOACTIVATE: isize = 3;

    match msg {
        WM_MOUSEACTIVATE => MA_NOACTIVATE,

        WM_NCCALCSIZE => 0,

        WM_NCHITTEST => {
            let hit = DefWindowProcW(hwnd, msg, wparam, lparam);
            if hit == HTCLIENT as isize || hit == HTCAPTION as isize {
                HTCLIENT as isize
            } else {
                hit
            }
        }

        WM_SETCURSOR => {
            let ht = lparam & 0xFFFF;
            if ht == HTCLIENT as isize {
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    IDC_ARROW, LoadCursorW, SetCursor,
                };
                SetCursor(LoadCursorW(0, IDC_ARROW));
                1
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }

        WM_LBUTTONDOWN => {
            drag_window(hwnd);
            0
        }

        WM_LBUTTONDBLCLK => {
            if let Some(state_mutex) = OUTPUT_WND_STATE.get() {
                if let Ok(mut state) = state_mutex.lock() {
                    toggle_fullscreen_impl(hwnd, &mut state);
                }
            }
            0
        }

        WM_SETUP_MPV_CHILD => {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                GetWindow, GetWindowLongPtrW, SetWindowLongPtrW, GW_CHILD,
            };
            const GWL_EXSTYLE: i32 = -20;
            const WS_EX_TRANSPARENT: isize = 0x20;
            let child = GetWindow(hwnd, GW_CHILD);
            if child != 0 {
                let ex = GetWindowLongPtrW(child, GWL_EXSTYLE);
                SetWindowLongPtrW(child, GWL_EXSTYLE, ex | WS_EX_TRANSPARENT);
            }
            if let Some(&overlay) = FADE_OVERLAY_HWND.get() {
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    SetWindowPos, HWND_TOP, SWP_NOMOVE, SWP_NOSIZE, SWP_NOACTIVATE,
                };
                SetWindowPos(
                    overlay, HWND_TOP,
                    0, 0, 0, 0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
            0
        }

        WM_SIZE => {
            if let Some(&overlay) = FADE_OVERLAY_HWND.get() {
                let w = (lparam & 0xFFFF) as i32;
                let h = ((lparam >> 16) & 0xFFFF) as i32;
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    SetWindowPos, SWP_NOZORDER, SWP_NOACTIVATE, SWP_NOMOVE,
                };
                SetWindowPos(
                    overlay, 0, 0, 0, w, h,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE,
                );
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_DO_FADE => {
            if let Some(fs) = FADE_STATE.get() {
                let duration_ms = {
                    let state = fs.lock().unwrap();
                    state.duration_ms
                };
                if duration_ms == 0 {
                    let target = fs.lock().unwrap().target_alpha;
                    set_overlay_alpha(target);
                    {
                        let mut state = fs.lock().unwrap();
                        state.current_alpha = target;
                    }
                    execute_fade_pending(hwnd);
                } else {
                    let already_active = {
                        let state = fs.lock().unwrap();
                        state.timer_active
                    };
                    if !already_active {
                        fs.lock().unwrap().timer_active = true;
                        SetTimer(hwnd, FADE_TIMER_ID, 16, None);
                    }
                }
            }
            0
        }

        WM_TIMER => {
            if wparam == FADE_TIMER_ID {
                if let Some(fs) = FADE_STATE.get() {
                    let (new_alpha, done) = {
                        let mut state = fs.lock().unwrap();
                        let elapsed = state.start_time.elapsed().as_millis() as u32;
                        let progress = if state.duration_ms == 0 {
                            1.0_f32
                        } else {
                            (elapsed as f32 / state.duration_ms as f32).min(1.0)
                        };
                        let delta =
                            (state.target_alpha as i16 - state.start_alpha as i16) as f32;
                        let new_alpha = (state.start_alpha as f32 + delta * progress) as u8;
                        state.current_alpha = new_alpha;
                        let done = progress >= 1.0 || elapsed >= state.duration_ms;
                        if done {
                            state.current_alpha = state.target_alpha;
                            state.timer_active = false;
                        }
                        (state.current_alpha, done)
                    };

                    set_overlay_alpha(new_alpha);

                    if done {
                        KillTimer(hwnd, FADE_TIMER_ID);
                        execute_fade_pending(hwnd);
                    }
                }
            }
            0
        }

        WM_CLOSE => {
            ShowWindow(hwnd, SW_HIDE);
            0
        }

        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ---------------------------------------------------------------------------
// Fade overlay window procedure
// ---------------------------------------------------------------------------

unsafe extern "system" fn overlay_wnd_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::DefWindowProcW;
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

// ---------------------------------------------------------------------------
// Drag helper
// ---------------------------------------------------------------------------

fn drag_window(hwnd: isize) {
    unsafe {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetCursorPos, PostMessageW, HTCAPTION, WM_NCLBUTTONDOWN,
        };

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let screen_lp = (pt.x as u16 as isize) | ((pt.y as u16 as isize) << 16);
        ReleaseCapture();
        PostMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, screen_lp);
    }
}

// ---------------------------------------------------------------------------
// Fullscreen helpers
// ---------------------------------------------------------------------------

pub(super) fn toggle_fullscreen_impl(hwnd: isize, state: &mut OutputWndState) {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, SetWindowPos, HWND_TOPMOST,
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER,
    };

    unsafe {
        if state.is_fullscreen {
            let (l, t, r, b) = state.saved_rect;
            set_resizable(hwnd);
            SetWindowPos(
                hwnd, 0,
                l, t, r - l, b - t,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
            state.is_fullscreen = false;
        } else {
            let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            GetWindowRect(hwnd, &mut rc);
            state.saved_rect = (rc.left, rc.top, rc.right, rc.bottom);

            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(monitor, &mut mi);

            let mr = mi.rcMonitor;
            set_borderless(hwnd);
            SetWindowPos(
                hwnd, HWND_TOPMOST,
                mr.left, mr.top,
                mr.right - mr.left, mr.bottom - mr.top,
                SWP_NOOWNERZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
            state.is_fullscreen = true;
        }
    }
}

pub(super) unsafe fn set_borderless(hwnd: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_STYLE,
    };
    const WS_SIZEBOX: isize = 0x0004_0000;
    let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
    SetWindowLongPtrW(hwnd, GWL_STYLE, style & !WS_SIZEBOX);
}

pub(super) unsafe fn set_resizable(hwnd: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_STYLE,
    };
    const WS_SIZEBOX: isize = 0x0004_0000;
    let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
    SetWindowLongPtrW(hwnd, GWL_STYLE, style | WS_SIZEBOX);
}
