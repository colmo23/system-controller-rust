use crate::config::{Host, ServiceConfig};
use crate::ssh::SessionManager;
use glob_match::glob_match;

#[derive(Debug, Clone, PartialEq)]
pub enum ServiceStatus {
    Unknown,
    Active,
    Inactive,
    Failed,
    NotFound,
    Error(String),
}

impl ServiceStatus {
    pub fn from_str(s: &str) -> Self {
        match s.trim() {
            "active" => ServiceStatus::Active,
            "inactive" => ServiceStatus::Inactive,
            "failed" => ServiceStatus::Failed,
            "not-found" | "not found" => ServiceStatus::NotFound,
            "" => ServiceStatus::Unknown,
            other => {
                if other.contains("could not be found") || other.contains("not-found") {
                    ServiceStatus::NotFound
                } else {
                    ServiceStatus::Error(other.to_string())
                }
            }
        }
    }

    pub fn display(&self) -> &str {
        match self {
            ServiceStatus::Unknown => "???",
            ServiceStatus::Active => "active",
            ServiceStatus::Inactive => "inactive",
            ServiceStatus::Failed => "FAILED",
            ServiceStatus::NotFound => "not found",
            ServiceStatus::Error(e) => e.as_str(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HostService {
    pub host_address: String,
    pub service_name: String,
    pub config: ServiceConfig,
    pub status: ServiceStatus,
}

/// Expand glob patterns by querying systemctl list-units on the host.
/// Returns the list of concrete service names for this host.
pub async fn expand_globs(
    session_mgr: &mut SessionManager,
    host: &Host,
    service_configs: &[ServiceConfig],
) -> Vec<(String, ServiceConfig)> {
    let mut results = Vec::new();

    // Check if any configs are globs
    let has_globs = service_configs.iter().any(|c| c.is_glob);

    // If we have globs, fetch the unit list once
    let unit_list: Vec<String> = if has_globs {
        log::debug!("Fetching unit list from {} for glob expansion", host.address);
        match session_mgr
            .run_command(
                &host.address,
                "systemctl list-units --type=service --all --no-legend --no-pager",
            )
            .await
        {
            Ok(output) => {
                let units: Vec<String> = output
                    .lines()
                    .filter_map(|line| {
                        let unit = line.split_whitespace().next()?;
                        Some(unit.strip_suffix(".service").unwrap_or(unit).to_string())
                    })
                    .collect();
                log::debug!("Found {} units on {}", units.len(), host.address);
                units
            }
            Err(e) => {
                log::error!("Failed to list units on {}: {}", host.address, e);
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    for config in service_configs {
        if config.is_glob {
            let mut matched: Vec<String> = unit_list
                .iter()
                .filter(|unit| glob_match(&config.name_pattern, unit))
                .cloned()
                .collect();
            matched.sort();
            log::info!(
                "Glob '{}' matched {} services on {}: {:?}",
                config.name_pattern,
                matched.len(),
                host.address,
                matched
            );
            for name in matched {
                results.push((name, config.clone()));
            }
        } else {
            results.push((config.name_pattern.clone(), config.clone()));
        }
    }

    results
}

/// Fetch statuses for a list of services on a host in a single SSH call.
pub async fn fetch_statuses(
    session_mgr: &mut SessionManager,
    host: &str,
    service_names: &[String],
) -> Vec<ServiceStatus> {
    if service_names.is_empty() {
        return Vec::new();
    }

    let svc_args: Vec<String> = service_names
        .iter()
        .map(|s| format!("{}.service", s))
        .collect();
    let cmd = format!("systemctl is-active {}", svc_args.join(" "));

    log::debug!("Fetching status for {} services on {}", service_names.len(), host);

    match session_mgr.run_command(host, &cmd).await {
        Ok(output) => {
            let statuses: Vec<ServiceStatus> = output
                .lines()
                .map(|line| ServiceStatus::from_str(line))
                .collect();
            // Pad with Unknown if fewer lines than expected
            let mut result = statuses;
            while result.len() < service_names.len() {
                result.push(ServiceStatus::Unknown);
            }
            for (i, name) in service_names.iter().enumerate() {
                log::debug!("  {}:{} = {:?}", host, name, result[i]);
            }
            result
        }
        Err(e) => {
            log::error!("Failed to fetch statuses on {}: {}", host, e);
            vec![ServiceStatus::Error(e.to_string()); service_names.len()]
        }
    }
}

/// Build the initial grid: expand globs, then fetch all statuses.
/// Returns (column_names, grid) where grid[host_idx][svc_idx] = HostService.
pub async fn build_grid(
    session_mgr: &mut SessionManager,
    hosts: &[Host],
    service_configs: &[ServiceConfig],
) -> (Vec<String>, Vec<Vec<HostService>>) {
    log::info!("Building grid for {} hosts, {} service configs", hosts.len(), service_configs.len());

    // First pass: expand globs on all hosts, collect the union of service names
    let mut all_expanded: Vec<Vec<(String, ServiceConfig)>> = Vec::new();
    let mut all_service_names: Vec<String> = Vec::new();

    for host in hosts {
        let expanded = expand_globs(session_mgr, host, service_configs).await;
        for (name, _) in &expanded {
            if !all_service_names.contains(name) {
                all_service_names.push(name.clone());
            }
        }
        all_expanded.push(expanded);
    }

    log::info!("Service columns after glob expansion: {:?}", all_service_names);

    // Build grid
    let mut grid: Vec<Vec<HostService>> = Vec::new();

    for (host_idx, host) in hosts.iter().enumerate() {
        let expanded = &all_expanded[host_idx];
        let expanded_map: std::collections::HashMap<&str, &ServiceConfig> =
            expanded.iter().map(|(n, c)| (n.as_str(), c)).collect();

        // Fetch statuses for services this host has
        let host_svc_names: Vec<String> = all_service_names
            .iter()
            .filter(|n| expanded_map.contains_key(n.as_str()))
            .cloned()
            .collect();

        let statuses = fetch_statuses(session_mgr, &host.address, &host_svc_names).await;

        let mut status_map: std::collections::HashMap<String, ServiceStatus> =
            std::collections::HashMap::new();
        for (i, name) in host_svc_names.iter().enumerate() {
            status_map.insert(name.clone(), statuses[i].clone());
        }

        let mut row = Vec::new();
        for svc_name in &all_service_names {
            let (config, status) = if let Some(cfg) = expanded_map.get(svc_name.as_str()) {
                let st = status_map
                    .remove(svc_name)
                    .unwrap_or(ServiceStatus::Unknown);
                ((*cfg).clone(), st)
            } else {
                // This host doesn't have this service (e.g., glob didn't match)
                let cfg = service_configs
                    .iter()
                    .find(|c| c.name_pattern == *svc_name || (c.is_glob && glob_match(&c.name_pattern, svc_name)))
                    .cloned()
                    .unwrap_or(ServiceConfig {
                        name_pattern: svc_name.clone(),
                        files: Vec::new(),
                        commands: Vec::new(),
                        is_glob: false,
                    });
                (cfg, ServiceStatus::NotFound)
            };

            let mut config = config;
            config.commands.push(format!("sudo journalctl -u {}", svc_name));

            row.push(HostService {
                host_address: host.address.clone(),
                service_name: svc_name.clone(),
                config,
                status,
            });
        }
        grid.push(row);
    }

    log::info!("Grid built: {} rows x {} columns", grid.len(), all_service_names.len());
    (all_service_names, grid)
}

/// Refresh status for a single cell.
pub async fn refresh_cell(
    session_mgr: &mut SessionManager,
    host: &str,
    service_name: &str,
) -> ServiceStatus {
    log::debug!("Refreshing status for {}:{}", host, service_name);
    let statuses = fetch_statuses(session_mgr, host, &[service_name.to_string()]).await;
    statuses.into_iter().next().unwrap_or(ServiceStatus::Unknown)
}
