fn main() {
    tauri_build::build();

    // Output-window backend selection (see engine/output_engine).
    //   output_gl    → unified mpv OpenGL Render API path (render.rs). The render
    //                  loop + GL fade quad are identical on every OS; only the
    //                  native window creation differs (winit on Windows/Linux,
    //                  AppKit/objc2 on macOS — winit cannot run on Tauri's main thread).
    //   output_win32 → legacy Win32 wid-embed + layered overlay (win32_window.rs).
    // Linux + macOS always use the GL path; Windows uses it by default and Win32 only
    // when the `legacy-win32-output` feature is enabled.
    println!("cargo::rustc-check-cfg=cfg(output_gl)");
    println!("cargo::rustc-check-cfg=cfg(output_win32)");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let win32 =
        target_os == "windows" && std::env::var("CARGO_FEATURE_LEGACY_WIN32_OUTPUT").is_ok();
    if win32 {
        println!("cargo::rustc-cfg=output_win32");
    }
    if target_os == "linux" || target_os == "macos" || (target_os == "windows" && !win32) {
        println!("cargo::rustc-cfg=output_gl");
    }

    // macOS: the GL output path creates/manages its own NSWindow via raw `msg_send!`
    // (engine/output_engine/macos_window.rs), so AppKit must be linked. Foundation is
    // pulled in transitively by objc2-foundation.
    if target_os == "macos" {
        println!("cargo::rustc-link-lib=framework=AppKit");
    }

    // Copy libmpv-2.dll next to the compiled binary so it can be loaded at runtime.
    // OUT_DIR is  target/{profile}/build/inkue-<hash>/out  — three levels up is target/{profile}.
    #[cfg(target_os = "windows")]
    {
        let out_dir = std::env::var("OUT_DIR").unwrap();
        let target_dir = std::path::Path::new(&out_dir)
            .ancestors()
            .nth(3)
            .unwrap()
            .to_path_buf();

        let dll_src = std::path::Path::new("vendor/mpv/libmpv-2.dll");
        let dll_dst = target_dir.join("libmpv-2.dll");

        if dll_src.exists() {
            if let Err(e) = std::fs::copy(dll_src, &dll_dst) {
                // The destination is locked while the app is running (`tauri dev`
                // holds libmpv-2.dll open). If a copy is already in place, keep going
                // rather than failing the whole build; otherwise it's a real error.
                if dll_dst.exists() {
                    println!("cargo:warning=libmpv-2.dll in use — keeping existing copy ({e})");
                } else {
                    panic!("Failed to copy vendor/mpv/libmpv-2.dll to target dir: {e}");
                }
            }
            println!("cargo:rerun-if-changed=vendor/mpv/libmpv-2.dll");
        } else {
            println!("cargo:warning=vendor/mpv/libmpv-2.dll not found — video playback will fail at runtime");
        }
    }
}
