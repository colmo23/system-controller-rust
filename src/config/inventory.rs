use anyhow::{Context, Result};
use std::fs;

#[cfg(test)]
struct TempIni(std::path::PathBuf);

#[cfg(test)]
impl TempIni {
    fn new(content: &str) -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let path = std::env::temp_dir()
            .join(format!("sc_inv_{}_{}.ini", std::process::id(), nanos));
        std::fs::write(&path, content).unwrap();
        TempIni(path)
    }

    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

#[cfg(test)]
impl Drop for TempIni {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    // --- is_ip_address ---

    #[test]
    fn test_is_ip_valid() {
        assert!(is_ip_address("192.168.1.1"));
        assert!(is_ip_address("127.0.0.1"));
        assert!(is_ip_address("10.10.44.55"));
    }

    #[test]
    fn test_is_ip_hostname() {
        assert!(!is_ip_address("localhost"));
        assert!(!is_ip_address("google.ie"));
        assert!(!is_ip_address("myhost"));
    }

    // --- extract_address ---

    #[test]
    fn test_extract_address_ansible_host() {
        let addr = extract_address("myhost ansible_host=10.0.0.5");
        assert_eq!(addr, Some("10.0.0.5".to_string()));
    }

    #[test]
    fn test_extract_address_ip_among_tokens() {
        // "abc 127.0.0.1 abc" — should pick the IP
        let addr = extract_address("abc 127.0.0.1 abc");
        assert_eq!(addr, Some("127.0.0.1".to_string()));
    }

    #[test]
    fn test_extract_address_plain_ip() {
        let addr = extract_address("192.168.1.10");
        assert_eq!(addr, Some("192.168.1.10".to_string()));
    }

    #[test]
    fn test_extract_address_hostname_fallback() {
        let addr = extract_address("google.ie");
        assert_eq!(addr, Some("google.ie".to_string()));
    }

    #[test]
    fn test_extract_address_empty() {
        let addr = extract_address("");
        assert_eq!(addr, None);
    }

    #[test]
    fn test_extract_address_key_value_only() {
        // A line that is only key=value pairs with no usable address
        let addr = extract_address("key=value foo=bar");
        assert_eq!(addr, None);
    }

    // --- parse_inventory ---

    #[test]
    fn test_parse_inventory_simple_group() {
        let f = TempIni::new("[servers]\n192.168.1.1\n192.168.1.2\n");
        let hosts = parse_inventory(f.path()).unwrap();
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0].address, "192.168.1.1");
        assert_eq!(hosts[0].group, "servers");
        assert_eq!(hosts[1].address, "192.168.1.2");
        assert_eq!(hosts[1].group, "servers");
    }

    #[test]
    fn test_parse_inventory_multiple_groups() {
        let f = TempIni::new("[web]\n10.0.0.1\n[db]\n10.0.0.2\n");
        let hosts = parse_inventory(f.path()).unwrap();
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0].group, "web");
        assert_eq!(hosts[1].group, "db");
    }

    #[test]
    fn test_parse_inventory_skips_meta_groups() {
        let f = TempIni::new(
            "[web]\n10.0.0.1\n[all:children]\nweb\n[web:vars]\nfoo=bar\n",
        );
        let hosts = parse_inventory(f.path()).unwrap();
        // Only the host from [web] should be present
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].address, "10.0.0.1");
    }

    #[test]
    fn test_parse_inventory_ignores_comments_and_blanks() {
        let f = TempIni::new(
            "# top comment\n\n[group]\n; inline comment\n\n10.0.0.1\n\n# another\n10.0.0.2\n",
        );
        let hosts = parse_inventory(f.path()).unwrap();
        assert_eq!(hosts.len(), 2);
    }

    #[test]
    fn test_parse_inventory_ansible_host_key() {
        let f = TempIni::new("[prod]\nmyalias ansible_host=172.16.0.1\n");
        let hosts = parse_inventory(f.path()).unwrap();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].address, "172.16.0.1");
    }

    #[test]
    fn test_parse_inventory_hostname_entry() {
        let f = TempIni::new("[hosts]\ngoogle.ie\n");
        let hosts = parse_inventory(f.path()).unwrap();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].address, "google.ie");
    }

    #[test]
    fn test_parse_inventory_empty_file_fails() {
        let f = TempIni::new("# just a comment\n");
        assert!(parse_inventory(f.path()).is_err());
    }

    #[test]
    fn test_parse_inventory_missing_file_fails() {
        assert!(parse_inventory("/tmp/nonexistent_sc_test_xyz.ini").is_err());
    }

    /// Validates the actual inventory-small.ini used by run-test.sh.
    #[test]
    fn test_parse_inventory_small_ini() {
        let hosts = parse_inventory("inventory-small.ini")
            .expect("inventory-small.ini should parse successfully");

        assert_eq!(hosts.len(), 4, "expected 4 hosts");

        // Row 1: "abc 127.0.0.1 abc" → IP wins
        assert_eq!(hosts[0].address, "127.0.0.1");
        assert_eq!(hosts[0].group, "localhost");

        // Row 2: plain IP
        assert_eq!(hosts[1].address, "126.0.0.2");
        assert_eq!(hosts[1].group, "localhost");

        // Row 3: hostname fallback
        assert_eq!(hosts[2].address, "google.ie");
        assert_eq!(hosts[2].group, "localhost");

        // Row 4: different group
        assert_eq!(hosts[3].address, "10.10.44.55");
        assert_eq!(hosts[3].group, "other hosts");
    }
}
