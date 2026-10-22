//! Frontmost-app name, secure-input and Wayland detection.

/// Name of the frontmost application, or `None` if unknown.
/// Linux: not implemented in Part 1 (returns `None`).
pub fn frontmost_app_name() -> Option<String> {
    imp::frontmost_app_name()
}

/// macOS: whether Secure Event Input is on (password fields; synthetic keys
/// are swallowed). Always false elsewhere.
pub fn secure_input_active() -> bool {
    imp::secure_input_active()
}

/// Pure Wayland detection. `DISPLAY` is deliberately ignored: XWayland sets
/// it on Wayland desktops too.
pub fn detect_wayland(wayland_display: Option<&str>, session_type: Option<&str>) -> bool {
    wayland_display.is_some() || session_type == Some("wayland")
}

/// Linux Wayland session (synthetic paste is unreliable there).
pub fn is_wayland() -> bool {
    #[cfg(target_os = "linux")]
    {
        detect_wayland(
            std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
            std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2_app_kit::NSWorkspace;

    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn IsSecureEventInputEnabled() -> u8;
    }

    pub fn frontmost_app_name() -> Option<String> {
        let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        app.localizedName().map(|n| n.to_string())
    }

    pub fn secure_input_active() -> bool {
        // SAFETY: no-argument Carbon query with no side effects.
        unsafe { IsSecureEventInputEnabled() != 0 }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::path::Path;
    use windows_sys::Win32::Foundation::{CloseHandle, MAX_PATH};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };

    pub fn frontmost_app_name() -> Option<String> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() {
                return None;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == 0 {
                return None;
            }
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut buf = [0u16; MAX_PATH as usize];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
            CloseHandle(h);
            if ok == 0 {
                return None;
            }
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            Path::new(&path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        }
    }

    pub fn secure_input_active() -> bool {
        false
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod imp {
    pub fn frontmost_app_name() -> Option<String> {
        None
    }
    pub fn secure_input_active() -> bool {
        false
    }
}
