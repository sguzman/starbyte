//! Compare Mesa EGL attached Wayland displays with separately connected EGL.
//! Only allowed inside the disposable private Starbyte QEMU guest.
use std::env;
use std::ffi::{c_char, c_void};
use std::ptr;

const WAYLAND: u32 = 0x31D8; // EGL_PLATFORM_WAYLAND_EXT
const SURFACELESS: u32 = 0x31DD; // EGL_PLATFORM_SURFACELESS_MESA
const EGL_SUCCESS: i32 = 0x3000;

#[link(name = "EGL")]
unsafe extern "C" {
    fn eglGetPlatformDisplay(platform: u32, native: *mut c_void, attrs: *const isize) -> *mut c_void;
    fn eglInitialize(display: *mut c_void, major: *mut i32, minor: *mut i32) -> u32;
    fn eglTerminate(display: *mut c_void) -> u32;
    fn eglGetError() -> i32;
}

#[link(name = "wayland-client")]
unsafe extern "C" {
    fn wl_display_connect(name: *const c_char) -> *mut c_void;
    fn wl_display_disconnect(display: *mut c_void);
}

fn attempt(platform: u32, native: *mut c_void) -> (bool, i32) {
    // A native pointer is supplied only by the private guest's wl_display_connect.
    unsafe {
        let display = eglGetPlatformDisplay(platform, native, ptr::null());
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
    let (attached, attached_error) = attempt(WAYLAND, connected_display);
    let (own, own_error) = attempt(WAYLAND, ptr::null_mut());
    let (surfaceless, surfaceless_error) = attempt(SURFACELESS, ptr::null_mut());
    unsafe { wl_display_disconnect(connected_display) };
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
