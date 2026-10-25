//! Validate guest Wayland protocol registry via the native Unix wire format.
//! No host Wayland socket, compositor, filesystem mounts or GPU are accessed.
//! The caller must restrict XDG_RUNTIME_DIR to the private disposable guest.
use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

const REGISTRY_ID: u32 = 2;
const SYNC_ID: u32 = 3;

fn request(object: u32, opcode: u16, new_id: u32) -> [u8; 12] {
    let mut bytes = [0_u8; 12];
    bytes[..4].copy_from_slice(&object.to_ne_bytes());
    bytes[4..8].copy_from_slice(&((12_u32 << 16) | u32::from(opcode)).to_ne_bytes());
    bytes[8..12].copy_from_slice(&new_id.to_ne_bytes());
    bytes
}

fn run() -> Result<(bool, bool), &'static str> {
    let runtime = env::var_os("XDG_RUNTIME_DIR").ok_or("missing_runtime")?;
    let display = env::var("WAYLAND_DISPLAY").map_err(|_| "missing_display")?;
    if display.contains('/') || display.contains("..") || display.is_empty() {
        return Err("unsafe_display_name");
    }
    let path = PathBuf::from(runtime).join(display);
    let mut stream = UnixStream::connect(path).map_err(|_| "connect_failed")?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|_| "timeout_setup_failed")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|_| "timeout_setup_failed")?;

    // wl_display.get_registry(new_id=2), then wl_display.sync(new_id=3).
    // The wl_callback.done event provides an unambiguous registry barrier.
    stream.write_all(&request(1, 1, REGISTRY_ID)).map_err(|_| "write_failed")?;
    stream.write_all(&request(1, 0, SYNC_ID)).map_err(|_| "write_failed")?;

    let mut compositor = false;
    let mut seat = false;
    for _ in 0..256 {
        let mut header = [0_u8; 8];
        stream.read_exact(&mut header).map_err(|_| "read_failed")?;
        let sender = u32::from_ne_bytes(header[0..4].try_into().unwrap());
        let word = u32::from_ne_bytes(header[4..8].try_into().unwrap());
        let length = (word >> 16) as usize;
        let opcode = (word & 0xffff) as u16;
        if !(8..=65535).contains(&length) {
            return Err("invalid_frame_length");
        }
        let mut payload = vec![0_u8; length - 8];
        stream.read_exact(&mut payload).map_err(|_| "payload_failed")?;

        if sender == 1 && opcode == 0 {
            return Err("display_protocol_error");
        }
        if sender == REGISTRY_ID && opcode == 0 && payload.len() >= 8 {
            let interface_length =
                u32::from_ne_bytes(payload[4..8].try_into().unwrap()) as usize;
            if interface_length == 0 || interface_length > payload.len() - 8 {
                return Err("invalid_registry_global");
            }
            let name = &payload[8..8 + interface_length];
            compositor |= name == b"wl_compositor\0";
            seat |= name == b"wl_seat\0";
        }
        if sender == SYNC_ID && opcode == 0 {
            return Ok((compositor, seat));
        }
    }
    Err("registry_event_limit")
}

fn main() {
    match run() {
        Ok((compositor, seat)) => {
            println!(
                "STARBYTE_GUEST_WAYLAND_WIRE_RESULT connected=1 registry_done=1 compositor={} seat={}",
                u8::from(compositor),
                u8::from(seat)
            );
            if !compositor || !seat {
                std::process::exit(2);
            }
        }
        Err(reason) => {
            println!("STARBYTE_GUEST_WAYLAND_WIRE_BLOCKED {reason}");
            std::process::exit(2);
        }
    }
}
