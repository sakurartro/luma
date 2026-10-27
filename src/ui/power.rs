use crate::MainWindow;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

pub(super) fn wire(main_window: &MainWindow) {
    main_window.on_power_action(move |action| {
        let cmd = match action.as_str() {
            "poweroff" => "systemctl poweroff",
            "reboot" => "systemctl reboot",
            "suspend" => "systemctl suspend",
            "lock" => "loginctl lock-session",
            other => {
                eprintln!("unknown power action: {other}");
                return;
            }
        };
        if let Err(err) = Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
        {
            eprintln!("failed to run {cmd}: {err}");
        }
    });
}
