use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::thread;

/// Per-user socket: shared /tmp would let users toggle (or block) each other.
fn socket_path() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir).join("luma-rust.sock"),
        None => PathBuf::from(format!("/tmp/luma-rust-{}.sock", unsafe { libc::getuid() })),
    }
}

/// Sends a toggle command to the running instance. Returns true if one existed.
pub fn notify_running() -> bool {
    match UnixStream::connect(socket_path()) {
        Ok(mut stream) => stream.write_all(b"toggle").is_ok(),
        Err(_) => false,
    }
}

/// Binds the toggle socket. Called before the heavy init so keybinds during
/// startup reach us instead of spawning duplicate instances. If the socket
/// file exists, a successful connect means a live instance owns it (do not
/// steal it), while a failed connect means it is stale and safe to rebind.
/// None = a live instance is already running.
pub fn bind_socket() -> Option<UnixListener> {
    let path = socket_path();
    match UnixListener::bind(&path) {
        Ok(listener) => Some(listener),
        Err(_) => {
            if UnixStream::connect(&path).is_ok() {
                return None;
            }
            let _ = std::fs::remove_file(&path);
            Some(UnixListener::bind(&path).expect("failed to bind toggle socket"))
        }
    }
}

pub fn start_listener(listener: UnixListener, on_toggle: impl Fn() + Send + 'static) {
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0; 16];
            if stream.read(&mut buf).unwrap_or(0) > 0 {
                on_toggle();
            }
        }
    });
}
