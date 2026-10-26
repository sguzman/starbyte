//! Compare Mesa EGL attached Wayland displays with separately connected EGL.
//! Only allowed inside the disposable private Starbyte QEMU guest.
use std::env;
use std::ffi::{c_char, c_void, CStr};
use std::ptr;

const WAYLAND: u32 = 0x31D8; // EGL_PLATFORM_WAYLAND_EXT
const SURFACELESS: u32 = 0x31DD; // EGL_PLATFORM_SURFACELESS_MESA
const EGL_SUCCESS: i32 = 0x3000;
const EGL_NONE: isize = 0x3038;
const EGL_TRACK_REFERENCES_KHR: isize = 0x3352;
const EGL_EXTENSIONS: i32 = 0x3055;

#[link(name = "EGL")]
unsafe extern "C" {
    fn eglGetPlatformDisplay(platform: u32, native: *mut c_void, attrs: *const isize) -> *mut c_void;
    fn eglInitialize(display: *mut c_void, major: *mut i32, minor: *mut i32) -> u32;
    fn eglTerminate(display: *mut c_void) -> u32;
    fn eglGetError() -> i32;
    fn eglQueryString(display: *mut c_void, name: i32) -> *const c_char;
}

#[link(name = "wayland-client")]
unsafe extern "C" {
    fn wl_display_connect(name: *const c_char) -> *mut c_void;
    fn wl_display_disconnect(display: *mut c_void);
}

fn attempt_with_attrs(platform: u32, native: *mut c_void, attrs: *const isize) -> (bool, i32) {
    // A native pointer is supplied only by the private guest's wl_display_connect.
    unsafe {
        let display = eglGetPlatformDisplay(platform, native, attrs);
        if display.is_null() {
            return (false, eglGetError());
        }
        let mut major = 0_i32;
        let mut minor = 0_i32;
        let initialized = eglInitialize(display, &mut major, &mut minor) != 0;
        let code = if initialized { EGL_SUCCESS } else { eglGetError() };
        if initialized {
            let _ = eglTerminate(display);
        }
        (initialized, code)
    }
}

fn attempt(platform: u32, native: *mut c_void) -> (bool, i32) {
    attempt_with_attrs(platform, native, ptr::null())
}

fn track_references_advertised() -> bool {
    // Glutin checks the client extension string before adding this attribute.
    unsafe {
        let extensions = eglQueryString(ptr::null_mut(), EGL_EXTENSIONS);
        !extensions.is_null()
            && CStr::from_ptr(extensions)
                .to_bytes()
                .split(|b| *b == b' ')
                .any(|name| name == b"EGL_KHR_display_reference")
    }
}

fn main() {
    // Fail closed when invoked outside the disposable guest image.
    if env::var("STARBYTE_GUEST_EGL_PROBE").as_deref() != Ok("1")
        || env::var("XDG_RUNTIME_DIR").as_deref() != Ok("/run/user/0")
    {
        println!("STARBYTE_GUEST_EGL_ATTACHED_BLOCKED private_guest_only");
        std::process::exit(2);
    }
    // Do not open host sockets. The runner's guest namespace has its own
    // XDG_RUNTIME_DIR and Wayland socket.
    let connected_display = unsafe { wl_display_connect(ptr::null()) };
    if connected_display.is_null() {
        println!("STARBYTE_GUEST_EGL_ATTACHED_RESULT connected=0 attached=0 own=0 surfaceless=0 attached_error=0x3001 own_error=0x3001 surfaceless_error=0x3001");
        return;
    }
    let track_advertised = track_references_advertised();
    // glutin 0.32 optionally sets EGL_TRACK_REFERENCES_KHR=EGL_TRUE;
    // unlike eglinfo and the previous probe, which use a NULL attribute list.
    // Run tracked first so a prior initialized display cannot mask its failure.
    let tracked_attrs = [EGL_TRACK_REFERENCES_KHR, 1, EGL_NONE];
    let (tracked, tracked_error) = if track_advertised {
        attempt_with_attrs(WAYLAND, connected_display, tracked_attrs.as_ptr())
    } else {
        (false, EGL_SUCCESS)
    };
    // Match the non-null EGL_NONE attribute list used by Glutin 0.32.
    let none_attrs = [EGL_NONE];
    let (egl_none, egl_none_error) =
        attempt_with_attrs(WAYLAND, connected_display, none_attrs.as_ptr());
    let (attached, attached_error) = attempt(WAYLAND, connected_display);
    let (own, own_error) = attempt(WAYLAND, ptr::null_mut());
    let (surfaceless, surfaceless_error) = attempt(SURFACELESS, ptr::null_mut());
    unsafe { wl_display_disconnect(connected_display) };
    println!(
        "STARBYTE_GUEST_EGL_NONE_RESULT initialized={} error=0x{:04x}",
        u8::from(egl_none),
        egl_none_error,
    );
    println!(
        "STARBYTE_GUEST_EGL_TRACKED_RESULT advertised={} initialized={} error=0x{:04x}",
        u8::from(track_advertised),
        u8::from(tracked),
        tracked_error,
    );
    println!(
        "STARBYTE_GUEST_EGL_ATTACHED_RESULT connected=1 attached={} own={} surfaceless={} attached_error=0x{:04x} own_error=0x{:04x} surfaceless_error=0x{:04x}",
        u8::from(attached),
        u8::from(own),
        u8::from(surfaceless),
        attached_error,
        own_error,
        surfaceless_error,
    );
}
