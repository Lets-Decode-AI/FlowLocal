//! Physical key-state probes (Windows / X11). The hotkey tracker only sees key events, so a
//! release that goes elsewhere (Win+L, Alt+Tab, a focus-stealing dialog...) leaves a modifier
//! "stuck down" in its bookkeeping. Before a chord is reported, the other keys are checked
//! against the OS so such stale keys are discarded.
//!
//! The key-code tables are pure and unit-tested; only the thin FFI shims at the bottom are
//! platform specific.
use super::Key;

/// Windows virtual-key code of `key` (`VK_LCONTROL` ...). `Fn` has none.
pub fn windows_vk(key: Key) -> Option<u16> {
    Some(match key {
        Key::ControlLeft => 0xA2,
        Key::ControlRight => 0xA3,
        Key::MetaLeft => 0x5B,
        Key::MetaRight => 0x5C,
        Key::AltLeft => 0xA4,
        Key::AltRight => 0xA5,
        Key::ShiftLeft => 0xA0,
        Key::ShiftRight => 0xA1,
        Key::Space => 0x20,
        Key::Escape => 0x1B,
        Key::Fn => return None,
    })
}

/// X11 keycode of `key`: the same numbers `rdev` maps its Linux events from.
pub fn x11_keycode(key: Key) -> Option<u8> {
    Some(match key {
        Key::ControlLeft => 37,
        Key::ControlRight => 105,
        Key::MetaLeft => 133,
        Key::MetaRight => 134,
        Key::AltLeft => 64,
        Key::AltRight => 108,
        Key::ShiftLeft => 50,
        Key::ShiftRight => 62,
        Key::Space => 65,
        Key::Escape => 9,
        Key::Fn => return None,
    })
}

/// Bit for `keycode` in the 32-byte bitmap `XQueryKeymap` fills in.
pub fn keymap_bit(keymap: &[u8; 32], keycode: u8) -> bool {
    keymap[usize::from(keycode / 8)] & (1 << (keycode % 8)) != 0
}

/// Is `key` physically down right now? `true` whenever that cannot be determined, so a probe
/// failure never blocks the hotkey.
#[cfg(target_os = "windows")]
pub fn is_down(key: Key) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    let Some(vk) = windows_vk(key) else {
        return true;
    };
    // SAFETY: plain call with a value argument. The high bit of the result (negative i16)
    // means "down".
    unsafe { GetAsyncKeyState(i32::from(vk)) < 0 }
}

/// See the Windows variant.
#[cfg(target_os = "linux")]
pub fn is_down(key: Key) -> bool {
    use std::os::raw::c_char;
    use x11::xlib::{XCloseDisplay, XOpenDisplay, XQueryKeymap};
    let Some(code) = x11_keycode(key) else {
        return true;
    };
    // SAFETY: the display is checked for null, `raw` is the 32-byte buffer XQueryKeymap
    // requires, and the display is closed before returning.
    unsafe {
        let dpy = XOpenDisplay(std::ptr::null());
        if dpy.is_null() {
            return true;
        }
        let mut raw = [0 as c_char; 32];
        XQueryKeymap(dpy, raw.as_mut_ptr());
        XCloseDisplay(dpy);
        keymap_bit(&raw.map(|b| b as u8), code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_cover_every_key_but_fn() {
        for k in [
            Key::ControlLeft,
            Key::ControlRight,
            Key::MetaLeft,
            Key::MetaRight,
            Key::AltLeft,
            Key::AltRight,
            Key::ShiftLeft,
            Key::ShiftRight,
            Key::Space,
            Key::Escape,
        ] {
            assert!(windows_vk(k).is_some() && x11_keycode(k).is_some(), "{k:?}");
        }
        assert_eq!((windows_vk(Key::Fn), x11_keycode(Key::Fn)), (None, None));
        // Spot checks against VK_LWIN / VK_LCONTROL and rdev's Linux table.
        assert_eq!(windows_vk(Key::MetaLeft), Some(0x5B));
        assert_eq!(windows_vk(Key::ControlLeft), Some(0xA2));
        assert_eq!(x11_keycode(Key::MetaLeft), Some(133));
        assert_eq!(x11_keycode(Key::ControlLeft), Some(37));
    }

    #[test]
    fn keymap_bit_indexes_the_x11_bitmap() {
        let mut map = [0u8; 32];
        map[4] = 0b0010_0000; // keycode 37 = byte 4, bit 5
        assert!(keymap_bit(&map, 37));
        assert!(!keymap_bit(&map, 36));
        assert!(!keymap_bit(&map, 133));
        map[16] = 0b0010_0000; // keycode 133 = byte 16, bit 5
        assert!(keymap_bit(&map, 133));
    }
}
