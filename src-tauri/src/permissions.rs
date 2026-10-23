//! OS permission probes (macOS Accessibility + Microphone). Other platforms need none.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Perm {
    Granted,
    Denied,
    Unknown,
    NotRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PermissionsStatus {
    pub microphone: Perm,
    pub accessibility: Perm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermKind {
    Microphone,
    Accessibility,
    /// Keyboard pane, for the Globe key setting.
    Keyboard,
}

/// `AVAuthorizationStatus` raw value -> [`Perm`].
pub fn map_av_status(raw: isize) -> Perm {
    match raw {
        3 => Perm::Granted,
        1 | 2 => Perm::Denied,
        _ => Perm::Unknown,
    }
}

pub fn status() -> PermissionsStatus {
    PermissionsStatus {
        microphone: microphone(),
        accessibility: accessibility(),
    }
}

/// The OS has explicitly denied (or restricted) microphone access.
pub fn microphone_denied() -> bool {
    microphone() == Perm::Denied
}

pub fn settings_url(kind: PermKind) -> &'static str {
    match kind {
        PermKind::Microphone => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
        }
        PermKind::Accessibility => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        PermKind::Keyboard => "x-apple.systempreferences:com.apple.Keyboard-Settings.extension",
    }
}

/// Opens the relevant system settings pane. For Accessibility on macOS this also raises the
/// system trust prompt (`AXIsProcessTrustedWithOptions`).
pub fn open_settings(kind: PermKind) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if kind == PermKind::Accessibility {
            mac::prompt_accessibility();
        }
        std::process::Command::new("open")
            .arg(settings_url(kind))
            .status() // waits for `open` to exit (it returns at once), so no zombie is left
            .map_err(|e| e.to_string())
            .and_then(|s| {
                s.success()
                    .then_some(())
                    .ok_or_else(|| format!("`open` failed: {s}"))
            })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = kind;
        Ok(())
    }
}

/// Shows the system microphone prompt if the user has not decided yet.
pub fn request_microphone() {
    #[cfg(target_os = "macos")]
    mac::request_microphone();
}

#[cfg(target_os = "macos")]
fn microphone() -> Perm {
    mac::microphone()
}
#[cfg(not(target_os = "macos"))]
fn microphone() -> Perm {
    Perm::NotRequired
}

#[cfg(target_os = "macos")]
fn accessibility() -> Perm {
    mac::accessibility()
}
#[cfg(not(target_os = "macos"))]
fn accessibility() -> Perm {
    Perm::NotRequired
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{Perm, map_av_status};
    use block2::RcBlock;
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::string::CFString;
    use objc2::runtime::Bool;
    use objc2_av_foundation::{AVCaptureDevice, AVMediaTypeAudio};

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    }

    pub fn accessibility() -> Perm {
        // SAFETY: plain C call without arguments.
        if unsafe { AXIsProcessTrusted() } {
            Perm::Granted
        } else {
            Perm::Denied
        }
    }

    pub fn prompt_accessibility() {
        let key = CFString::from_static_string("AXTrustedCheckOptionPrompt");
        let opts = CFDictionary::from_CFType_pairs(&[(
            key.as_CFType(),
            CFBoolean::true_value().as_CFType(),
        )]);
        // SAFETY: `opts` is a valid CFDictionary that outlives the call.
        unsafe { AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef()) };
    }

    pub fn microphone() -> Perm {
        // SAFETY: AVMediaTypeAudio is a valid media type constant.
        unsafe {
            let Some(audio) = AVMediaTypeAudio else {
                return Perm::Unknown;
            };
            map_av_status(AVCaptureDevice::authorizationStatusForMediaType(audio).0)
        }
    }

    pub fn request_microphone() {
        // SAFETY: the block is copied by AVFoundation; AVMediaTypeAudio is valid.
        unsafe {
            let Some(audio) = AVMediaTypeAudio else {
                return;
            };
            let handler = RcBlock::new(|_granted: Bool| {});
            AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &handler);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn av_status_mapping() {
        assert_eq!(map_av_status(0), Perm::Unknown);
        assert_eq!(map_av_status(1), Perm::Denied);
        assert_eq!(map_av_status(2), Perm::Denied);
        assert_eq!(map_av_status(3), Perm::Granted);
    }

    #[test]
    fn perm_serializes_camel_case() {
        let v = serde_json::to_value(PermissionsStatus {
            microphone: Perm::NotRequired,
            accessibility: Perm::Granted,
        })
        .unwrap();
        assert_eq!(v["microphone"], "notRequired");
        assert_eq!(v["accessibility"], "granted");
    }

    #[test]
    fn settings_urls_match_brief() {
        assert!(settings_url(PermKind::Microphone).ends_with("?Privacy_Microphone"));
        assert!(settings_url(PermKind::Accessibility).ends_with("?Privacy_Accessibility"));
        assert_eq!(
            settings_url(PermKind::Keyboard),
            "x-apple.systempreferences:com.apple.Keyboard-Settings.extension"
        );
    }

    #[test]
    fn keyboard_kind_deserializes_from_the_ipc_name() {
        let k: PermKind = serde_json::from_str("\"keyboard\"").unwrap();
        assert_eq!(k, PermKind::Keyboard);
    }
}
