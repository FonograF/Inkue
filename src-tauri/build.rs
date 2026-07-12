fn main() {
    tauri_build::build();

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

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
