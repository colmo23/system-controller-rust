mod app;
mod config;
mod logging;
mod monitor;
mod ssh;
mod tui;

use anyhow::{Context, Result};
use std::env;
use std::panic;

fn print_usage(program: &str) {
    eprintln!("Usage: {} [--log <logfile>] [--user <username>] <inventory.ini> <services.yaml>", program);
}

#[tokio::main]
async fn main() -> Result<()> {
    // Set up panic hook to restore terminal
    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        let _ = crate::tui::restore();
        original_hook(panic_info);
    }));

    let args: Vec<String> = env::args().collect();

    // Parse optional --log <file>, --user <username>, and positional args
    let mut log_file: Option<String> = None;
    let mut ssh_user: Option<String> = None;
    let mut positional = Vec::new();
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--log" {
            if i + 1 >= args.len() {
                print_usage(&args[0]);
                std::process::exit(1);
            }
            log_file = Some(args[i + 1].clone());
            i += 2;
        } else if args[i] == "--user" {
            if i + 1 >= args.len() {
                print_usage(&args[0]);
                std::process::exit(1);
            }
            ssh_user = Some(args[i + 1].clone());
            i += 2;
        } else {
            positional.push(args[i].clone());
            i += 1;
        }
    }

    if positional.len() != 2 {
        print_usage(&args[0]);
        std::process::exit(1);
    }

    if let Some(ref path) = log_file {
        logging::init(path).context("Failed to initialize logging")?;
        log::info!("system-controller starting");
    }

    let inventory_path = &positional[0];
    let services_path = &positional[1];

    log::info!("Parsing inventory: {}", inventory_path);
    let hosts = config::inventory::parse_inventory(inventory_path)
        .context("Failed to parse inventory")?;
    log::info!("Loaded {} hosts", hosts.len());

    log::info!("Parsing services config: {}", services_path);
    let service_configs = config::services::parse_services(services_path)
        .context("Failed to parse services config")?;
    log::info!("Loaded {} service configs", service_configs.len());

    app::run(hosts, service_configs, ssh_user).await?;

    log::info!("system-controller exiting");
    Ok(())
}
