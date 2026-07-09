//! Camera / capture-device enumeration, per OS.
//!
//! Returns the identifiers ffmpeg's capture demuxers expect (which is what
//! libmpv uses to open the feed — see `CameraSource::mpv_url`):
//! - **Windows**: DirectShow friendly names (COM `ICreateDevEnum` over the
//!   video-input category — the exact names lavf's `dshow` demuxer matches).
//! - **Linux**: `/dev/videoN` paths, names from sysfs (no ioctl needed).
//! - **macOS**: AVFoundation device names via `AVCaptureDevice` (objc2).
//!
//! Enumeration can touch flaky drivers, so the command layer runs it on a
//! blocking thread — never on the main thread (same rule as audio devices).

use serde::Serialize;

/// One connected capture device.
#[derive(Debug, Clone, Serialize)]
pub struct CameraDeviceInfo {
    /// Platform identifier used to open the device (DirectShow name on
    /// Windows, `/dev/videoN` on Linux, AVFoundation name on macOS).
    pub id: String,
    /// Human-readable name shown in the inspector.
    pub name: String,
}

// ---------------------------------------------------------------------------
// Windows — DirectShow video-input category via COM
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
pub fn list_camera_devices() -> Vec<CameraDeviceInfo> {
    windows_impl::enumerate().unwrap_or_default()
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::CameraDeviceInfo;
    use std::ffi::c_void;
    use windows_sys::core::GUID;
    use windows_sys::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    };
    use windows_sys::Win32::System::Variant::{VariantClear, VARIANT, VT_BSTR};

    const CLSID_SYSTEM_DEVICE_ENUM: GUID = GUID {
        data1: 0x62BE5D10, data2: 0x60EB, data3: 0x11D0,
        data4: [0xBD, 0x3B, 0x00, 0xA0, 0xC9, 0x11, 0xCE, 0x86],
    };
    const CLSID_VIDEO_INPUT_DEVICE_CATEGORY: GUID = GUID {
        data1: 0x860BB310, data2: 0x5D01, data3: 0x11D0,
        data4: [0xBD, 0x3B, 0x00, 0xA0, 0xC9, 0x11, 0xCE, 0x86],
    };
    const IID_ICREATE_DEV_ENUM: GUID = GUID {
        data1: 0x29840822, data2: 0x5B84, data3: 0x11D0,
        data4: [0xBD, 0x3B, 0x00, 0xA0, 0xC9, 0x11, 0xCE, 0x86],
    };
    const IID_IPROPERTY_BAG: GUID = GUID {
        data1: 0x55272A00, data2: 0x42CB, data3: 0x11CE,
        data4: [0x81, 0x35, 0x00, 0xAA, 0x00, 0x4B, 0xB8, 0x51],
    };

    // windows-sys 0.52 exposes COM interfaces as opaque pointers, so the
    // vtables are laid out by hand.  Each struct is the *flat* method table
    // in declaration order (IUnknown first); unused slots are `usize`.

    type Release = unsafe extern "system" fn(*mut c_void) -> u32;

    /// `ICreateDevEnum`: IUnknown + CreateClassEnumerator.
    #[repr(C)]
    struct ICreateDevEnumVtbl {
        _query_interface: usize,
        _add_ref: usize,
        release: Release,
        create_class_enumerator:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void, u32) -> i32,
    }

    /// `IEnumMoniker`: IUnknown + Next (Skip/Reset/Clone unused).
    #[repr(C)]
    struct IEnumMonikerVtbl {
        _query_interface: usize,
        _add_ref: usize,
        release: Release,
        next: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void, *mut u32) -> i32,
    }

    /// `IMoniker`: IUnknown(3) + IPersist(1) + IPersistStream(4) +
    /// BindToObject + BindToStorage (rest unused).
    #[repr(C)]
    struct IMonikerVtbl {
        _query_interface: usize,
        _add_ref: usize,
        release: Release,
        _get_class_id: usize,
        _is_dirty: usize,
        _load: usize,
        _save: usize,
        _get_size_max: usize,
        _bind_to_object: usize,
        bind_to_storage: unsafe extern "system" fn(
            *mut c_void,
            *mut c_void,
            *mut c_void,
            *const GUID,
            *mut *mut c_void,
        ) -> i32,
    }

    /// `IPropertyBag`: IUnknown + Read (Write unused).
    #[repr(C)]
    struct IPropertyBagVtbl {
        _query_interface: usize,
        _add_ref: usize,
        release: Release,
        read: unsafe extern "system" fn(
            *mut c_void,
            *const u16,
            *mut VARIANT,
            *mut c_void,
        ) -> i32,
    }

    /// The vtable pointer sits at offset 0 of every COM object.
    unsafe fn vtbl<T>(obj: *mut c_void) -> *const T {
        unsafe { *(obj as *mut *const T) }
    }

    pub(super) fn enumerate() -> Option<Vec<CameraDeviceInfo>> {
        unsafe {
            // Per-thread COM init; S_FALSE (already initialised) is fine.
            let hr_init = CoInitializeEx(std::ptr::null(), COINIT_MULTITHREADED as u32);
            let need_uninit = hr_init >= 0;

            let result = enumerate_inner();

            if need_uninit {
                CoUninitialize();
            }
            result
        }
    }

    unsafe fn enumerate_inner() -> Option<Vec<CameraDeviceInfo>> {
        let mut dev_enum: *mut c_void = std::ptr::null_mut();
        let hr = unsafe {
            CoCreateInstance(
                &CLSID_SYSTEM_DEVICE_ENUM,
                std::ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_ICREATE_DEV_ENUM,
                &mut dev_enum,
            )
        };
        if hr < 0 || dev_enum.is_null() {
            log::warn!("[camera] CoCreateInstance(SystemDeviceEnum) failed: 0x{hr:08x}");
            return None;
        }

        let mut enum_moniker: *mut c_void = std::ptr::null_mut();
        let hr = unsafe {
            ((*vtbl::<ICreateDevEnumVtbl>(dev_enum)).create_class_enumerator)(
                dev_enum,
                &CLSID_VIDEO_INPUT_DEVICE_CATEGORY,
                &mut enum_moniker,
                0,
            )
        };
        // S_FALSE (1) = the category is empty (no cameras connected).
        let devices = if hr == 0 && !enum_moniker.is_null() {
            unsafe { drain_monikers(enum_moniker) }
        } else {
            Vec::new()
        };

        unsafe {
            if !enum_moniker.is_null() {
                ((*vtbl::<IEnumMonikerVtbl>(enum_moniker)).release)(enum_moniker);
            }
            ((*vtbl::<ICreateDevEnumVtbl>(dev_enum)).release)(dev_enum);
        }
        Some(devices)
    }

    unsafe fn drain_monikers(enum_moniker: *mut c_void) -> Vec<CameraDeviceInfo> {
        let mut out = Vec::new();
        loop {
            let mut moniker: *mut c_void = std::ptr::null_mut();
            let mut fetched: u32 = 0;
            let hr = unsafe {
                ((*vtbl::<IEnumMonikerVtbl>(enum_moniker)).next)(
                    enum_moniker, 1, &mut moniker, &mut fetched,
                )
            };
            if hr != 0 || fetched == 0 || moniker.is_null() {
                break;
            }

            if let Some(name) = unsafe { friendly_name(moniker) } {
                out.push(CameraDeviceInfo { id: name.clone(), name });
            }
            unsafe {
                ((*vtbl::<IMonikerVtbl>(moniker)).release)(moniker);
            }
        }
        out
    }

    unsafe fn friendly_name(moniker: *mut c_void) -> Option<String> {
        let mut bag: *mut c_void = std::ptr::null_mut();
        let hr = unsafe {
            ((*vtbl::<IMonikerVtbl>(moniker)).bind_to_storage)(
                moniker,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &IID_IPROPERTY_BAG,
                &mut bag,
            )
        };
        if hr < 0 || bag.is_null() {
            return None;
        }

        let mut var: VARIANT = unsafe { std::mem::zeroed() };
        let key: Vec<u16> = "FriendlyName".encode_utf16().chain(std::iter::once(0)).collect();
        let hr = unsafe {
            ((*vtbl::<IPropertyBagVtbl>(bag)).read)(
                bag, key.as_ptr(), &mut var, std::ptr::null_mut(),
            )
        };

        let name = if hr == 0 && unsafe { var.Anonymous.Anonymous.vt } == VT_BSTR {
            let bstr = unsafe { var.Anonymous.Anonymous.Anonymous.bstrVal };
            if bstr.is_null() {
                None
            } else {
                let mut len = 0usize;
                while unsafe { *bstr.add(len) } != 0 {
                    len += 1;
                }
                let slice = unsafe { std::slice::from_raw_parts(bstr, len) };
                Some(String::from_utf16_lossy(slice))
            }
        } else {
            None
        };

        unsafe {
            VariantClear(&mut var);
            ((*vtbl::<IPropertyBagVtbl>(bag)).release)(bag);
        }
        name
    }
}

// ---------------------------------------------------------------------------
// Linux — sysfs scan (no ioctl needed for names)
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
pub fn list_camera_devices() -> Vec<CameraDeviceInfo> {
    let Ok(entries) = std::fs::read_dir("/sys/class/video4linux") else {
        return Vec::new();
    };
    let mut out: Vec<CameraDeviceInfo> = entries
        .flatten()
        .filter_map(|entry| {
            let dev = entry.file_name().to_string_lossy().to_string();
            if !dev.starts_with("video") {
                return None;
            }
            let name = std::fs::read_to_string(entry.path().join("name"))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| dev.clone());
            Some(CameraDeviceInfo { id: format!("/dev/{dev}"), name })
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

// ---------------------------------------------------------------------------
// macOS — AVFoundation via objc2
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
pub fn list_camera_devices() -> Vec<CameraDeviceInfo> {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use objc2_foundation::NSString;

    unsafe {
        // AVMediaTypeVideo == @"vide" (constant string from AVFoundation).
        let media_type = NSString::from_str("vide");
        let devices: *mut AnyObject =
            msg_send![class!(AVCaptureDevice), devicesWithMediaType: &*media_type];
        if devices.is_null() {
            return Vec::new();
        }
        let count: usize = msg_send![devices, count];
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let device: *mut AnyObject = msg_send![devices, objectAtIndex: i];
            if device.is_null() {
                continue;
            }
            let name_obj: *mut AnyObject = msg_send![device, localizedName];
            if name_obj.is_null() {
                continue;
            }
            let name = (*(name_obj as *mut NSString)).to_string();
            // lavf's avfoundation demuxer accepts the device name directly.
            out.push(CameraDeviceInfo { id: name.clone(), name });
        }
        out
    }
}
