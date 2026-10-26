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

fn run() -> Result<(bool, bool, u32), &'static str> {
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
    let mut globals = 0_u32;
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
            globals = globals.saturating_add(1);
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
            return Ok((compositor, seat, globals));
        }
    }
    Err("registry_event_limit")
}


// Inspect the *actual* wl_shm formats and wl_seat capabilities, not merely
// global names. All traffic is confined to the caller-provided private socket.
// wl_shm.format and wl_seat.capabilities carry no file descriptors.
fn capabilities() -> Result<(bool, bool, bool, u32), &'static str> {
    let runtime = env::var_os("XDG_RUNTIME_DIR").ok_or("missing_runtime")?;
    let display = env::var("WAYLAND_DISPLAY").map_err(|_| "missing_display")?;
    if display.is_empty() || display.contains('/') || display.contains("..") {
        return Err("unsafe_display_name");
    }
    let mut stream =
        UnixStream::connect(PathBuf::from(runtime).join(display)).map_err(|_| "connect_failed")?;
    stream
        .set_read_timeout(Some(Duration::from_secs(8)))
        .map_err(|_| "timeout_setup_failed")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(8)))
        .map_err(|_| "timeout_setup_failed")?;
    stream.write_all(&request(1, 1, REGISTRY_ID)).map_err(|_| "write_failed")?;
    stream.write_all(&request(1, 0, SYNC_ID)).map_err(|_| "write_failed")?;

    fn frame(stream: &mut UnixStream) -> Result<(u32, u16, Vec<u8>), &'static str> {
        let mut header = [0_u8; 8];
        stream.read_exact(&mut header).map_err(|_| "read_failed")?;
        let sender = u32::from_ne_bytes(header[0..4].try_into().unwrap());
        let word = u32::from_ne_bytes(header[4..8].try_into().unwrap());
        let len = (word >> 16) as usize;
        if !(8..=65535).contains(&len) {
            return Err("invalid_frame_length");
        }
        let mut payload = vec![0; len - 8];
        stream.read_exact(&mut payload).map_err(|_| "payload_failed")?;
        Ok((sender, (word & 0xffff) as u16, payload))
    }

    let mut seat_global = None;
    let mut shm_global = None;
    let mut registry_done = false;
    for _ in 0..256 {
        let (sender, opcode, payload) = frame(&mut stream)?;
        if sender == 1 && opcode == 0 {
            return Err("display_protocol_error");
        }
        if sender == REGISTRY_ID && opcode == 0 && payload.len() >= 12 {
            let name_id = u32::from_ne_bytes(payload[0..4].try_into().unwrap());
            let n = u32::from_ne_bytes(payload[4..8].try_into().unwrap()) as usize;
            let next = 8_usize.checked_add(n).ok_or("invalid_registry_global")?;
            let aligned = next.checked_add(3).ok_or("invalid_registry_global")? & !3;
            if n == 0 || aligned + 4 > payload.len() {
                return Err("invalid_registry_global");
            }
            let version = u32::from_ne_bytes(payload[aligned..aligned + 4].try_into().unwrap());
            match &payload[8..next] {
                b"wl_seat\0" => seat_global = Some((name_id, version)),
                b"wl_shm\0" => shm_global = Some((name_id, version)),
                _ => {}
            }
        }
        if sender == SYNC_ID && opcode == 0 {
            registry_done = true;
            break;
        }
    }
    if !registry_done {
        return Err("registry_event_limit");
    }
    let (seat_name, seat_version) = seat_global.ok_or("seat_global_missing")?;
    let (shm_name, shm_version) = shm_global.ok_or("shm_global_missing")?;
    fn bind(stream: &mut UnixStream, global_name: u32, iface: &str, version: u32, id: u32)
        -> Result<(), &'static str>
    {
        let mut payload = Vec::with_capacity(40);
        payload.extend_from_slice(&global_name.to_ne_bytes());
        payload.extend_from_slice(&((iface.len() + 1) as u32).to_ne_bytes());
        payload.extend_from_slice(iface.as_bytes());
        payload.push(0);
        while payload.len() % 4 != 0 { payload.push(0); }
        payload.extend_from_slice(&version.min(1).to_ne_bytes());
        payload.extend_from_slice(&id.to_ne_bytes());
        let length = 8_u32 + payload.len() as u32;
        let mut bytes = Vec::with_capacity(length as usize);
        bytes.extend_from_slice(&REGISTRY_ID.to_ne_bytes());
        bytes.extend_from_slice(&(length << 16).to_ne_bytes());
        bytes.extend_from_slice(&payload);
        stream.write_all(&bytes).map_err(|_| "write_failed")
    }
    const SHM_ID: u32 = 4;
    const SEAT_ID: u32 = 5;
    const CAP_SYNC_ID: u32 = 6;
    bind(&mut stream, shm_name, "wl_shm", shm_version, SHM_ID)?;
    bind(&mut stream, seat_name, "wl_seat", seat_version, SEAT_ID)?;
    stream
        .write_all(&request(1, 0, CAP_SYNC_ID))
        .map_err(|_| "write_failed")?;

    let mut keyboard = false;
    let mut argb = false;
    let mut xrgb = false;
    let mut formats = 0_u32;
    for _ in 0..256 {
        let (sender, opcode, payload) = frame(&mut stream)?;
        if sender == 1 && opcode == 0 {
            return Err("display_protocol_error");
        }
        if sender == SHM_ID && opcode == 0 && payload.len() >= 4 {
            let value = u32::from_ne_bytes(payload[..4].try_into().unwrap());
            formats = formats.saturating_add(1);
            argb |= value == 0; // WL_SHM_FORMAT_ARGB8888
            xrgb |= value == 1; // WL_SHM_FORMAT_XRGB8888
        }
        if sender == SEAT_ID && opcode == 0 && payload.len() >= 4 {
            let caps = u32::from_ne_bytes(payload[..4].try_into().unwrap());
            keyboard = (caps & 2) != 0; // WL_SEAT_CAPABILITY_KEYBOARD
        }
        if sender == CAP_SYNC_ID && opcode == 0 {
            return Ok((keyboard, argb, xrgb, formats));
        }
    }
    Err("capability_event_limit")
}

fn main() {
    let mode = env::args().nth(1);
    if mode.as_deref() == Some("--capabilities") {
        match capabilities() {
            Ok((keyboard, argb, xrgb, formats)) => {
                println!(
                    "STARBYTE_GUEST_WAYLAND_CAP_RESULT keyboard={} shm_argb={} shm_xrgb={} formats={}",
                    u8::from(keyboard), u8::from(argb), u8::from(xrgb), formats
                );
                return;
            }
            Err(reason) => {
                println!("STARBYTE_GUEST_WAYLAND_CAP_BLOCKED {reason}");
                std::process::exit(2);
            }
        }
    }
    if mode.as_deref() == Some("--self-check") {
        let registry = request(1, 1, REGISTRY_ID);
        let sync = request(1, 0, SYNC_ID);
        if registry[4..8] == ((12_u32 << 16) | 1_u32).to_ne_bytes()
            && sync[8..12] == SYNC_ID.to_ne_bytes()
        {
            println!("STARBYTE_GUEST_WAYLAND_WIRE_SELFTEST_OK frames=valid");
            return;
        }
        std::process::exit(2);
    }
    match run() {
        Ok((compositor, seat, globals)) => {
            println!(
                "STARBYTE_GUEST_WAYLAND_WIRE_RESULT connected=1 registry_done=1 compositor={} seat={} globals={}",
                u8::from(compositor),
                u8::from(seat),
                globals
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
