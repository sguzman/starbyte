//! Read synthetic KEY_A from Linux evdev in disposable guest, never host input.
use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::thread;
use std::time::{Duration, Instant};

const O_NONBLOCK: i32 = 0x800;
const EVENT_SIZE: usize = 24; // x86-64 input_event: 16 timeval + 8 type/code/value.

fn main() {
    let Ok(nodes) = std::fs::read_dir("/dev/input") else {
        println!("STARBYTE_GUEST_INPUT_RESULT key_a=0");
        std::process::exit(2);
    };
    let mut files = Vec::new();
    for node in nodes.flatten() {
        if !node.file_name().to_string_lossy().starts_with("event") {
            continue;
        }
        if let Ok(file) = OpenOptions::new()
            .read(true)
            .custom_flags(O_NONBLOCK)
            .open(node.path())
        {
            files.push(file);
        }
    }
    if files.is_empty() {
        println!("STARBYTE_GUEST_INPUT_RESULT key_a=0");
        std::process::exit(2);
    }
    // The host waits for this guest-owned receipt before using private QMP.
    println!("STARBYTE_GUEST_INPUT_READY evdev=present");
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        for file in &mut files {
            let mut buf = [0_u8; EVENT_SIZE * 32];
            let Ok(n) = file.read(&mut buf) else {
                continue;
            };
            for event in buf[..n].chunks_exact(EVENT_SIZE) {
                let kind = u16::from_ne_bytes([event[16], event[17]]);
                let code = u16::from_ne_bytes([event[18], event[19]]);
                let value = i32::from_ne_bytes([event[20], event[21], event[22], event[23]]);
                if kind == 1 && code == 30 && value == 1 {
                    println!("STARBYTE_GUEST_INPUT_RESULT key_a=1");
                    return;
                }
            }
        }
        thread::sleep(Duration::from_millis(25));
    }
    println!("STARBYTE_GUEST_INPUT_RESULT key_a=0");
    std::process::exit(2);
}
