use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::thread;

use crate::api;

/// Per-user socket: shared /tmp would let users toggle (or block) each other.
pub fn socket_path() -> PathBuf {
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
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(_) => {
            if UnixStream::connect(&path).is_ok() {
                return None;
            }
            let _ = std::fs::remove_file(&path);
            UnixListener::bind(&path).expect("failed to bind toggle socket")
        }
    };
    // Search answers carry file paths; the /tmp fallback is reachable by other users.
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    Some(listener)
}

pub fn start_listener(listener: UnixListener, on_toggle: impl Fn() + Send + 'static) {
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            // Clients close their write side, so EOF ends the request.
            let mut buf = Vec::new();
            let _ = stream.read_to_end(&mut buf);
            let request = String::from_utf8_lossy(&buf);
            if let Some((kind, query)) = api::parse_request(&request) {
                let query = query.to_string();
                thread::spawn(move || api::handle_search(stream, kind, &query));
            } else if !buf.is_empty() {
                on_toggle();
            }
        }
    });
}
