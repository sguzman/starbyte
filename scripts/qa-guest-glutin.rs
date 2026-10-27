//! Fail-closed guest-only Glutin/winit display selection diagnostic.
//! Uses the same compiled rlibs as Starbyte; opens no GUI surface.
use std::{env, ffi::c_void};

use glutin::config::ConfigTemplateBuilder;
use glutin::display::{Display, DisplayApiPreference};
use glutin::error::ErrorKind;
use glutin::prelude::GlDisplay;
use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};

const EGL_WAYLAND_PLATFORM: u32 = 0x31D8;
const EGL_NONE: isize = 0x3038;
const EGL_SUCCESS: i32 = 0x3000;

#[link(name = "EGL")]
unsafe extern "C" {
    fn eglGetPlatformDisplay(platform: u32, native: *mut c_void, attributes: *const isize) -> *mut c_void;
    fn eglInitialize(display: *mut c_void, major: *mut i32, minor: *mut i32) -> u32;
    fn eglTerminate(display: *mut c_void) -> u32;
    fn eglGetError() -> i32;
}

#[link(name = "wayland-client")]
unsafe extern "C" {
    fn wl_display_connect(name: *const std::ffi::c_char) -> *mut c_void;
    fn wl_display_disconnect(display: *mut c_void);
    fn wl_display_get_fd(display: *mut c_void) -> i32;
    fn wl_display_get_error(display: *mut c_void) -> i32;
}

/// Try the exact Wayland pointer that winit handed to Glutin, using the
/// same raw EGL 1.5 entrypoint as the independently passing guest probe.
/// This runs only after Glutin has already failed, so it cannot mask that
/// error by preinitializing Mesa state. No pointer value leaves guest RAM.
fn raw_egl_winit_display(handle: RawDisplayHandle, before_glutin: bool) {
    let RawDisplayHandle::Wayland(wayland) = handle else {
        return;
    };
    unsafe {
        // Validate the native-display handle before calling EGL in this guest.
        // Only publish booleans, never file descriptors or addresses.
        let fd = wl_display_get_fd(wayland.display.as_ptr());
        let error = wl_display_get_error(wayland.display.as_ptr());
        println!(
            "STARBYTE_GUEST_GLUTIN_WINIT_CONN_RESULT fd_ok={} error_clear={}",
            u8::from(fd >= 0), u8::from(error == 0),
        );
        let attrs = [EGL_NONE];
        let display = eglGetPlatformDisplay(
            EGL_WAYLAND_PLATFORM, wayland.display.as_ptr(), attrs.as_ptr()
        );
        let (initialized, error) = if display.is_null() {
            (false, eglGetError())
        } else {
            let mut major = 0;
            let mut minor = 0;
            if eglInitialize(display, &mut major, &mut minor) == 0 {
                (false, eglGetError())
            } else {
                let _ = eglTerminate(display);
                (true, EGL_SUCCESS)
            }
        };
        let label = if before_glutin {
            "STARBYTE_GUEST_GLUTIN_BEFORE_RESULT"
        } else {
            "STARBYTE_GUEST_GLUTIN_RAW_EGL_RESULT"
        };
        println!(
            "{label} initialized={} error=0x{:04x}",
            u8::from(initialized), error,
        );
    }
}

/// Compare against an independent libwayland connection to the SAME guest
/// compositor in this process, after Glutin's failure. Distinguishes the
/// winit-provided display handle from process-wide EGL initialization issues.
/// Never prints the socket name or either pointer.
fn fresh_c_connection_after_glutin_failure() {
    unsafe {
        let connection = wl_display_connect(std::ptr::null());
        if connection.is_null() {
            println!("STARBYTE_GUEST_GLUTIN_C_EGL_RESULT connected=0 initialized=0 error=0x3001");
            return;
        }
        let fd = wl_display_get_fd(connection);
        let error = wl_display_get_error(connection);
        println!(
            "STARBYTE_GUEST_GLUTIN_C_CONN_RESULT fd_ok={} error_clear={}",
            u8::from(fd >= 0), u8::from(error == 0),
        );
        let attrs = [EGL_NONE];
        let display = eglGetPlatformDisplay(EGL_WAYLAND_PLATFORM, connection, attrs.as_ptr());
        let (initialized, error) = if display.is_null() {
            (false, eglGetError())
        } else {
            let mut major = 0;
            let mut minor = 0;
            if eglInitialize(display, &mut major, &mut minor) == 0 {
                (false, eglGetError())
            } else {
                let _ = eglTerminate(display);
                (true, EGL_SUCCESS)
            }
        };
        wl_display_disconnect(connection);
        println!(
            "STARBYTE_GUEST_GLUTIN_C_EGL_RESULT connected=1 initialized={} error=0x{:04x}",
            u8::from(initialized), error,
        );
    }
}

fn result(eventloop: bool, wayland: bool, display: bool, config: bool, reason: &str) {
    println!(
        "STARBYTE_GUEST_GLUTIN_RESULT eventloop={} wayland={} display={} config={} reason={reason}",
        u8::from(eventloop),
        u8::from(wayland),
        u8::from(display),
        u8::from(config)
    );
}

fn main() {
    // This executable is staged solely in an isolated, disposable VM image.
    if env::var("STARBYTE_GUEST_GLUTIN_PROBE").as_deref() != Ok("1")
        || env::var("XDG_RUNTIME_DIR").as_deref() != Ok("/run/user/0")
        || !env::var("WAYLAND_DISPLAY")
            .is_ok_and(|name| name.starts_with("wayland-") && !name.contains('/'))
    {
        println!("STARBYTE_GUEST_GLUTIN_BLOCKED private_guest_only");
        std::process::exit(2);
    }

    let event_loop = match winit::event_loop::EventLoop::new() {
        Ok(it) => it,
        Err(_) => {
            result(false, false, false, false, "eventloop");
            return;
        }
    };
    let handle = match event_loop.display_handle() {
        Ok(it) => it.as_raw(),
        Err(_) => {
            result(true, false, false, false, "backend");
            return;
        }
    };
    let is_wayland = matches!(handle, RawDisplayHandle::Wayland(_));
    if !is_wayland {
        result(true, false, false, false, "backend");
        return;
    }
    // Run the independent pre-Glutin control in its own short-lived
    // process: otherwise a successful eglTerminate would disturb Glutin's
    // subsequent use of the shared EGLDisplay and confound the comparison.
    if env::var("STARBYTE_GUEST_GLUTIN_PHASE").as_deref() == Ok("before") {
        raw_egl_winit_display(handle, true);
        return;
    }
    // The Wayland raw handle is valid for the whole event_loop lifetime.
    let display = match unsafe { Display::new(handle, DisplayApiPreference::Egl) } {
        Ok(it) => it,
        Err(error) => {
            let reason = match error.error_kind() {
                ErrorKind::InitializationFailed => "egl_init",
                ErrorKind::NotSupported(_) => "display_unsupported",
                _ => "display_other",
            };
            result(true, true, false, false, reason);
            raw_egl_winit_display(handle, false);
            fresh_c_connection_after_glutin_failure();
            return;
        }
    };
    let configs = unsafe { display.find_configs(ConfigTemplateBuilder::new().build()) };
    match configs {
        Ok(mut configs) => {
            if configs.next().is_some() {
                result(true, true, true, true, "none");
            } else {
                result(true, true, true, false, "config_empty");
            }
        }
        Err(_) => result(true, true, true, false, "config_other"),
    }
}
