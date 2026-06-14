//! Win32 window creation, message loop, and fullscreen helpers for the output window.
//!
//! The parent popup window owns mpv's D3D11 render child.
//! The fade overlay child window sits above it for dip-to-black transitions.
//! The timer overlay child window sits above everything and shows the cue countdown.

use anyhow::{anyhow, Result};

use super::fade::{execute_fade_pending, set_overlay_alpha};
use super::types::OutputWndState;
use super::{
    wide, FADE_OVERLAY_HWND, FADE_STATE, FADE_TIMER_ID,
    FLOAT_TIMER_FONT, FLOAT_TIMER_HWND, FLOAT_TIMER_TEXT,
    OUTPUT_PARENT_HWND, OUTPUT_WND_STATE, TIMER_OVERLAY_HWND, TIMER_TEXT,
    WM_DO_FADE, WM_FLOAT_VISIBILITY, WM_SETUP_MPV_CHILD,
};

/// Posted to the parent window to show/hide the timer overlay from any thread.
/// wparam = 1 → show, wparam = 0 → hide.
pub(super) const WM_TIMER_VISIBILITY: u32 = 0x8003;

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
                    MSG, SW_HIDE, SW_SHOWNA, WS_CLIPCHILDREN, WS_EX_NOACTIVATE, WS_POPUP, WS_SIZEBOX,
                    WNDCLASSEXW,
                };

                let hinstance = GetModuleHandleW(std::ptr::null());

                // --- Parent window class ---
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

                // --- Fade overlay class ---
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

                // --- Timer overlay class (plain child, dark background, no layering) ---
                let timer_class = wide("WinCueTimerOverlay\0");
                let wc_timer = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(timer_wnd_proc),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: hinstance,
                    hIcon: 0,
                    hCursor: 0,
                    hbrBackground: 0, // painted in WM_PAINT
                    lpszMenuName: std::ptr::null(),
                    lpszClassName: timer_class.as_ptr(),
                    hIconSm: 0,
                };
                RegisterClassExW(&wc_timer);

                // --- Create parent window ---
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

                // --- Fade overlay (layered, full-size, dip-to-black) ---
                let overlay_name = wide("WinCueFadeOverlay\0");
                let overlay_hwnd = CreateWindowExW(
                    WS_EX_LAYERED,
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
                    log::warn!("[output] CreateWindowExW (fade overlay) failed — fades disabled");
                }

                // --- Timer overlay (plain child, initially hidden, centered) ---
                // WS_EX_TRANSPARENT: mouse events pass through to the parent so
                // dragging the output window still works.
                // Width = 70% of parent (896px), height = 40% (288px).
                let (tw, th) = (896i32, 288i32);
                let (tx_pos, ty_pos) = ((1280 - tw) / 2, (720 - th) / 2);
                let timer_name = wide("WinCueTimerOverlay\0");
                let timer_hwnd = CreateWindowExW(
                    WS_EX_TRANSPARENT, // pass-through for mouse; does NOT affect WM_PAINT
                    timer_class.as_ptr(),
                    timer_name.as_ptr(),
                    WS_CHILD, // starts hidden (no WS_VISIBLE)
                    tx_pos, ty_pos, tw, th,
                    parent_hwnd, 0, hinstance, std::ptr::null(),
                );

                if timer_hwnd != 0 {
                    TIMER_TEXT.get_or_init(|| Mutex::new(String::new()));
                    TIMER_OVERLAY_HWND.get_or_init(|| timer_hwnd);
                    ShowWindow(timer_hwnd, SW_HIDE); // explicit hide
                } else {
                    log::warn!("[output] CreateWindowExW (timer overlay) failed");
                }

                // --- Floating timer window class ---
                let float_class = wide("WinCueFloatTimer\0");
                let wc_float = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(floating_timer_wnd_proc),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: hinstance,
                    hIcon: 0,
                    hCursor: 0,
                    hbrBackground: 0,
                    lpszMenuName: std::ptr::null(),
                    lpszClassName: float_class.as_ptr(),
                    hIconSm: 0,
                };
                RegisterClassExW(&wc_float);

                // --- Create floating timer window ---
                // WS_EX_TOPMOST  : always on top of other windows
                // WS_EX_TOOLWINDOW: no taskbar button
                // WS_EX_NOACTIVATE: won't steal focus from the main app
                // WS_POPUP | WS_SIZEBOX: borderless + resizable grip
                const WS_EX_TOPMOST:     u32 = 0x0000_0008;
                const WS_EX_TOOLWINDOW:  u32 = 0x0000_0080;
                let float_name = wide("WinCue Timer\0");
                let float_hwnd = CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    float_class.as_ptr(),
                    float_name.as_ptr(),
                    WS_POPUP | WS_SIZEBOX,
                    100, 100, 420, 110,
                    0, 0, hinstance, std::ptr::null(),
                );

                if float_hwnd != 0 {
                    use std::sync::Mutex as StdMutex;
                    FLOAT_TIMER_TEXT.get_or_init(|| StdMutex::new(String::new()));
                    FLOAT_TIMER_FONT.get_or_init(|| StdMutex::new("Arial".to_owned()));
                    FLOAT_TIMER_HWND.get_or_init(|| float_hwnd);
                    ShowWindow(float_hwnd, SW_HIDE);
                } else {
                    log::warn!("[output] CreateWindowExW (floating timer) failed");
                }

                ShowWindow(parent_hwnd, SW_HIDE);
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
// Shared resize hit-testing helper
// ---------------------------------------------------------------------------

/// Returns the appropriate HTXXX resize constant when the cursor is within
/// `border` pixels of a window edge, or `None` for the interior.
///
/// Called from WM_NCHITTEST handlers so borderless windows still resize from
/// all four edges and all four corners.
fn resize_hit(hwnd: isize, lparam: isize) -> Option<isize> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowRect,
        HTLEFT, HTRIGHT, HTTOP, HTBOTTOM,
        HTTOPLEFT, HTTOPRIGHT, HTBOTTOMLEFT, HTBOTTOMRIGHT,
    };
    const BORDER: i32 = 8;
    // lparam carries the cursor screen position.
    let cx = (lparam & 0xFFFF) as i16 as i32;
    let cy = ((lparam >> 16) & 0xFFFF) as i16 as i32;
    unsafe {
        let mut wr: RECT = std::mem::zeroed();
        GetWindowRect(hwnd, &mut wr);
        let left   = cx < wr.left   + BORDER;
        let right  = cx > wr.right  - BORDER;
        let top    = cy < wr.top    + BORDER;
        let bottom = cy > wr.bottom - BORDER;
        match (top, bottom, left, right) {
            (true,  _,     true,  _)     => Some(HTTOPLEFT     as isize),
            (true,  _,     _,     true)  => Some(HTTOPRIGHT    as isize),
            (_,     true,  true,  _)     => Some(HTBOTTOMLEFT  as isize),
            (_,     true,  _,     true)  => Some(HTBOTTOMRIGHT as isize),
            (true,  _,     _,     _)     => Some(HTTOP         as isize),
            (_,     true,  _,     _)     => Some(HTBOTTOM      as isize),
            (_,     _,     true,  _)     => Some(HTLEFT        as isize),
            (_,     _,     _,     true)  => Some(HTRIGHT       as isize),
            _                            => None,
        }
    }
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
        SW_HIDE, SW_SHOWNA, WM_CLOSE, WM_DESTROY, WM_NCCALCSIZE, WM_SIZE,
        WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_MOUSEACTIVATE, WM_NCHITTEST,
        WM_SETCURSOR, HTCLIENT, HTCAPTION,
    };

    const WM_TIMER: u32      = 0x0113;
    const MA_NOACTIVATE: isize = 3;

    match msg {
        WM_MOUSEACTIVATE => MA_NOACTIVATE,

        WM_NCCALCSIZE => 0,

        WM_NCHITTEST => {
            // Resize borders — checked before falling back to client-area drag.
            let is_fullscreen = OUTPUT_WND_STATE.get()
                .and_then(|m| m.lock().ok())
                .map(|s| s.is_fullscreen)
                .unwrap_or(false);
            if !is_fullscreen {
                if let Some(hit) = resize_hit(hwnd, lparam) { return hit; }
            }
            HTCLIENT as isize
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
                SetWindowPos, HWND_TOP, SWP_NOMOVE, SWP_NOSIZE, SWP_NOACTIVATE,
            };
            const GWL_EXSTYLE: i32 = -20;
            const WS_EX_TRANSPARENT: isize = 0x20;
            let child = GetWindow(hwnd, GW_CHILD);
            if child != 0 {
                let ex = GetWindowLongPtrW(child, GWL_EXSTYLE);
                SetWindowLongPtrW(child, GWL_EXSTYLE, ex | WS_EX_TRANSPARENT);
            }
            // Fade overlay on top of mpv child, timer overlay on top of fade.
            if let Some(&overlay) = FADE_OVERLAY_HWND.get() {
                SetWindowPos(overlay, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
            if let Some(&timer) = TIMER_OVERLAY_HWND.get() {
                SetWindowPos(timer, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
            0
        }

        WM_SIZE => {
            let w = (lparam & 0xFFFF) as i32;
            let h = ((lparam >> 16) & 0xFFFF) as i32;
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                SetWindowPos, SWP_NOZORDER, SWP_NOACTIVATE, SWP_NOMOVE,
            };
            // Resize fade overlay to fill the parent.
            if let Some(&overlay) = FADE_OVERLAY_HWND.get() {
                SetWindowPos(overlay, 0, 0, 0, w, h, SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE);
            }
            // Keep timer overlay centered and proportional (70% × 40%).
            if let Some(&timer) = TIMER_OVERLAY_HWND.get() {
                let tw = (w * 70 / 100).max(400);
                let th = (h * 40 / 100).max(160);
                let tx_pos = (w - tw) / 2;
                let ty_pos = (h - th) / 2;
                SetWindowPos(timer, 0, tx_pos, ty_pos, tw, th, SWP_NOACTIVATE | SWP_NOZORDER);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        // Thread-safe show/hide for the timer overlay (posted from any thread).
        WM_TIMER_VISIBILITY => {
            if let Some(&timer) = TIMER_OVERLAY_HWND.get() {
                use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
                if wparam != 0 {
                    ShowWindow(timer, SW_SHOWNA);
                    InvalidateRect(timer, std::ptr::null(), 1);
                } else {
                    ShowWindow(timer, SW_HIDE);
                }
            }
            0
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

    // Return HTTRANSPARENT so all mouse events fall through to the parent window
    // (drag, double-click fullscreen). This replaces WS_EX_TRANSPARENT, which must
    // NOT be combined with WS_EX_LAYERED — that combination prevents SetLayeredWindowAttributes
    // from rendering the overlay's own black background, breaking fade animations.
    const WM_NCHITTEST:  u32   = 0x0084;
    const HTTRANSPARENT: isize = -1;

    if msg == WM_NCHITTEST {
        return HTTRANSPARENT;
    }

    DefWindowProcW(hwnd, msg, wparam, lparam)
}

// ---------------------------------------------------------------------------
// Timer overlay window procedure
// ---------------------------------------------------------------------------

unsafe extern "system" fn timer_wnd_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::DefWindowProcW;

    const WM_PAINT:      u32 = 0x000F;
    const WM_ERASEBKGND: u32 = 0x0014;

    match msg {
        // Suppress default background erase — WM_PAINT fills everything.
        WM_ERASEBKGND => 1,

        WM_PAINT => {
            use windows_sys::Win32::Foundation::RECT;
            use windows_sys::Win32::Graphics::Gdi::{
                BeginPaint, EndPaint, PAINTSTRUCT,
                GetStockObject, BLACK_BRUSH,
                FillRect, SetBkMode, SetTextColor, SelectObject,
                CreateFontW, DeleteObject, DrawTextW,
            };
            use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

            // get_or_init guarantees we always have a Mutex, even before the
            // Win32 init thread has had a chance to call get_or_init itself.
            let text_owned: String = TIMER_TEXT
                .get_or_init(|| std::sync::Mutex::new(String::new()))
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default();

            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);

            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut rc);

            // Solid dark background.
            FillRect(hdc, &rc, GetStockObject(BLACK_BRUSH) as isize);

            if !text_owned.is_empty() {
                const TRANSPARENT_BK: i32  = 1;
                const FW_BOLD: i32         = 700;
                const ANSI_CHARSET: u32    = 0;
                const DEFAULT_QUALITY: u32 = 0;
                const DT_CENTER:     u32   = 0x0000_0001;
                const DT_VCENTER:    u32   = 0x0000_0004;
                const DT_SINGLELINE: u32   = 0x0000_0020;
                const DT_NOCLIP:     u32   = 0x0000_0100;

                SetBkMode(hdc, TRANSPARENT_BK);

                // Font: 60% of window height — massive, readable from a distance.
                let font_h = -((rc.bottom - rc.top) * 60 / 100).max(40);
                let face = wide("Arial\0");
                let font = CreateFontW(
                    font_h, 0, 0, 0, FW_BOLD, 0, 0, 0,
                    ANSI_CHARSET, 0, 0, DEFAULT_QUALITY, 0,
                    face.as_ptr(),
                );
                let old_font = SelectObject(hdc, font as isize);

                let wtext: Vec<u16> = text_owned.encode_utf16().chain(std::iter::once(0)).collect();

                // Drop shadow: draw black text slightly offset, then white on top.
                // IMPORTANT: rc must be *mut RECT — DrawTextW with DT_VCENTER
                // writes the computed bounding box back into the RECT.
                let mut shadow_rc = RECT { left: rc.left + 3, top: rc.top + 3, right: rc.right, bottom: rc.bottom };
                SetTextColor(hdc, 0x0000_0000);
                DrawTextW(hdc, wtext.as_ptr(), -1, &mut shadow_rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOCLIP);

                let mut draw_rc = rc; // mutable copy for DrawTextW
                SetTextColor(hdc, 0x00FF_FFFF);
                DrawTextW(hdc, wtext.as_ptr(), -1, &mut draw_rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOCLIP);

                SelectObject(hdc, old_font);
                DeleteObject(font as isize);

            }

            EndPaint(hwnd, &mut ps);
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
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

// ---------------------------------------------------------------------------
// Floating timer window procedure
// ---------------------------------------------------------------------------

unsafe extern "system" fn floating_timer_wnd_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::DefWindowProcW;

    const WM_PAINT:        u32 = 0x000F;
    const WM_ERASEBKGND:   u32 = 0x0014;
    const WM_CLOSE:        u32 = 0x0010;
    const WM_NCCALCSIZE:   u32 = 0x0083;
    const WM_NCHITTEST:    u32 = 0x0084;
    const WM_GETMINMAXINFO: u32 = 0x0024;

    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HTCLIENT, HTCAPTION, ShowWindow, SW_HIDE, SW_SHOWNA,
    };

    match msg {
        WM_NCCALCSIZE => 0, // full client area — no OS chrome

        WM_NCHITTEST => {
            // Resize borders take priority; interior = caption for native drag.
            if let Some(hit) = resize_hit(hwnd, lparam) { return hit; }
            HTCAPTION as isize
        }

        WM_GETMINMAXINFO => {
            use windows_sys::Win32::UI::WindowsAndMessaging::MINMAXINFO;
            let mmi = &mut *(lparam as *mut MINMAXINFO);
            mmi.ptMinTrackSize.x = 160;
            mmi.ptMinTrackSize.y = 60;
            0
        }

        WM_ERASEBKGND => 1,

        WM_PAINT => {
            use windows_sys::Win32::Foundation::RECT;
            use windows_sys::Win32::Graphics::Gdi::{
                BeginPaint, EndPaint, PAINTSTRUCT,
                GetStockObject, BLACK_BRUSH,
                FillRect, SetBkMode, SetTextColor, SelectObject,
                CreateFontW, DeleteObject, DrawTextW,
                CreateCompatibleDC, CreateCompatibleBitmap, BitBlt, DeleteDC,
            };
            use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

            let text_owned: String = FLOAT_TIMER_TEXT
                .get_or_init(|| std::sync::Mutex::new(String::new()))
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default();

            let font_name: String = FLOAT_TIMER_FONT
                .get_or_init(|| std::sync::Mutex::new("Arial".to_owned()))
                .lock()
                .map(|g| g.clone())
                .unwrap_or_else(|_| "Arial".to_owned());

            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);

            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut rc);
            let w = rc.right - rc.left;
            let h = rc.bottom - rc.top;

            // Double-buffer: paint into a memory DC, then blit once — no flicker.
            let hdc_mem = CreateCompatibleDC(hdc);
            let hbm = CreateCompatibleBitmap(hdc, w, h);
            let old_hbm = SelectObject(hdc_mem, hbm as isize);

            FillRect(hdc_mem, &rc, GetStockObject(BLACK_BRUSH) as isize);

            const SRCCOPY: u32 = 0x00CC_0020;

            if !text_owned.is_empty() {
                use windows_sys::Win32::Foundation::SIZE;
                use windows_sys::Win32::Graphics::Gdi::GetTextExtentPoint32W;

                const TRANSPARENT_BK: i32  = 1;
                const FW_BOLD: i32         = 700;
                const ANSI_CHARSET: u32    = 0;
                const DEFAULT_QUALITY: u32 = 0;
                const DT_CENTER:     u32   = 0x0000_0001;
                const DT_VCENTER:    u32   = 0x0000_0004;
                const DT_SINGLELINE: u32   = 0x0000_0020;
                const DT_NOCLIP:     u32   = 0x0000_0100;

                SetBkMode(hdc_mem, TRANSPARENT_BK);

                let wtext: Vec<u16> = text_owned.encode_utf16().chain(std::iter::once(0)).collect();
                let nchars = (wtext.len() - 1) as i32; // exclude null terminator
                let mut face: Vec<u16> = font_name.encode_utf16().chain(std::iter::once(0)).collect();

                // Auto-fit: start from 80% of window height, then shrink to fit width.
                let max_h = (h * 80 / 100).max(24);
                let probe = CreateFontW(
                    -max_h, 0, 0, 0, FW_BOLD, 0, 0, 0,
                    ANSI_CHARSET, 0, 0, DEFAULT_QUALITY, 0,
                    face.as_mut_ptr(),
                );
                let old_probe = SelectObject(hdc_mem, probe as isize);
                let mut sz: SIZE = std::mem::zeroed();
                GetTextExtentPoint32W(hdc_mem, wtext.as_ptr(), nchars, &mut sz);
                SelectObject(hdc_mem, old_probe);
                DeleteObject(probe as isize);

                // If text is wider than 92% of the window, scale font height down.
                let avail_w = (w * 92 / 100).max(1);
                let font_h = if sz.cx > avail_w {
                    -(max_h as i64 * avail_w as i64 / sz.cx as i64).max(10) as i32
                } else {
                    -max_h
                };

                let font = CreateFontW(
                    font_h, 0, 0, 0, FW_BOLD, 0, 0, 0,
                    ANSI_CHARSET, 0, 0, DEFAULT_QUALITY, 0,
                    face.as_mut_ptr(),
                );
                let old_font = SelectObject(hdc_mem, font as isize);

                let mut shadow_rc = RECT { left: rc.left + 2, top: rc.top + 2, right: rc.right, bottom: rc.bottom };
                SetTextColor(hdc_mem, 0x0000_0000);
                DrawTextW(hdc_mem, wtext.as_ptr(), -1, &mut shadow_rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOCLIP);

                let mut draw_rc = rc;
                SetTextColor(hdc_mem, 0x00FF_FFFF);
                DrawTextW(hdc_mem, wtext.as_ptr(), -1, &mut draw_rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOCLIP);

                SelectObject(hdc_mem, old_font);
                DeleteObject(font as isize);
            }

            BitBlt(hdc, 0, 0, w, h, hdc_mem, 0, 0, SRCCOPY);

            SelectObject(hdc_mem, old_hbm);
            DeleteObject(hbm as isize);
            DeleteDC(hdc_mem);

            EndPaint(hwnd, &mut ps);
            0
        }

        WM_FLOAT_VISIBILITY => {
            if wparam != 0 {
                ShowWindow(hwnd, SW_SHOWNA);
                use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
                InvalidateRect(hwnd, std::ptr::null(), 1);
            } else {
                ShowWindow(hwnd, SW_HIDE);
            }
            0
        }

        // Hide (don't destroy) when the user presses Alt+F4 or right-click-closes.
        WM_CLOSE => {
            ShowWindow(hwnd, SW_HIDE);
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
