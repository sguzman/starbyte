//! GBM allocation in an isolated guest. Only virtual /dev/dri nodes allowed.
use std::ffi::c_void;
use std::fs::OpenOptions;
use std::os::fd::AsRawFd;

#[link(name = "gbm")]
unsafe extern "C" {
    fn gbm_create_device(fd: i32) -> *mut c_void;
    fn gbm_device_destroy(device: *mut c_void);
    fn gbm_bo_create(device: *mut c_void, width: u32, height: u32, format: u32, flags: u32) -> *mut c_void;
    fn gbm_bo_destroy(buffer: *mut c_void);
}

fn main() {
    let node = std::env::args().nth(1).unwrap_or_default();
    let kind = if node.starts_with("/dev/dri/renderD") {
        "render"
    } else if node.starts_with("/dev/dri/card") {
        "card"
    } else {
        "invalid"
    };
    println!("STARBYTE_GUEST_GBM_PROCESS_STARTED node={kind}");
    if kind == "invalid" {
        println!("STARBYTE_GUEST_GBM_RESULT node={kind} device=0 buffer=0");
        return;
    }
    let Ok(file) = OpenOptions::new().read(true).write(true).open(node) else {
        println!("STARBYTE_GUEST_GBM_RESULT node={kind} device=0 buffer=0");
        return;
    };
    println!("STARBYTE_GUEST_GBM_NODE_OPEN_OK node={kind}");
    // SAFETY: The fd remains open until device and buffer destruction.
    let device = unsafe { gbm_create_device(file.as_raw_fd()) };
    if device.is_null() {
        println!("STARBYTE_GUEST_GBM_RESULT node={kind} device=0 buffer=0");
        return;
    }
    // DRM_FORMAT_XRGB8888 + GBM_BO_USE_RENDERING, guest buffer only.
    let bo = unsafe { gbm_bo_create(device, 64, 64, 0x3432_5258, 1 << 2) };
    let allocated = !bo.is_null();
    if allocated {
        // SAFETY: GBM returned a buffer owned by this live device.
        unsafe { gbm_bo_destroy(bo) };
    }
    // SAFETY: No buffer survives beyond the device.
    unsafe { gbm_device_destroy(device) };
    println!(
        "STARBYTE_GUEST_GBM_RESULT node={kind} device=1 buffer={}",
        u8::from(allocated)
    );
}
