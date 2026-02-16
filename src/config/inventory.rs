use anyhow::{Context, Result};
use std::fs;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Host {
    pub address: String,
    pub group: String,
}

fn is_ip_address(s: &str) -> bool {
    // Match IPv4: digits and dots, at least one dot
    if s.contains('.') {
        return s.chars().all(|c| c.is_ascii_digit() || c == '.');
    }
    false
}

fn extract_address(line: &str) -> Option<String> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }

    // First check if any token is a key=value with ansible_host
    for token in &tokens {
        if let Some(value) = token.strip_prefix("ansible_host=") {
            return Some(value.to_string());
        }
    }

    // Otherwise take the first token that looks like an IP address
    for token in &tokens {
        // Skip key=value pairs
        if token.contains('=') {
            continue;
        }
        if is_ip_address(token) {
            return Some(token.to_string());
        }
    }

    // Fall back to the first token (could be a hostname)
    let first = tokens[0];
    if !first.contains('=') {
        Some(first.to_string())
    } else {
        None
    }
}

pub fn parse_inventory(path: &str) -> Result<Vec<Host>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read inventory file: {}", path))?;

    let mut hosts = Vec::new();
    let mut current_group = String::from("ungrouped");

    for line in content.lines() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        // Section header
        if line.starts_with('[') && line.ends_with(']') {
            current_group = line[1..line.len() - 1].to_string();
            // Skip meta-groups like [group:children] or [group:vars]
            if current_group.contains(':') {
                current_group = String::from("_skip");
            }
            continue;
        }

        if current_group == "_skip" {
            continue;
        }

        if let Some(address) = extract_address(line) {
            hosts.push(Host {
                address,
                group: current_group.clone(),
            });
        }
    }

    anyhow::ensure!(!hosts.is_empty(), "No hosts found in inventory file: {}", path);

    for host in &hosts {
        log::debug!("Inventory host: {} (group: {})", host.address, host.group);
    }

    Ok(hosts)
}
