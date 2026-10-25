//! Name of the AirPlay output the system is sending audio/video to, read from
//! CoreAudio's default output device. AVPlayer exposes no device name of its own.
#![allow(unsafe_code)]
use std::os::raw::{c_char, c_void};

type OSStatus = i32;
type AudioObjectID = u32;

#[repr(C)]
struct AudioObjectPropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioObjectGetPropertyData(
        object: AudioObjectID,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
        data: *mut c_void,
    ) -> OSStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *const c_void);
    fn CFStringGetCStringPtr(s: *const c_void, encoding: u32) -> *const c_char;
    fn CFStringGetLength(s: *const c_void) -> isize;
    fn CFStringGetCString(s: *const c_void, buf: *mut c_char, size: isize, encoding: u32) -> u8;
}

const SYSTEM_OBJECT: AudioObjectID = 1;
const SCOPE_GLOBAL: u32 = fourcc(b"glob");
const ELEMENT_MAIN: u32 = 0;
const PROP_DEFAULT_OUTPUT: u32 = fourcc(b"dOut");
const PROP_TRANSPORT_TYPE: u32 = fourcc(b"tran");
const PROP_NAME: u32 = fourcc(b"lnam");
const TRANSPORT_AIRPLAY: u32 = fourcc(b"airp");
const UTF8: u32 = 0x0800_0100;

const fn fourcc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | (s[3] as u32)
}

unsafe fn get<T: Default>(obj: AudioObjectID, selector: u32) -> Option<T> {
    let addr = AudioObjectPropertyAddress { selector, scope: SCOPE_GLOBAL, element: ELEMENT_MAIN };
    let mut size = std::mem::size_of::<T>() as u32;
    let mut out = T::default();
    let st = AudioObjectGetPropertyData(
        obj,
        &addr,
        0,
        std::ptr::null(),
        &mut size,
        &mut out as *mut T as *mut c_void,
    );
    (st == 0).then_some(out)
}

/// The name of the default output device when it is an AirPlay device, e.g. "Living Room Apple TV".
/// `None` when the output is local (speakers, headphones) or the lookup fails.
pub fn airplay_output_name() -> Option<String> {
    unsafe {
        let dev: AudioObjectID = get(SYSTEM_OBJECT, PROP_DEFAULT_OUTPUT)?;
        let transport: u32 = get(dev, PROP_TRANSPORT_TYPE)?;
        if transport != TRANSPORT_AIRPLAY {
            return None;
        }
        // The name is a CFStringRef the caller owns (released below).
        let name = get::<usize>(dev, PROP_NAME)? as *const c_void;
        if name.is_null() {
            return None;
        }
        let s = cf_string(name);
        CFRelease(name);
        s.filter(|s| !s.trim().is_empty())
    }
}

unsafe fn cf_string(s: *const c_void) -> Option<String> {
    let direct = CFStringGetCStringPtr(s, UTF8);
    if !direct.is_null() {
        return Some(std::ffi::CStr::from_ptr(direct).to_string_lossy().into_owned());
    }
    let len = CFStringGetLength(s);
    let mut buf = vec![0 as c_char; (len as usize) * 4 + 1];
    if CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, UTF8) == 0 {
        return None;
    }
    Some(std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_codes_match_coreaudio_constants() {
        assert_eq!(PROP_DEFAULT_OUTPUT, 0x644F_7574); // 'dOut'
        assert_eq!(TRANSPORT_AIRPLAY, 0x6169_7270); // 'airp'
        assert_eq!(PROP_NAME, 0x6C6E_616D); // 'lnam'
    }
}
