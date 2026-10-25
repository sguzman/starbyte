//! GBM device/buffer allocation inside an ephemeral Linux guest, not host DRM.
use std::ffi::c_void;
use std::fs::OpenOptions;
use std::os::fd::AsRawFd;

#[link(name = "gbm")]
unsafe extern "C" {
    fn gbm_create_device(fd: i32) -> *mut c_void;
    fn gbm_device_destroy(device: *mut c_void);
    fn gbm_bo_create(
        device: *mut c_void,
        width: u32,
        height: u32,
        format: u32,
        flags: u32,
    ) -> *mut c_void;
    fn gbm_bo_destroy(buffer: *mut c_void);
}

fn main() {
    println!("STARBYTE_GUEST_GBM_PROCESS_STARTED");
    let node = std::env::args().nth(1).unwrap_or_default();
    if !node.starts_with("/dev/dri/renderD") {
        println!("STARBYTE_GUEST_GBM_RESULT device=0 buffer=0");
        return;
    }
    let Ok(file) = OpenOptions::new().read(true).write(true).open(node) else {
        println!("STARBYTE_GUEST_GBM_RESULT device=0 buffer=0");
        return;
    };
    println!("STARBYTE_GUEST_GBM_NODE_OPEN_OK");
    // SAFETY: The file descriptor outlives the device and any GBM buffer.
    let device = unsafe { gbm_create_device(file.as_raw_fd()) };
    if device.is_null() {
        println!("STARBYTE_GUEST_GBM_RESULT device=0 buffer=0");
        return;
    }
    // DRM_FORMAT_XRGB8888, GBM_BO_USE_RENDERING: 64 x 64 render buffer.
    let buffer = unsafe { gbm_bo_create(device, 64, 64, 0x3432_5258, 1 << 2) };
    let allocated = !buffer.is_null();
    if allocated {
        // SAFETY: Buffer is owned by the valid device.
        unsafe { gbm_bo_destroy(buffer) };
    }
    // SAFETY: All buffers were destroyed before their device.
    unsafe { gbm_device_destroy(device) };
    println!(
        "STARBYTE_GUEST_GBM_RESULT device=1 buffer={}",
        u8::from(allocated)
    );
}
