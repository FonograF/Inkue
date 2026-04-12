fn main() {
    tauri_build::build();

    // Copy libmpv-2.dll next to the compiled binary so it can be loaded at runtime.
    // OUT_DIR is  target/{profile}/build/wincue-<hash>/out  — three levels up is target/{profile}.
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
            std::fs::copy(&dll_src, &dll_dst)
                .expect("Failed to copy vendor/mpv/libmpv-2.dll to target dir");
            println!("cargo:rerun-if-changed=vendor/mpv/libmpv-2.dll");
        } else {
            println!("cargo:warning=vendor/mpv/libmpv-2.dll not found — video playback will fail at runtime");
        }
    }
}
