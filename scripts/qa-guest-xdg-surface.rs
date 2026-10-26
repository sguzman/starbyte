//! Isolated XDG toplevel configuration handshake over a private Wayland socket.
//! No buffer, input, GPU or owner's desktop is used.
use std::{env, io::{Read, Write}, os::unix::net::UnixStream, path::PathBuf, time::Duration};

const REGISTRY: u32 = 2;
const REGISTRY_SYNC: u32 = 3;
const COMPOSITOR: u32 = 4;
// Wayland client IDs must be allocated in ascending order. The shell bind
// runs before create_surface, so its ID must precede the surface's ID.
const XDG_SHELL: u32 = 5;
const SURFACE: u32 = 6;
const XDG_SURFACE: u32 = 7;
const XDG_TOPLEVEL: u32 = 8;
const ACK_SYNC: u32 = 9;

// This is the actual new-object request order below, including the initial
// registry and sync callback. Wayland rejects gaps in client object IDs.
// Fail the guest probe at compilation if someone reintroduces one.
const _: () = assert!(
    REGISTRY == 2
        && REGISTRY_SYNC == REGISTRY + 1
        && COMPOSITOR == REGISTRY_SYNC + 1
        && XDG_SHELL == COMPOSITOR + 1
        && SURFACE == XDG_SHELL + 1
        && XDG_SURFACE == SURFACE + 1
        && XDG_TOPLEVEL == XDG_SURFACE + 1
        && ACK_SYNC == XDG_TOPLEVEL + 1
);

fn send(socket: &mut UnixStream, object: u32, opcode: u16, payload: &[u8])
    -> Result<(), &'static str>
{
    let len = payload.len() + 8;
    if len > u16::MAX as usize { return Err("invalid_request"); }
    socket.write_all(&object.to_ne_bytes()).map_err(|_| "write_failed")?;
    socket.write_all(&(((len as u32) << 16) | u32::from(opcode)).to_ne_bytes())
        .map_err(|_| "write_failed")?;
    socket.write_all(payload).map_err(|_| "write_failed")
}

fn event(socket: &mut UnixStream) -> Result<(u32, u16, Vec<u8>), &'static str> {
    let mut header = [0u8; 8];
    socket.read_exact(&mut header).map_err(|_| "read_failed")?;
    let sender = u32::from_ne_bytes(header[0..4].try_into().unwrap());
    let word = u32::from_ne_bytes(header[4..8].try_into().unwrap());
    let len = (word >> 16) as usize;
    if !(8..=65535).contains(&len) { return Err("invalid_event"); }
    let mut payload = vec![0u8; len - 8];
    socket.read_exact(&mut payload).map_err(|_| "read_failed")?;
    Ok((sender, (word & 0xffff) as u16, payload))
}

// wl_display.error contains only the failed object ID, protocol code and a
// message. Do not disclose the freeform message or guest raw Wayland trace.
fn check_protocol_error(sender: u32, opcode: u16, payload: &[u8]) -> Result<(), &'static str> {
    if sender != 1 || opcode != 0 { return Ok(()); }
    let (interface, code_label) = if payload.len() < 8 {
        ("malformed", "other".to_owned())
    } else {
        let id = u32::from_ne_bytes(payload[..4].try_into().unwrap());
        let code = u32::from_ne_bytes(payload[4..8].try_into().unwrap());
        let interface = match id {
            1 => "display",
            REGISTRY => "registry",
            REGISTRY_SYNC | ACK_SYNC => "sync",
            COMPOSITOR => "compositor",
            SURFACE => "surface",
            XDG_SHELL => "xdg_wm_base",
            XDG_SURFACE => "xdg_surface",
            XDG_TOPLEVEL => "xdg_toplevel",
            _ => "other",
        };
        (interface, if code <= 16 { code.to_string() } else { "other".to_owned() })
    };
    println!("STARBYTE_GUEST_XDG_ERROR_DIAG interface={interface} code={code_label}");
    Err("protocol_error")
}

fn bind(socket: &mut UnixStream, name: u32, iface: &str, version: u32, new_id: u32)
    -> Result<(), &'static str>
{
    let mut payload = Vec::new();
    payload.extend_from_slice(&name.to_ne_bytes());
    payload.extend_from_slice(&((iface.len() + 1) as u32).to_ne_bytes());
    payload.extend_from_slice(iface.as_bytes());
    payload.push(0);
    while payload.len() % 4 != 0 { payload.push(0); }
    payload.extend_from_slice(&version.min(1).to_ne_bytes());
    payload.extend_from_slice(&new_id.to_ne_bytes());
    send(socket, REGISTRY, 0, &payload)
}

fn run() -> Result<(), &'static str> {
    if env::var("STARBYTE_GUEST_XDG_PROBE").as_deref() != Ok("1")
        || env::var("XDG_RUNTIME_DIR").as_deref() != Ok("/run/user/0")
    {
        return Err("private_guest_only");
    }
    let display = env::var("WAYLAND_DISPLAY").map_err(|_| "missing_display")?;
    if display.is_empty() || display.contains('/') || display.contains("..") {
        return Err("unsafe_display_name");
    }
    let path = PathBuf::from("/run/user/0").join(display);
    let mut socket = UnixStream::connect(path).map_err(|_| "connect_failed")?;
    socket.set_read_timeout(Some(Duration::from_secs(12))).map_err(|_| "timeout_failed")?;
    socket.set_write_timeout(Some(Duration::from_secs(12))).map_err(|_| "timeout_failed")?;
    send(&mut socket, 1, 1, &REGISTRY.to_ne_bytes())?;
    send(&mut socket, 1, 0, &REGISTRY_SYNC.to_ne_bytes())?;

    let mut compositor = None;
    let mut shell = None;
    let mut synced = false;
    for _ in 0..256 {
        let (sender, opcode, payload) = event(&mut socket)?;
        check_protocol_error(sender, opcode, &payload)?;
        if sender == REGISTRY && opcode == 0 && payload.len() >= 12 {
            let name = u32::from_ne_bytes(payload[0..4].try_into().unwrap());
            let n = u32::from_ne_bytes(payload[4..8].try_into().unwrap()) as usize;
            let end = 8usize.checked_add(n).ok_or("invalid_global")?;
            let align = end.checked_add(3).ok_or("invalid_global")? & !3;
            if n == 0 || align + 4 > payload.len() { return Err("invalid_global"); }
            let version = u32::from_ne_bytes(payload[align..align+4].try_into().unwrap());
            let interface = &payload[8..end];
            if interface == b"wl_compositor\0" {
                compositor = Some((name, version));
            } else if interface == b"xdg_wm_base\0" {
                shell = Some((name, version));
            }
        }
        if sender == REGISTRY_SYNC && opcode == 0 { synced = true; break; }
    }
    if !synced { return Err("registry_timeout"); }
    let (compositor_name, compositor_version) = compositor.ok_or("compositor_missing")?;
    let (shell_name, shell_version) = shell.ok_or("xdg_shell_missing")?;
    bind(&mut socket, compositor_name, "wl_compositor", compositor_version, COMPOSITOR)?;
    bind(&mut socket, shell_name, "xdg_wm_base", shell_version, XDG_SHELL)?;

    send(&mut socket, COMPOSITOR, 0, &SURFACE.to_ne_bytes())?;
    let mut xdg_surface = Vec::new();
    xdg_surface.extend_from_slice(&XDG_SURFACE.to_ne_bytes());
    xdg_surface.extend_from_slice(&SURFACE.to_ne_bytes());
    send(&mut socket, XDG_SHELL, 2, &xdg_surface)?;
    send(&mut socket, XDG_SURFACE, 1, &XDG_TOPLEVEL.to_ne_bytes())?;
    // Initial empty commit is the XDG shell requirement for first configure.
    send(&mut socket, SURFACE, 6, &[])?;

    let mut configured = false;
    for _ in 0..256 {
        let (sender, opcode, payload) = event(&mut socket)?;
        check_protocol_error(sender, opcode, &payload)?;
        if sender == XDG_SHELL && opcode == 0 && payload.len() >= 4 {
            send(&mut socket, XDG_SHELL, 3, &payload[..4])?;
        }
        if sender == XDG_SURFACE && opcode == 0 && payload.len() >= 4 {
            send(&mut socket, XDG_SURFACE, 4, &payload[..4])?;
            send(&mut socket, SURFACE, 6, &[])?;
            // Do not claim the compositor accepted our ack based only on
            // socket writes. The sync callback proves the requests were
            // dispatched; any intervening wl_display.error fails the probe.
            send(&mut socket, 1, 0, &ACK_SYNC.to_ne_bytes())?;
            configured = true;
            break;
        }
    }
    if !configured { return Err("configure_timeout"); }
    for _ in 0..256 {
        let (sender, opcode, payload) = event(&mut socket)?;
        check_protocol_error(sender, opcode, &payload)?;
        if sender == XDG_SHELL && opcode == 0 && payload.len() >= 4 {
            send(&mut socket, XDG_SHELL, 3, &payload[..4])?;
        }
        if sender == ACK_SYNC && opcode == 0 && payload.len() >= 4 {
            return Ok(());
        }
    }
    Err("ack_timeout")
}

fn main() {
    match run() {
        Ok(()) => println!("STARBYTE_GUEST_XDG_SURFACE_RESULT connected=1 xdg=1 committed=1 configured=1 acked=1"),
        Err(reason) => {
            println!("STARBYTE_GUEST_XDG_SURFACE_BLOCKED {reason}");
            std::process::exit(2);
        }
    }
}
