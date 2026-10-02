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

pub fn start_listener(on_toggle: impl Fn() + Send + 'static) {
    let _ = std::fs::remove_file(SOCKET_PATH);
    let Ok(listener) = UnixListener::bind(SOCKET_PATH) else {
        eprintln!("failed to bind toggle socket {SOCKET_PATH}");
        return;
    };

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