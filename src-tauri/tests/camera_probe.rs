//! Manual probe: prints the capture devices Inkue's own enumeration sees.
//!
//! `#[ignore]` — needs real hardware (or a virtual camera like OBS's) and is
//! meant for interactive verification, not CI:
//!
//! ```text
//! cargo test --test camera_probe -- --ignored --nocapture
//! ```

#[test]
#[ignore = "manual probe — requires a physical or virtual camera"]
fn print_camera_devices() {
    let devices = inkue_lib::engine::camera_enum::list_camera_devices();
    println!("--- {} capture device(s) found ---", devices.len());
    for d in &devices {
        println!("id: {:?}  name: {:?}", d.id, d.name);
    }
    assert!(
        !devices.is_empty(),
        "no capture device found — is a webcam connected or a virtual camera running?"
    );
}
