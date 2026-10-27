//! Fail-closed guest-only Glutin/winit display selection diagnostic.
//! Uses the same compiled rlibs as Starbyte; opens no GUI surface.
use std::env;

use glutin::config::ConfigTemplateBuilder;
use glutin::display::{Display, DisplayApiPreference};
use glutin::error::ErrorKind;
use glutin::prelude::GlDisplay;
use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};

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
