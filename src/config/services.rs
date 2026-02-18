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

#[cfg(test)]
mod tests {
    use super::*;

    struct TempYaml(std::path::PathBuf);

    impl TempYaml {
        fn new(content: &str) -> Self {
            use std::time::{SystemTime, UNIX_EPOCH};
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .subsec_nanos();
            let path = std::env::temp_dir()
                .join(format!("sc_svc_{}_{}.yaml", std::process::id(), nanos));
            std::fs::write(&path, content).unwrap();
            TempYaml(path)
        }

        fn path(&self) -> &str {
            self.0.to_str().unwrap()
        }
    }

    impl Drop for TempYaml {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn test_parse_services_single_entry() {
        let f = TempYaml::new("services:\n  nginx:\n    commands:\n      - nginx -T\n");
        let configs = parse_services(f.path()).unwrap();
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].name_pattern, "nginx");
        assert_eq!(configs[0].commands, vec!["nginx -T"]);
        assert!(!configs[0].is_glob);
    }

    #[test]
    fn test_parse_services_glob_star() {
        let f = TempYaml::new("services:\n  nginx*:\n    commands:\n      - nginx -T\n");
        let configs = parse_services(f.path()).unwrap();
        assert!(configs[0].is_glob);
    }

    #[test]
    fn test_parse_services_glob_question_mark() {
        let f = TempYaml::new("services:\n  nginx?:\n    commands: []\n");
        let configs = parse_services(f.path()).unwrap();
        assert!(configs[0].is_glob);
    }

    #[test]
    fn test_parse_services_glob_bracket() {
        let f = TempYaml::new("services:\n  \"nginx[0-9]\":\n    commands: []\n");
        let configs = parse_services(f.path()).unwrap();
        assert!(configs[0].is_glob);
    }

    #[test]
    fn test_parse_services_non_glob() {
        let f = TempYaml::new("services:\n  sshd:\n    commands: []\n");
        let configs = parse_services(f.path()).unwrap();
        assert!(!configs[0].is_glob);
    }

    #[test]
    fn test_parse_services_files_and_commands() {
        let f = TempYaml::new(
            "services:\n  nginx:\n    files:\n      - /etc/nginx/nginx.conf\n    commands:\n      - nginx -T\n",
        );
        let configs = parse_services(f.path()).unwrap();
        assert_eq!(configs[0].files, vec!["/etc/nginx/nginx.conf"]);
        assert_eq!(configs[0].commands, vec!["nginx -T"]);
    }

    #[test]
    fn test_parse_services_sorted_by_name() {
        let f = TempYaml::new(
            "services:\n  zebra:\n    commands: []\n  alpha:\n    commands: []\n  middle:\n    commands: []\n",
        );
        let configs = parse_services(f.path()).unwrap();
        let names: Vec<&str> = configs.iter().map(|c| c.name_pattern.as_str()).collect();
        assert_eq!(names, vec!["alpha", "middle", "zebra"]);
    }

    #[test]
    fn test_parse_services_empty_defaults() {
        let f = TempYaml::new("services:\n  bare:\n");
        let configs = parse_services(f.path()).unwrap();
        assert_eq!(configs[0].files, Vec::<String>::new());
        assert_eq!(configs[0].commands, Vec::<String>::new());
    }

    #[test]
    fn test_parse_services_missing_file_fails() {
        assert!(parse_services("/tmp/nonexistent_sc_test_xyz.yaml").is_err());
    }

    #[test]
    fn test_parse_services_invalid_yaml_fails() {
        let f = TempYaml::new("this is not: valid: yaml: at: all\n  broken\n");
        assert!(parse_services(f.path()).is_err());
    }

    /// Validates the actual services.yaml used by run-test.sh.
    #[test]
    fn test_parse_services_yaml() {
        let configs = parse_services("services.yaml")
            .expect("services.yaml should parse successfully");

        // 5 entries: nginx*, postgresql*, redis*, s*, c*
        assert_eq!(configs.len(), 5, "expected 5 service configs");

        // Configs are sorted by name_pattern
        let names: Vec<&str> = configs.iter().map(|c| c.name_pattern.as_str()).collect();
        assert_eq!(names, vec!["c*", "nginx*", "postgresql*", "redis*", "s*"]);

        // All patterns contain * → all are globs
        for cfg in &configs {
            assert!(cfg.is_glob, "{} should be a glob", cfg.name_pattern);
        }

        let nginx = configs.iter().find(|c| c.name_pattern == "nginx*").unwrap();
        assert_eq!(nginx.files.len(), 2);
        assert_eq!(nginx.commands.len(), 1);

        let redis = configs.iter().find(|c| c.name_pattern == "redis*").unwrap();
        assert_eq!(redis.files.len(), 0);
        assert_eq!(redis.commands.len(), 1);
    }
}
