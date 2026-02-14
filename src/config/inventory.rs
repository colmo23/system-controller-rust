use anyhow::{Context, Result};
use configparser::ini::Ini;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Host {
    pub address: String,
    pub group: String,
}

pub fn parse_inventory(path: &str) -> Result<Vec<Host>> {
    let mut config = Ini::new_cs();
    config
        .load(path)
        .map_err(|e| anyhow::anyhow!(e))
        .with_context(|| format!("Failed to load inventory file: {}", path))?;

    let mut hosts = Vec::new();

    for section in config.sections() {
        if let Some(map) = config.get_map_ref().get(&section) {
            for (key, _) in map {
                // In Ansible-style inventory, bare hostnames are keys with no value
                hosts.push(Host {
                    address: key.clone(),
                    group: section.clone(),
                });
            }
        }
    }

    anyhow::ensure!(!hosts.is_empty(), "No hosts found in inventory file: {}", path);

    for host in &hosts {
        log::debug!("Inventory host: {} (group: {})", host.address, host.group);
    }

    Ok(hosts)
}
