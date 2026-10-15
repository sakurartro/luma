use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::thread;

const SOCKET_PATH: &str = "/tmp/luma-rust.sock";

/// Sends a toggle command to the running instance. Returns true if one existed.
pub fn notify_running() -> bool {
    match UnixStream::connect(SOCKET_PATH) {
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
    match UnixListener::bind(SOCKET_PATH) {
        Ok(listener) => Some(listener),
        Err(_) => {
            if UnixStream::connect(SOCKET_PATH).is_ok() {
                return None;
            }
            let _ = std::fs::remove_file(SOCKET_PATH);
            Some(UnixListener::bind(SOCKET_PATH).expect("failed to bind toggle socket"))
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
