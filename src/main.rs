mod app;
mod config;
mod monitor;
mod ssh;
mod tui;

use anyhow::{Context, Result};
use std::env;
use std::panic;

#[tokio::main]
async fn main() -> Result<()> {
    // Set up panic hook to restore terminal
    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        let _ = crate::tui::restore();
        original_hook(panic_info);
    }));

    let args: Vec<String> = env::args().collect();

    if args.len() != 3 {
        eprintln!("Usage: {} <inventory.ini> <services.yaml>", args[0]);
        std::process::exit(1);
    }

    let inventory_path = &args[1];
    let services_path = &args[2];

    let hosts = config::inventory::parse_inventory(inventory_path)
        .context("Failed to parse inventory")?;
    let service_configs = config::services::parse_services(services_path)
        .context("Failed to parse services config")?;

    app::run(hosts, service_configs).await?;

    Ok(())
}
