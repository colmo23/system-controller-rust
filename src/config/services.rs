use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;

#[derive(Debug, Clone)]
pub struct ServiceConfig {
    pub name_pattern: String,
    pub files: Vec<String>,
    pub commands: Vec<String>,
    pub is_glob: bool,
}

#[derive(Deserialize)]
struct ServicesFile {
    services: HashMap<String, ServiceEntry>,
}

#[derive(Deserialize)]
struct ServiceEntry {
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    commands: Vec<String>,
}

pub fn parse_services(path: &str) -> Result<Vec<ServiceConfig>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read services file: {}", path))?;

    let file: ServicesFile = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse services YAML: {}", path))?;

    let mut configs: Vec<ServiceConfig> = file
        .services
        .into_iter()
        .map(|(name, entry)| {
            let is_glob = name.contains('*') || name.contains('?') || name.contains('[');
            ServiceConfig {
                name_pattern: name,
                files: entry.files,
                commands: entry.commands,
                is_glob,
            }
        })
        .collect();

    configs.sort_by(|a, b| a.name_pattern.cmp(&b.name_pattern));

    for cfg in &configs {
        log::debug!(
            "Service config: {} (glob={}, files={}, commands={})",
            cfg.name_pattern,
            cfg.is_glob,
            cfg.files.len(),
            cfg.commands.len()
        );
    }

    Ok(configs)
}
