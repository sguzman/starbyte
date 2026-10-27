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
    fn eglGetProcAddress(name: *const c_char) -> *const c_void;
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

fn client_extension_advertised(name: &[u8]) -> bool {
    // Glutin selects the core KHR or legacy EXT Wayland display API
    // according to exactly these EGL client-extension names.
    unsafe {
        let extensions = eglQueryString(ptr::null_mut(), EGL_EXTENSIONS);
        !extensions.is_null()
            && CStr::from_ptr(extensions)
                .to_bytes()
                .split(|b| *b == b' ')
                .any(|item| item == name)
    }
}

/// Compare the legacy EGL_EXT_platform_base route with core EGL 1.5.
/// Glutin may choose this route if the KHR client extension is absent.
/// Resolve the extension entrypoint at runtime: linking directly against
/// the optional symbol would cause an unrelated loader error.
fn attempt_ext_wayland(native: *mut c_void) -> (bool, bool, i32) {
    type PlatformDisplayExt =
        unsafe extern "C" fn(u32, *mut c_void, *const i32) -> *mut c_void;
    unsafe {
        let symbol = eglGetProcAddress(c"eglGetPlatformDisplayEXT".as_ptr());
        if symbol.is_null() {
            return (false, false, EGL_SUCCESS);
        }
        let get_display: PlatformDisplayExt = std::mem::transmute(symbol);
        let attrs = [EGL_NONE as i32];
        let display = get_display(WAYLAND, native, attrs.as_ptr());
        if display.is_null() {
            return (true, false, eglGetError());
        }
        let mut major = 0_i32;
        let mut minor = 0_i32;
        let initialized = eglInitialize(display, &mut major, &mut minor) != 0;
        let error = if initialized { EGL_SUCCESS } else { eglGetError() };
        if initialized {
            let _ = eglTerminate(display);
        }
        (true, initialized, error)
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
    let track_advertised = client_extension_advertised(b"EGL_KHR_display_reference");
    let khr_wayland = client_extension_advertised(b"EGL_KHR_platform_wayland");
    let ext_wayland = client_extension_advertised(b"EGL_EXT_platform_wayland");
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
    let (ext_symbol, ext_initialized, ext_error) = if ext_wayland {
        attempt_ext_wayland(connected_display)
    } else {
        (false, false, EGL_SUCCESS)
    };
    let (attached, attached_error) = attempt(WAYLAND, connected_display);
    let (own, own_error) = attempt(WAYLAND, ptr::null_mut());
    let (surfaceless, surfaceless_error) = attempt(SURFACELESS, ptr::null_mut());
    unsafe { wl_display_disconnect(connected_display) };
    println!(
        "STARBYTE_GUEST_EGL_EXTENSION_DIAG khr_wayland={} ext_wayland={}",
        u8::from(khr_wayland),
        u8::from(ext_wayland),
    );
    println!(
        "STARBYTE_GUEST_EGL_EXT_RESULT advertised={} symbol={} initialized={} error=0x{:04x}",
        u8::from(ext_wayland),
        u8::from(ext_symbol),
        u8::from(ext_initialized),
        ext_error,
    );
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
