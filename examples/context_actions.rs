use anyhow;
use hyprland::data::Clients;
use hyprland::shared::{HyprData, HyprDataActive, HyprDataActiveOptional};

fn main() -> anyhow::Result<()> {
    let clients = Clients::get()?;

    for client in clients {
        println!("{} (PID: {})", client.title, client.pid)
    }
    Ok(())
}
